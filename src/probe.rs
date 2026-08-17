//! `dcc --probe` dumps the desktop window family into the log.
//!
//! Run from the app rather than a script so the numbers are DPI-correct and
//! come from exactly the same API calls the attach path uses.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn class_of(h: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 128];
        let n = GetClassNameW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn line(h: HWND) -> String {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        let has_defview = FindWindowExW(Some(h), None, w!("SHELLDLL_DefView"), None).is_ok();
        format!(
            "{:?} {:<18} {}x{} at {},{} vis={} style={:08x}{}",
            h.0,
            class_of(h),
            r.right - r.left,
            r.bottom - r.top,
            r.left,
            r.top,
            IsWindowVisible(h).as_bool(),
            style,
            if has_defview { "  <-- hosts SHELLDLL_DefView" } else { "" }
        )
    }
}

unsafe extern "system" fn collect_top(h: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp.0 as *mut Vec<HWND>);
    let c = class_of(h);
    if c == "WorkerW" || c == "Progman" {
        out.push(h);
    }
    BOOL(1)
}

/// Walk the children of `parent`, one level, reporting each.
fn children(parent: HWND, out: &mut Vec<String>, indent: &str) {
    unsafe {
        let mut prev: Option<HWND> = None;
        for _ in 0..40 {
            match FindWindowExW(Some(parent), prev, None, None) {
                Ok(c) if !c.0.is_null() => {
                    out.push(format!("{}{}", indent, line(c)));
                    if indent.len() < 8 {
                        children(c, out, &format!("{}    ", indent));
                    }
                    prev = Some(c);
                }
                _ => break,
            }
        }
    }
}

pub fn report(nudge: bool) -> String {
    let mut out = Vec::new();
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok();
        out.push(format!("Progman found: {}", progman.is_some()));

        if nudge {
            if let Some(pm) = progman {
                let mut ignored: usize = 0;
                for (wp, lp) in [(0usize, 0isize), (0x0D, 0x01)] {
                    let _ = SendMessageTimeoutW(
                        pm, 0x052C, WPARAM(wp), LPARAM(lp), SMTO_NORMAL, 2000, Some(&mut ignored),
                    );
                }
                out.push("nudged Progman with 0x052C (0,0) and (0x0D,0x01)".into());
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }

        if let Some(pm) = progman {
            out.push(format!("PROGMAN  {}", line(pm)));
            children(pm, &mut out, "    ");
        }

        out.push("TOP-LEVEL Progman/WorkerW:".into());
        let mut tops: Vec<HWND> = Vec::new();
        let _ = EnumWindows(Some(collect_top), LPARAM(&mut tops as *mut _ as isize));
        for h in tops {
            out.push(format!("    {}", line(h)));
            children(h, &mut out, "        ");
        }
    }
    out.join("\n")
}
