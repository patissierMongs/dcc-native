//! The network worker: weather, FX, BTC, headlines, calendar.
//!
//! One thread, one-minute heartbeat, per-feed refresh intervals. Every parser
//! here is a hand-rolled scanner over a known payload — the JSON "parsing" is
//! finding a key and reading the number after it, which is exactly as much
//! JSON as four stable public endpoints require. A failed fetch keeps the
//! previous value on screen; `ok` flags only flip when a feed has never
//! loaded, so a flaky minute never blanks a panel.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Weather {
    pub ok: bool,
    pub temp_c: f64,
    pub desc: String,
    pub wind_ms: f64,
    pub rain_prob: f64,
    pub tmin: f64,
    pub tmax: f64,
}

#[derive(Clone, Default)]
pub struct Ticker {
    pub ok: bool,
    pub usd_krw: f64,
    pub btc_krw: f64,
    pub btc_chg_pct: f64,
}

#[derive(Clone)]
pub struct Headline {
    pub title: String,
    pub url: String,
}

#[derive(Clone)]
pub struct CalEvent {
    /// "HH:MM", or empty for all-day.
    pub hm: String,
    pub summary: String,
}

#[derive(Clone, Default)]
pub struct AwApp {
    pub app: String,
    pub secs: u64,
}

/// Today's screen time, from the local ActivityWatch server.
#[derive(Clone, Default)]
pub struct AwDay {
    pub ok: bool,
    pub active_secs: u64,
    pub afk_secs: u64,
    pub apps: Vec<AwApp>,
}

#[derive(Clone, Default)]
pub struct Feeds {
    pub weather: Weather,
    pub ticker: Ticker,
    pub news: Vec<Headline>,
    pub cal: Vec<CalEvent>,
    pub cal_configured: bool,
    pub aw: AwDay,
}

static FEEDS: Mutex<Feeds> = Mutex::new(Feeds {
    weather: Weather {
        ok: false,
        temp_c: 0.0,
        desc: String::new(),
        wind_ms: 0.0,
        rain_prob: 0.0,
        tmin: 0.0,
        tmax: 0.0,
    },
    ticker: Ticker { ok: false, usd_krw: 0.0, btc_krw: 0.0, btc_chg_pct: 0.0 },
    news: Vec::new(),
    cal: Vec::new(),
    cal_configured: false,
    aw: AwDay { ok: false, active_secs: 0, afk_secs: 0, apps: Vec::new() },
});

pub fn get() -> Feeds {
    FEEDS.lock().map(|f| f.clone()).unwrap_or_default()
}

// ── tiny scanners ─────────────────────────────────────────────────────────

/// The first number after `"key":` — all these payloads put the value we want
/// in the first occurrence of its key.
fn num_after(json: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\":", key);
    let at = json.find(&pat)? + pat.len();
    let rest = json[at..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// First number inside the array after `"key":[`.
fn first_in_array(json: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\":[", key);
    let at = json.find(&pat)? + pat.len();
    let rest = &json[at..];
    let end = rest.find(|c: char| c == ',' || c == ']')?;
    rest[..end].trim().parse().ok()
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
}

fn tag_text<'a>(chunk: &'a str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let a = chunk.find(&open)? + open.len();
    let b = chunk[a..].find(&close)? + a;
    let raw = chunk[a..b].trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
        .unwrap_or(raw);
    Some(unescape(raw.trim()))
}

/// The href attribute of the first `<link .../>` in the chunk — how Atom
/// carries links, as opposed to RSS's `<link>text</link>`.
fn atom_href(chunk: &str) -> Option<String> {
    let at = chunk.find("<link")?;
    let tag_end = chunk[at..].find('>')? + at;
    let tag = &chunk[at..tag_end];
    for quote in ['\'', '"'] {
        if let Some(h) = tag.find("href=") {
            let rest = &tag[h + 5..];
            if rest.starts_with(quote) {
                if let Some(end) = rest[1..].find(quote) {
                    return Some(unescape(&rest[1..1 + end]));
                }
            }
        }
    }
    None
}

