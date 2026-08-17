//! Claude Code usage, straight from the transcripts.
//!
//! Every session Claude Code runs leaves a JSONL file under
//! `~/.claude/projects/<encoded-cwd>/<session>.jsonl`; assistant messages in
//! it carry a `usage` block with exact token counts and the model name. No
//! hooks, no servers — the wallpaper reads what is already on disk, the same
//! way ccusage does.
//!
//! Parsing is incremental: per file we remember the byte offset we've
//! consumed and only read what was appended, so steady-state cost is
//! proportional to new tokens, not transcript size. Costs are estimates from
//! a static price table — good enough for a wall.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use crate::state::{Claude, ClaudeSession};

/// Aggregates for one transcript file.
#[derive(Default, Clone)]
struct FileAgg {
    offset: u64,
    /// yyyymmdd (local) -> (tokens, cost)
    days: HashMap<u32, (u64, f64)>,
    cache_read: u64,
    cache_write: u64,
    model: String,
    project: String,
}

static OUT: Mutex<Option<Claude>> = Mutex::new(None);

pub fn current() -> Claude {
    OUT.lock().ok().and_then(|o| o.clone()).unwrap_or_default()
}

fn num_after(hay: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\":", key);
    let at = hay.find(&pat)? + pat.len();
    let rest = hay[at..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn str_after(hay: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\":\"", key);
    let at = hay.find(&pat)? + pat.len();
    let end = hay[at..].find('"')? + at;
    Some(hay[at..end].to_string())
}

/// "claude-fable-5-20260401" -> "fable-5"
fn model_short(model: &str) -> String {
    let m = model.strip_prefix("claude-").unwrap_or(model);
    let parts: Vec<&str> = m.split('-').collect();
    let keep: Vec<&str> = parts.iter().take_while(|p| p.parse::<u64>().map(|n| n < 1000).unwrap_or(true)).copied().collect();
    if keep.is_empty() { m.to_string() } else { keep.join("-") }
}

/// $ per million tokens (input, output) — estimates.
fn price(model: &str) -> (f64, f64) {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") || m.contains("fable") || m.contains("mythos") {
        (15.0, 75.0)
    } else if m.contains("haiku") {
        (0.8, 4.0)
    } else {
        (3.0, 15.0) // sonnet & default
    }
}

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

/// "2026-08-17T18:03:12.345Z" -> local yyyymmdd, minute-precision shift.
fn local_day(ts: &str, off_min: i64) -> Option<u32> {
    if ts.len() < 16 {
        return None;
    }
    let y: i64 = ts.get(0..4)?.parse().ok()?;
    let mo: i64 = ts.get(5..7)?.parse().ok()?;
    let d: i64 = ts.get(8..10)?.parse().ok()?;
    let h: i64 = ts.get(11..13)?.parse().ok()?;
    let mi: i64 = ts.get(14..16)?.parse().ok()?;
    let total = h * 60 + mi + off_min;
    let day_shift = total.div_euclid(1440);
    // Day arithmetic via a rough serial-date round trip.
    let serial = days_from_ymd(y, mo, d) + day_shift;
    let (yy, mm, dd) = ymd_from_days(serial);
    Some((yy * 10000 + mm * 100 + dd) as u32)
}

pub fn days_from_ymd(y: i64, m: i64, d: i64) -> i64 {
    // Howard Hinnant's civil-from-days inverse.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn ymd_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn today_local() -> u32 {
    let (y, mo, d, _, _, _) = crate::collect::local_time();
    y as u32 * 10000 + mo as u32 * 100 + d as u32
}

/// Consume newly appended lines of one transcript into its aggregate.
fn ingest(path: &PathBuf, agg: &mut FileAgg, off_min: i64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len <= agg.offset {
        agg.offset = agg.offset.min(len);
        return;
    }
    if f.seek(SeekFrom::Start(agg.offset)).is_err() {
        return;
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return;
    }
    // Keep a possibly-incomplete trailing line for the next pass.
    let consumed = match buf.rfind('\n') {
        Some(p) => p + 1,
        None => 0,
    };
    agg.offset += consumed as u64;
    for line in buf[..consumed].lines() {
        if agg.project.is_empty() {
            if let Some(cwd) = str_after(line, "cwd") {
                let clean = cwd.replace("\\\\", "\\");
                if let Some(last) = clean.rsplit(['\\', '/']).next() {
                    agg.project = last.to_string();
                }
            }
        }
        let Some(upos) = line.find("\"usage\":") else { continue };
        let usage = &line[upos..];
        let inp = num_after(usage, "input_tokens").unwrap_or(0.0);
        let out = num_after(usage, "output_tokens").unwrap_or(0.0);
        let cr = num_after(usage, "cache_read_input_tokens").unwrap_or(0.0);
        let cw = num_after(usage, "cache_creation_input_tokens").unwrap_or(0.0);
        if inp + out + cr + cw <= 0.0 {
            continue;
        }
        if let Some(model) = str_after(line, "model") {
            agg.model = model;
        }
        let (p_in, p_out) = price(&agg.model);
        let cost = (inp * p_in + out * p_out + cr * p_in * 0.1 + cw * p_in * 1.25) / 1e6;
        let day = str_after(line, "timestamp")
            .and_then(|ts| local_day(&ts, off_min))
            .unwrap_or_else(today_local);
        let e = agg.days.entry(day).or_insert((0, 0.0));
        e.0 += (inp + out + cr + cw) as u64;
        e.1 += cost;
        agg.cache_read += cr as u64;
        agg.cache_write += cw as u64;
    }
}

