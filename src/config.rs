//! `dcc.conf` — one small hand-parsed file instead of a TOML dependency.
//!
//! The format is sections of pipe-separated lines:
//!
//! ```text
//! [launcher]
//! # label | target | optional args
//! VS Code   | C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe
//! Downloads | C:\Users\me\Downloads
//! GitHub    | https://github.com
//! ```
//!
//! The file lives next to the executable, like dcc.log. A missing file is
//! created with a commented template so there is something to edit. The watch
//! thread re-reads it when the modification time changes — saving the file is
//! all it takes to update the wallpaper.

use std::sync::Mutex;

#[derive(Clone)]
pub struct LauncherItem {
    pub label: String,
    pub target: String,
    pub args: String,
}

#[derive(Clone)]
pub struct Config {
    pub launcher: Vec<LauncherItem>,
    pub rss: Vec<String>,
    pub ics: Option<String>,
    /// Roots scanned (two levels deep) for git repositories.
    pub git_roots: Vec<String>,
    /// OLED care: drift the whole render a few pixels every few minutes.
    pub pixel_shift: bool,
    /// Brightness factor during night hours (1.0 = off).
    pub night_dim: f64,
    pub night_start: u32,
    pub night_end: u32,
    /// "phosphor" (default) or "soft".
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            launcher: Vec::new(),
            rss: Vec::new(),
            ics: None,
            git_roots: vec!["%USERPROFILE%\\Documents".into()],
            pixel_shift: true,
            night_dim: 0.55,
            night_start: 23,
            night_end: 8,
            theme: "phosphor".into(),
        }
    }
}

static CONFIG: Mutex<Config> = Mutex::new(Config {
    launcher: Vec::new(),
    rss: Vec::new(),
    ics: None,
    git_roots: Vec::new(),
    pixel_shift: true,
    night_dim: 0.55,
    night_start: 23,
    night_end: 8,
    theme: String::new(),
});

pub fn path() -> std::path::PathBuf {
    std::env::current_exe()
        .map(|p| p.with_file_name("dcc.conf"))
        .unwrap_or_else(|_| "dcc.conf".into())
}

const TEMPLATE: &str = "\
# dcc.conf — saved changes apply live, no restart needed.
#
# [launcher] lines are:  label | target | optional args
# target can be an exe, a folder, or a URL.

[launcher]
Downloads   | shell:Downloads
Documents   | shell:Personal
Terminal    | wt.exe
Task Mgr    | taskmgr.exe

# [rss] one feed URL per line — headlines show in the news panel.
[rss]
https://news.hada.io/rss/news

# [calendar] a single secret ICS address; today's events show in the day
# panel. Google Calendar: settings > your calendar > 'Secret address in
# iCal format'.
[calendar]
# https://calendar.google.com/calendar/ical/....../basic.ics

# [git] roots to scan (two levels deep) for repositories.
[git]
%USERPROFILE%\\Documents

# [display] OLED care. pixel_shift drifts the render a few px every 5 min;
# night_dim darkens everything during night_hours (local, start-end).
[display]
pixel_shift = on
night_dim = 0.55
night_hours = 23-08
# theme = phosphor | soft (lower saturation, easier on long stares)
theme = phosphor
";

fn parse(text: &str) -> Config {
    let mut cfg = Config::default();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            continue;
        }
        match section.as_str() {
            "launcher" => {
                let mut parts = line.splitn(3, '|').map(str::trim);
                let label = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let args = parts.next().unwrap_or("").to_string();
                if !label.is_empty() && !target.is_empty() {
                    cfg.launcher.push(LauncherItem { label, target, args });
                }
            }
            "rss" => {
                if line.starts_with("http") {
                    cfg.rss.push(line.to_string());
                }
            }
            "git" => {
                cfg.git_roots.push(line.to_string());
            }
            "calendar" => {
                if line.starts_with("http") && cfg.ics.is_none() {
                    cfg.ics = Some(line.to_string());
                }
            }
            "display" => {
                if let Some((k, v)) = line.split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    match k {
                        "pixel_shift" => cfg.pixel_shift = v != "off" && v != "false" && v != "0",
                        "night_dim" => {
                            if let Ok(f) = v.parse::<f64>() {
                                cfg.night_dim = f.clamp(0.2, 1.0);
                            }
                        }
                        "theme" => cfg.theme = v.to_lowercase(),
                        "night_hours" => {
                            if let Some((a, b)) = v.split_once('-') {
                                if let (Ok(s), Ok(e)) = (a.trim().parse(), b.trim().parse()) {
                                    cfg.night_start = s;
                                    cfg.night_end = e;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    cfg
}

/// Read the file (writing the template first if absent) and publish it.
pub fn load() {
    let p = path();
    if !p.exists() {
        let _ = std::fs::write(&p, TEMPLATE);
    }
    if let Ok(text) = std::fs::read_to_string(&p) {
        if let Ok(mut c) = CONFIG.lock() {
            *c = parse(&text);
        }
    }
}

pub fn launcher() -> Vec<LauncherItem> {
    CONFIG.lock().map(|c| c.launcher.clone()).unwrap_or_default()
}

pub fn rss() -> Vec<String> {
    CONFIG.lock().map(|c| c.rss.clone()).unwrap_or_default()
}

pub fn ics() -> Option<String> {
    CONFIG.lock().ok().and_then(|c| c.ics.clone())
}

pub fn git_roots() -> Vec<String> {
    let roots = CONFIG.lock().map(|c| c.git_roots.clone()).unwrap_or_default();
    if roots.is_empty() {
        vec!["%USERPROFILE%\\Documents".into()]
    } else {
        roots
    }
}

/// (pixel_shift, night_dim, night_start, night_end)
pub fn display() -> (bool, f64, u32, u32) {
    CONFIG
        .lock()
        .map(|c| (c.pixel_shift, c.night_dim, c.night_start, c.night_end))
        .unwrap_or((true, 0.55, 23, 8))
}

pub fn soft_theme() -> bool {
    CONFIG.lock().map(|c| c.theme == "soft").unwrap_or(false)
}