/// Both dialects: RSS 2.0 (`<item>` + `<link>text</link>`) and Atom
/// (`<entry>` + `<link href='…'/>`). Geek News is Atom, HN's rss is RSS —
/// a feed reader that only speaks one of them is half a feed reader.
fn parse_feed(xml: &str, cap: usize) -> Vec<Headline> {
    let mut out = Vec::new();
    let (sep, atom) = if xml.contains("<entry>") { ("<entry>", true) } else { ("<item>", false) };
    for chunk in xml.split(sep).skip(1) {
        if out.len() >= cap {
            break;
        }
        let Some(title) = tag_text(chunk, "title") else { continue };
        let url = if atom { atom_href(chunk) } else { tag_text(chunk, "link") };
        let Some(url) = url else { continue };
        if !title.is_empty() && url.starts_with("http") {
            out.push(Headline { title, url });
        }
    }
    out
}

// ── weather ───────────────────────────────────────────────────────────────

fn weather_desc(code: f64) -> &'static str {
    match code as i64 {
        0 => "clear",
        1..=2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 => "rain",
        71..=77 => "snow",
        80..=82 => "showers",
        85 | 86 => "snow showers",
        95..=99 => "thunderstorm",
        _ => "—",
    }
}

const WEATHER_URL: &str = "https://api.open-meteo.com/v1/forecast?latitude=37.5665&longitude=126.978&current=temperature_2m,weather_code,wind_speed_10m&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&wind_speed_unit=ms&timezone=Asia%2FSeoul&forecast_days=1";

fn fetch_weather() {
    let Some(body) = crate::http::get(WEATHER_URL) else { return };
    let Some(temp) = num_after(&body, "temperature_2m") else { return };
    let code = num_after(&body, "weather_code").unwrap_or(-1.0);
    let wind = num_after(&body, "wind_speed_10m").unwrap_or(0.0);
    let tmax = first_in_array(&body, "temperature_2m_max").unwrap_or(0.0);
    let tmin = first_in_array(&body, "temperature_2m_min").unwrap_or(0.0);
    let pop = first_in_array(&body, "precipitation_probability_max").unwrap_or(0.0);
    if let Ok(mut f) = FEEDS.lock() {
        f.weather = Weather {
            ok: true,
            temp_c: temp,
            desc: weather_desc(code).to_string(),
            wind_ms: wind,
            rain_prob: pop,
            tmin,
            tmax,
        };
    }
}

// ── ticker ────────────────────────────────────────────────────────────────

fn fetch_fx() {
    let Some(body) = crate::http::get("https://open.er-api.com/v6/latest/USD") else { return };
    let Some(krw) = num_after(&body, "KRW") else { return };
    if let Ok(mut f) = FEEDS.lock() {
        f.ticker.usd_krw = krw;
        f.ticker.ok = true;
    }
}

fn fetch_btc() {
    let Some(body) = crate::http::get("https://api.upbit.com/v1/ticker?markets=KRW-BTC") else {
        return;
    };
    let Some(price) = num_after(&body, "trade_price") else { return };
    let chg = num_after(&body, "signed_change_rate").unwrap_or(0.0) * 100.0;
    if let Ok(mut f) = FEEDS.lock() {
        f.ticker.btc_krw = price;
        f.ticker.btc_chg_pct = chg;
        f.ticker.ok = true;
    }
}

// ── headlines ─────────────────────────────────────────────────────────────

fn fetch_news() {
    let urls = crate::config::rss();
    let n_urls = urls.len();
    let mut all = Vec::new();
    for url in urls {
        if let Some(xml) = crate::http::get(&url) {
            all.extend(parse_feed(&xml, 8));
        }
        if all.len() >= 16 {
            break;
        }
    }
    if all.is_empty() {
        if n_urls > 0 {
            crate::events::journal(
                crate::events::Kind::Warn,
                &format!("news: 0 headlines from {} feed(s)", n_urls),
            );
        }
        return;
    }
    if let Ok(mut f) = FEEDS.lock() {
        f.news = all;
    }
}