pub fn spawn() {
    std::thread::spawn(|| {
        let Some(profile) = std::env::var_os("USERPROFILE") else { return };
        let root = PathBuf::from(profile).join(".claude").join("projects");
        let mut files: HashMap<PathBuf, FileAgg> = HashMap::new();
        let mut announced = false;

        loop {
            let off_min = utc_offset_minutes();
            let now = std::time::SystemTime::now();
            let horizon = Duration::from_secs(8 * 86400);

            if let Ok(projects) = std::fs::read_dir(&root) {
                for proj in projects.flatten() {
                    let Ok(sessions) = std::fs::read_dir(proj.path()) else { continue };
                    for s in sessions.flatten() {
                        let path = s.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                            continue;
                        }
                        let Ok(meta) = s.metadata() else { continue };
                        let fresh = meta
                            .modified()
                            .ok()
                            .and_then(|m| now.duration_since(m).ok())
                            .map(|age| age < horizon)
                            .unwrap_or(false);
                        if !fresh && !files.contains_key(&path) {
                            continue;
                        }
                        let agg = files.entry(path.clone()).or_default();
                        ingest(&path, agg, off_min);
                    }
                }
            }

            // Fold per-file aggregates into the panel struct.
            let today = today_local();
            let mut days: HashMap<u32, (u64, f64)> = HashMap::new();
            let (mut cr, mut cw) = (0u64, 0u64);
            let mut model_tokens: HashMap<String, u64> = HashMap::new();
            let mut sessions: Vec<ClaudeSession> = Vec::new();
            for (path, agg) in &files {
                for (d, (tok, cost)) in &agg.days {
                    let e = days.entry(*d).or_insert((0, 0.0));
                    e.0 += tok;
                    e.1 += cost;
                }
                cr += agg.cache_read;
                cw += agg.cache_write;
                if !agg.model.is_empty() {
                    let today_tok = agg.days.get(&today).map(|d| d.0).unwrap_or(0);
                    *model_tokens.entry(model_short(&agg.model)).or_insert(0) += today_tok;
                    if let Ok(meta) = std::fs::metadata(path) {
                        if let Some(age) = meta.modified().ok().and_then(|m| now.duration_since(m).ok()) {
                            if age < Duration::from_secs(6 * 3600) {
                                sessions.push(ClaudeSession {
                                    project: if agg.project.is_empty() { "?".into() } else { agg.project.clone() },
                                    model: model_short(&agg.model),
                                    idle_secs: age.as_secs(),
                                    today_tokens: today_tok,
                                });
                            }
                        }
                    }
                }
            }
            sessions.sort_by_key(|s| s.idle_secs);
            sessions.truncate(4);

            let mut day_keys: Vec<u32> = days.keys().copied().collect();
            day_keys.sort_unstable();
            let sparkline: Vec<f64> = day_keys.iter().map(|d| days[d].0 as f64).collect();
            let last7: Vec<u32> = day_keys.iter().rev().take(7).copied().collect();
            let (today_tok, today_cost) = days.get(&today).copied().unwrap_or((0, 0.0));
            let (week_tok, week_cost) = last7
                .iter()
                .fold((0u64, 0.0f64), |acc, d| (acc.0 + days[d].0, acc.1 + days[d].1));
            let top_model = model_tokens
                .iter()
                .max_by_key(|(_, v)| **v)
                .map(|(k, _)| k.clone())
                .unwrap_or_default();

            let live = sessions.first().map(|s| s.idle_secs < 120).unwrap_or(false);
            let out = Claude {
                ok: !days.is_empty(),
                today_tokens: today_tok,
                today_cost,
                week_tokens: week_tok,
                week_cost,
                cache_read: cr,
                cache_write: cw,
                sparkline,
                top_model,
                session_project: sessions.first().map(|s| s.project.clone()).unwrap_or_default(),
                session_idle_secs: sessions.first().map(|s| s.idle_secs).unwrap_or(u64::MAX / 2),
                sessions,
            };
            if !announced && out.ok {
                announced = true;
                crate::events::journal(
                    crate::events::Kind::Ok,
                    &format!("claude: transcripts parsed — today {} tokens", out.today_tokens),
                );
            }
            let _ = live;
            if let Ok(mut guard) = OUT.lock() {
                *guard = Some(out);
            }

            std::thread::sleep(Duration::from_secs(60));
        }
    });
}
