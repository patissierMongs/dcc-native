//! One-shot commands: install / uninstall / stop / help.
//!
//! The wallpaper has no console, so these answer with a message box. Install
//! registers a Task Scheduler entry that starts DCC at sign-in with the
//! highest available rights — per-process network bytes come from a kernel
//! ETW session, which needs them — and with no execution time limit (the
//! Task Scheduler default of 72 hours would otherwise stop a long-running
//! wallpaper on day three).

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const TASK_NAME: &str = "DCC Wallpaper";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Posted by `--stop` to every running wallpaper. The wallpaper whitelists it
/// with ChangeWindowMessageFilterEx, because an elevated wallpaper would
/// otherwise drop messages from a non-elevated sender.
pub const WM_DCC_QUIT: u32 = WM_APP + 0x0DC;

/// Wallpaper flags worth carrying into the scheduled task.
const PASSTHROUGH: [&str; 7] = ["--monitor", "--all", "--backdrop", "--workerw", "--under-icons", "--cols", "--debug-bg"];

fn message(text: &str, error: bool) {
    let text_w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text_w.as_ptr()),
            w!("DCC"),
            if error { MB_OK | MB_ICONWARNING } else { MB_OK | MB_ICONINFORMATION },
        );
    }
}

/// Re-launch this executable elevated with the same command. Returns false
/// if the user declined the UAC prompt.
fn relaunch_elevated(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe_w: Vec<u16> = exe.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let args_w: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let r = ShellExecuteW(None, w!("runas"), PCWSTR(exe_w.as_ptr()), PCWSTR(args_w.as_ptr()), None, SW_SHOWNORMAL);
        r.0 as usize > 32
    }
}

fn is_admin() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The flags after `--install` that should stay on the wallpaper's command
/// line, e.g. `dcc --install --monitor 1`.
fn passthrough_args(args: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if PASSTHROUGH.contains(&a.as_str()) {
            out.push(a.clone());
            if (a == "--monitor" || a == "--cols") && i + 1 < args.len() {
                out.push(args[i + 1].clone());
                i += 1;
            }
        }
        i += 1;
    }
    out.join(" ")
}

fn task_xml(exe: &str, workdir: &str, args: &str, user: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>DCC - live desktop dashboard wallpaper</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT3S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{args}</Arguments>
      <WorkingDirectory>{workdir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        user = xml_escape(user),
        exe = xml_escape(exe),
        args = xml_escape(args),
        workdir = xml_escape(workdir),
    )
}

/// Ask every running wallpaper to quit. Returns how many were found.
pub fn stop_running() -> usize {
    unsafe extern "system" fn visit(h: HWND, lp: LPARAM) -> BOOL {
        let found = &mut *(lp.0 as *mut Vec<HWND>);
        let mut buf = [0u16; 64];
        let n = GetClassNameW(h, &mut buf);
        if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "DccWallpaperWindow" {
            found.push(h);
        }
        BOOL(1)
    }
    unsafe extern "system" fn visit_top(h: HWND, lp: LPARAM) -> BOOL {
        let _ = visit(h, lp);
        // The injected wallpaper is a child of the desktop's view window.
        let _ = EnumChildWindows(Some(h), Some(visit), lp);
        BOOL(1)
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(visit_top), LPARAM(&mut found as *mut _ as isize));
        found.sort_by_key(|h| h.0 as usize);
        found.dedup();
        for h in &found {
            let _ = PostMessageW(Some(*h), WM_DCC_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    found.len()
}

pub fn install(args: &[String]) {
    let extra = passthrough_args(args);
    if !is_admin() {
        if !relaunch_elevated(&format!("--install {}", extra)) {
            message("Administrator rights are needed once, to register DCC to start at sign-in.", true);
        }
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let exe_s = exe.to_string_lossy().into_owned();
    let workdir = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    // DOMAIN\user when the domain is known; a bare user name resolves too.
    let name = std::env::var("USERNAME").unwrap_or_default();
    let user = match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{}\\{}", d, name),
        _ => name,
    };

    // schtasks wants the XML as UTF-16 with a byte-order mark.
    let xml = task_xml(&exe_s, &workdir, &extra, &user);
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let tmp = std::env::temp_dir().join("dcc-task.xml");
    if std::fs::write(&tmp, bytes).is_err() {
        message("Could not write the task definition to the temp folder.", true);
        return;
    }
    let ok = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &tmp.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&tmp);
    if !ok {
        message("Task Scheduler refused the startup task (schtasks /Create failed).", true);
        return;
    }

    // Replace whatever is running with the scheduled instance.
    if stop_running() > 0 {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    let started = schtasks(&["/Run", "/TN", TASK_NAME]);
    message(
        &format!(
            "DCC is installed{}.\n\n\
             It starts at every sign-in, with administrator rights for per-process network stats.\n\n\
             Settings: {}\n(or press Ctrl+` and type config)\n\n\
             Remove it any time with:  dcc.exe --uninstall",
            if started { " and running" } else { "" },
            crate::config::path().display()
        ),
        false,
    );
}

pub fn uninstall() {
    if !is_admin() {
        if !relaunch_elevated("--uninstall") {
            message("Administrator rights are needed to remove the startup task.", true);
        }
        return;
    }
    stop_running();
    let _ = schtasks(&["/End", "/TN", TASK_NAME]);
    let removed = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]);
    crate::net_etw::stop();
    message(
        &format!(
            "{}\n\nSettings and logs are kept in:\n{}\nDelete that folder to remove them as well.",
            if removed { "DCC was removed from startup and stopped." } else { "DCC stopped (no startup task was registered)." },
            crate::paths::data_dir().display()
        ),
        false,
    );
}

pub fn help() {
    message(
        &format!(
            "DCC — DevOps Command Center\n\n\
             dcc.exe\t\t\trun the wallpaper\n\
             --install\t\tstart at every sign-in\n\
             --uninstall\t\tremove from startup, stop\n\
             --stop\t\t\tstop the running wallpaper\n\
             --monitor N\t\tuse monitor N (0-based)\n\
             --all\t\t\tspan every monitor\n\
             --window\t\tpreview in a normal window\n\
             --shot f.png [w h] [--demo]\trender one frame to PNG\n\
             --restore-icons\tundo the icon-list tweak\n\n\
             Palette: Ctrl+`\n\
             Settings: {}",
            crate::config::path().display()
        ),
        false,
    );
}