// ── calendar (ICS) ────────────────────────────────────────────────────────

/// Minutes to add to UTC to get local time, measured from the two clocks we
/// already have rather than time-zone tables.
fn utc_offset_minutes() -> i64 {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::SystemTimeToFileTime;
    unsafe {
        let lt = windows::Win32::System::SystemInformation::GetLocalTime();
        let st = windows::Win32::System::SystemInformation::GetSystemTime();
        let to100 = |s: &SYSTEMTIME| -> i64 {
            let mut ft = FILETIME::default();
            let _ = SystemTimeToFileTime(s, &mut ft);
            ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
        };
        (to100(&lt) - to100(&st)) / 600_000_000
    }
}

/// Unfold RFC5545 lines (continuations start with a space or tab).
fn unfold(ics: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in ics.lines() {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.push_str(&line[1..]);
        } else {
            out.push(line.trim_end().to_string());
        }
    }
    out
}

fn fetch_calendar() {
    let Some(url) = crate::config::ics() else {
        if let Ok(mut f) = FEEDS.lock() {
            f.cal_configured = false;
        }
        return;
    };
    let Some(ics) = crate::http::get(&url) else { return };

    let (y, mo, d, _, _, _) = crate::collect::local_time();
    let today = format!("{:04}{:02}{:02}", y, mo, d);
    let off_min = utc_offset_minutes();

    let mut events: Vec<CalEvent> = Vec::new();
    let mut in_event = false;
    let mut start = String::new();
    let mut summary = String::new();
    for line in unfold(&ics) {
        if line == "BEGIN:VEVENT" {
            in_event = true;
            start.clear();
            summary.clear();
        } else if line == "END:VEVENT" {
            in_event = false;
            // start is either YYYYMMDD (all-day) or YYYYMMDDTHHMMSS[Z].
            if start.len() >= 8 {
                let (date, hm) = if start.len() >= 15 {
                    let utc = start.ends_with('Z');
                    let (mut hh, mut mi): (i64, i64) = (
                        start[9..11].parse().unwrap_or(0),
                        start[11..13].parse().unwrap_or(0),
                    );
                    let mut date = start[..8].to_string();
                    if utc {
                        // Shift into local; carry across midnight by whole days.
                        let total = hh * 60 + mi + off_min;
                        let day_shift = total.div_euclid(1440);
                        let rem = total.rem_euclid(1440);
                        hh = rem / 60;
                        mi = rem % 60;
                        if day_shift != 0 {
                            if let Ok(dd) = start[6..8].parse::<i64>() {
                                // Good enough for a glance panel: same-month carry.
                                date = format!("{}{:02}", &start[..6], dd + day_shift);
                            }
                        }
                    }
                    (date, format!("{:02}:{:02}", hh, mi))
                } else {
                    (start[..8].to_string(), String::new())
                };
                if date == today && !summary.is_empty() {
                    events.push(CalEvent { hm, summary: summary.clone() });
                }
            }
        } else if in_event {
            if let Some(v) = line.strip_prefix("SUMMARY:") {
                summary = v.to_string();
            } else if line.starts_with("DTSTART") {
                if let Some(colon) = line.find(':') {
                    start = line[colon + 1..].to_string();
                }
            }
        }
    }
    events.sort_by(|a, b| a.hm.cmp(&b.hm));
    events.truncate(6);
    if let Ok(mut f) = FEEDS.lock() {
        f.cal = events;
        f.cal_configured = true;
    }
}

// ── activitywatch ─────────────────────────────────────────────────────────

