//! `dcc.conf` — one small hand-parsed file instead of a TOML dependency.
//!
//! The format is sections of plain lines — pipe-separated for lists,
//! `key = value` for settings:
//!
//! ```text
//! [launcher]
//! # label | target | optional args
//! VS Code   | code
//! Downloads | shell:Downloads
//! GitHub    | https://github.com
//!
//! [weather]
//! city = Seoul
//! lat  = 37.5665
//! lon  = 126.978
//! ```
//!
//! The file lives in the data folder (see `paths`). A missing file is created
//! with a commented template so there is something to edit. The watch thread
//! re-reads it when the modification time changes — saving the file is all
//! it takes to update the wallpaper.

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
    /// Weather location: a label for the panel and coordinates for the API.
    pub city: String,
    pub lat: f64,
    pub lon: f64,
    /// Currency the USD rate is quoted in; "off" hides the rate.
    pub currency: String,
    /// One Upbit market (e.g. KRW-BTC, USDT-ETH); "off" hides it.
    pub coin: String,
    /// Where the drop panel's "drive" zone copies to (Google Drive mirror by
    /// default, any synced folder works).
    pub drive: Option<String>,
}

const fn defaults() -> Config {
    Config {
        launcher: Vec::new(),
        rss: Vec::new(),
        ics: None,
        git_roots: Vec::new(),
        pixel_shift: true,
        night_dim: 0.55,
        night_start: 23,
        night_end: 8,
        theme: String::new(),
        city: String::new(),
        lat: 37.5665,
        lon: 126.978,
        currency: String::new(),
        coin: String::new(),
        drive: None,
    }
}

impl Default for Config {
    fn default() -> Self {
        defaults()
    }
}

static CONFIG: Mutex<Config> = Mutex::new(defaults());

pub fn path() -> std::path::PathBuf {
    crate::paths::file("dcc.conf")
}

const TEMPLATE: &str = "\
# dcc.conf — saved changes apply live, no restart needed.
# Open it any time from the palette: Ctrl+` then type 'config'.

# [launcher]  label | target | optional args
# target can be an exe, a folder, a URL or a shell: path. Entries show up
# in the palette (Ctrl+`) and filter as you type.
[launcher]
Downloads   | shell:Downloads
Documents   | shell:Personal
Terminal    | wt.exe
Task Mgr    | taskmgr.exe

# [rss] one RSS or Atom feed URL per line — headlines show in the news panel.
[rss]
https://news.hada.io/rss/news

# [calendar] a single secret ICS address; today's events show in the day
# panel. Google Calendar: Settings > your calendar > 'Secret address in iCal
# format'. Treat that address like a password.
[calendar]
# https://calendar.google.com/calendar/ical/....../basic.ics

# [weather] location for the day panel (Open-Meteo, no API key needed).
[weather]
city = Seoul
lat = 37.5665
lon = 126.978

# [ticker] currency = the USD exchange rate's quote currency (off to hide).
# coin = one Upbit market such as KRW-BTC or USDT-ETH (off to hide).
[ticker]
currency = KRW
coin = KRW-BTC

# [git] roots to scan (two levels deep) for repositories.
[git]
%USERPROFILE%\\Documents

# [drop] where the drop panel's 'drive' zone copies files. Leave commented
# to auto-detect a Google Drive for Desktop mirror.
[drop]
# drive = G:\\My Drive\\Inbox

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
        let kv = line.split_once('=').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim()));
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
            "weather" => match kv {
                Some((k, v)) if k == "city" => cfg.city = v.to_string(),
                Some((k, v)) if k == "lat" => cfg.lat = v.parse().unwrap_or(cfg.lat),
                Some((k, v)) if k == "lon" => cfg.lon = v.parse().unwrap_or(cfg.lon),
                _ => {}
            },
            "ticker" => match kv {
                Some((k, v)) if k == "currency" => cfg.currency = v.to_ascii_uppercase(),
                Some((k, v)) if k == "coin" => cfg.coin = v.to_ascii_uppercase(),
                _ => {}
            },
            "drop" => {
                if let Some((k, v)) = kv {
                    if k == "drive" && !v.is_empty() {
                        cfg.drive = Some(v.to_string());
                    }
                }
            }
            "display" => {
                if let Some((k, v)) = kv {
                    match k.as_str() {
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

/// The configured roots, or Documents when the section is empty.
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

/// (label, lat, lon) — the label defaults to "Seoul" to match the default
/// coordinates.
pub fn weather() -> (String, f64, f64) {
    CONFIG
        .lock()
        .map(|c| {
            let city = if c.city.is_empty() { "Seoul".to_string() } else { c.city.clone() };
            (city, c.lat, c.lon)
        })
        .unwrap_or_else(|_| ("Seoul".into(), 37.5665, 126.978))
}

/// (quote currency for USD, Upbit market) — empty string means hidden.
pub fn ticker() -> (String, String) {
    let (cur, coin) = CONFIG
        .lock()
        .map(|c| (c.currency.clone(), c.coin.clone()))
        .unwrap_or_default();
    let cur = match cur.as_str() {
        "" => "KRW".to_string(),
        "OFF" => String::new(),
        _ => cur,
    };
    let coin = match coin.as_str() {
        "" => "KRW-BTC".to_string(),
        "OFF" => String::new(),
        _ => coin,
    };
    (cur, coin)
}

pub fn drive() -> Option<String> {
    CONFIG.lock().ok().and_then(|c| c.drive.clone())
}