fn iso_utc(epoch: i64) -> String {
    let days = epoch.div_euclid(86400);
    let rem = epoch.rem_euclid(86400);
    let (y, m, d) = crate::claude::ymd_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// The full bucket id whose name starts with `prefix` (ids carry the host).
fn find_bucket(json: &str, prefix: &str) -> Option<String> {
    let at = json.find(&format!("\"{}", prefix))? + 1;
    let end = json[at..].find('"')? + at;
    Some(json[at..end].to_string())
}

/// Today's window/afk events from aw-server, aggregated by app.
fn fetch_aw() {
    const BASE: &str = "http://127.0.0.1:5600/api/0";
    let Some(buckets) = crate::http::get(&format!("{}/buckets/", BASE)) else {
        if let Ok(mut f) = FEEDS.lock() {
            f.aw.ok = false;
        }
        return;
    };
    let off_min = utc_offset_minutes();
    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, mo, d, _, _, _) = crate::collect::local_time();
    let midnight_utc = crate::claude::days_from_ymd(y as i64, mo as i64, d as i64) * 86400 - off_min * 60;
    let range = format!("start={}&end={}&limit=15000", iso_utc(midnight_utc), iso_utc(now_epoch));

    let mut day = AwDay::default();

    if let Some(id) = find_bucket(&buckets, "aw-watcher-window_") {
        if let Some(body) = crate::http::get(&format!("{}/buckets/{}/events?{}", BASE, id, range)) {
            let mut agg: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
            for chunk in body.split("\"timestamp\"").skip(1) {
                let dur = num_after(chunk, "duration").unwrap_or(0.0);
                let Some(app) = str_after_local(chunk, "app") else { continue };
                let app = app.to_lowercase();
                let app = app.strip_suffix(".exe").unwrap_or(&app).to_string();
                *agg.entry(app).or_insert(0.0) += dur;
            }
            let mut apps: Vec<AwApp> = agg
                .into_iter()
                .map(|(app, s)| AwApp { app, secs: s as u64 })
                .filter(|a| a.secs >= 60)
                .collect();
            apps.sort_by(|a, b| b.secs.cmp(&a.secs));
            apps.truncate(6);
            day.apps = apps;
            day.ok = true;
        }
    }
    if let Some(id) = find_bucket(&buckets, "aw-watcher-afk_") {
        if let Some(body) = crate::http::get(&format!("{}/buckets/{}/events?{}", BASE, id, range)) {
            for chunk in body.split("\"timestamp\"").skip(1) {
                let dur = num_after(chunk, "duration").unwrap_or(0.0) as u64;
                match str_after_local(chunk, "status").as_deref() {
                    Some("not-afk") => day.active_secs += dur,
                    Some("afk") => day.afk_secs += dur,
                    _ => {}
                }
            }
        }
    }

    if let Ok(mut f) = FEEDS.lock() {
        f.aw = day;
    }
}

/// String value after `"key":` — tolerating the space aw-server puts after
/// the colon, which the compact-JSON parsers elsewhere never see.
fn str_after_local(hay: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\":", key);
    let at = hay.find(&pat)? + pat.len();
    let rest = hay[at..].trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ── the worker ────────────────────────────────────────────────────────────

/// Fetch everything once, synchronously — used by `--shot` so previews show
/// live data, which doubles as an end-to-end test of every fetcher.
pub fn prefetch() {
    fetch_weather();
    fetch_fx();
    fetch_btc();
    fetch_news();
    fetch_calendar();
    fetch_aw();
}

pub fn spawn() {
    std::thread::spawn(|| {
        let t0 = Instant::now() - Duration::from_secs(3600);
        let mut last = [t0; 6]; // weather, fx, btc, news, cal, aw
        let every = [600u64, 1800, 300, 600, 900, 120];
        let jobs: [fn(); 6] = [fetch_weather, fetch_fx, fetch_btc, fetch_news, fetch_calendar, fetch_aw];
        loop {
            for i in 0..jobs.len() {
                if last[i].elapsed().as_secs() >= every[i] {
                    last[i] = Instant::now();
                    jobs[i]();
                }
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
}
