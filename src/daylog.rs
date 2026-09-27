//! Persistent day log + daily anomaly digest.
//!
//! Two consumers, one file. Every journal entry and every closed spike is
//! appended to `logs\dcc-day-YYYYMMDD.log` in the data folder, so a burst of
//! bandwidth at boot (an installer stub, an updater service) can still be
//! traced hours later. On top of the file an in-memory digest aggregates today's
//! anomalies — warn/err events and spike excursions folded by subject — and
//! the `day` panel renders that instead of a raw scroll. Restarts replay
//! today's file, so the digest covers the day, not the process lifetime.
//!
//! Line formats (pipe-separated, free text always last):
//!   S|HH:MM:SS|metric|dur_secs|peak_raw|peak_txt|name
//!   J|HH:MM:SS|K|text                (K: I/O/W/E)

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::events::Kind;

/// Appends only happen in the real wallpaper run — `--shot` previews and
/// `--window` layout checks replay the file but never write to it.
static APPEND: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
pub struct SpikeAgg {
    pub metric: String,
    pub name: String,
    pub count: u32,
    pub peak: f64,
    pub peak_txt: String,
    pub total_secs: u64,
    pub last_hms: String,
}

#[derive(Clone)]
pub struct EventAgg {
    pub kind: Kind,
    pub text: String,
    pub count: u32,
    pub last_hms: String,
}

struct Digest {
    day: (u16, u16, u16),
    spikes: HashMap<String, SpikeAgg>,
    events: HashMap<String, EventAgg>,
}

static DIGEST: Mutex<Option<Digest>> = Mutex::new(None);

fn today() -> (u16, u16, u16) {
    let (y, mo, d, _, _, _) = crate::collect::local_time();
    (y, mo, d)
}

pub fn path_today() -> std::path::PathBuf {
    let (y, mo, d) = today();
    crate::paths::logs_dir().join(format!("dcc-day-{:04}{:02}{:02}.log", y, mo, d))
}

fn append_line(line: &str) {
    if !APPEND.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut f) =
        std::fs::OpenOptions::new().create(true).append(true).open(path_today())
    {
        let _ = writeln!(f, "{}", line);
    }
}

/// Fold a record into the digest, resetting it when the day has rolled over.
fn with_digest(f: impl FnOnce(&mut Digest)) {
    let Ok(mut guard) = DIGEST.lock() else { return };
    let day = today();
    let d = guard.get_or_insert_with(|| Digest {
        day,
        spikes: HashMap::new(),
        events: HashMap::new(),
    });
    if d.day != day {
        *d = Digest { day, spikes: HashMap::new(), events: HashMap::new() };
    }
    f(d);
}

fn digest_spike(d: &mut Digest, metric: &str, name: &str, peak: f64, peak_txt: &str, dur: u64, hms: &str) {
    let key = format!("{}|{}", metric, name);
    let e = d.spikes.entry(key).or_insert_with(|| SpikeAgg {
        metric: metric.to_string(),
        name: name.to_string(),
        count: 0,
        peak: 0.0,
        peak_txt: peak_txt.to_string(),
        total_secs: 0,
        last_hms: hms.to_string(),
    });
    e.count += 1;
    e.total_secs += dur;
    e.last_hms = hms.to_string();
    if peak >= e.peak {
        e.peak = peak;
        e.peak_txt = peak_txt.to_string();
    }
}

fn digest_event(d: &mut Digest, kind: Kind, hms: &str, text: &str) {
    // The digest is for anomalies; routine info/ok entries stay file-only.
    if !matches!(kind, Kind::Warn | Kind::Err) {
        return;
    }
    let e = d.events.entry(text.to_string()).or_insert_with(|| EventAgg {
        kind,
        text: text.to_string(),
        count: 0,
        last_hms: hms.to_string(),
    });
    e.count += 1;
    e.last_hms = hms.to_string();
    if kind == Kind::Err {
        e.kind = Kind::Err;
    }
}

/// A closed spike excursion — called by the collector's sweep.
pub fn spike(metric: &str, name: &str, peak: f64, peak_txt: &str, dur_secs: u64, hms: &str) {
    append_line(&format!(
        "S|{}|{}|{}|{:.1}|{}|{}",
        hms, metric, dur_secs, peak, peak_txt, name
    ));
    with_digest(|d| digest_spike(d, metric, name, peak, peak_txt, dur_secs, hms));
}

/// A journal entry — called by the event bus for every entry it records.
pub fn event(kind: Kind, hms: &str, text: &str) {
    let k = match kind {
        Kind::Info => 'I',
        Kind::Ok => 'O',
        Kind::Warn => 'W',
        Kind::Err => 'E',
    };
    append_line(&format!("J|{}|{}|{}", hms, k, text));
    with_digest(|d| digest_event(d, kind, hms, text));
}

/// Rebuild the digest from today's file. Returns how many lines were
/// replayed. Call before enabling appends so nothing is counted twice.
pub fn replay_today() -> usize {
    let Ok(f) = std::fs::File::open(path_today()) else { return 0 };
    let mut fresh = Digest { day: today(), spikes: HashMap::new(), events: HashMap::new() };
    let mut n = 0usize;
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        let mut it = line.splitn(7, '|');
        match it.next() {
            Some("S") => {
                let (Some(hms), Some(metric), Some(dur), Some(peak), Some(peak_txt), Some(name)) =
                    (it.next(), it.next(), it.next(), it.next(), it.next(), it.next())
                else {
                    continue;
                };
                let dur = dur.parse::<u64>().unwrap_or(0);
                let peak = peak.parse::<f64>().unwrap_or(0.0);
                digest_spike(&mut fresh, metric, name, peak, peak_txt, dur, hms);
                n += 1;
            }
            Some("J") => {
                // J|hms|K|text — text may itself contain pipes.
                let mut it = line.splitn(4, '|');
                it.next();
                let (Some(hms), Some(k), Some(text)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                let kind = match k {
                    "O" => Kind::Ok,
                    "W" => Kind::Warn,
                    "E" => Kind::Err,
                    _ => Kind::Info,
                };
                digest_event(&mut fresh, kind, hms, text);
                n += 1;
            }
            _ => {}
        }
    }
    if let Ok(mut guard) = DIGEST.lock() {
        *guard = Some(fresh);
    }
    n
}

/// Replay today's file, then start persisting (unless this is a preview run).
pub fn init(persist: bool) -> usize {
    let n = replay_today();
    APPEND.store(persist, Ordering::Relaxed);
    n
}

/// What the `day` panel shows: errors first, then warnings, then spike
/// excursions by accumulated duration.
pub enum Line {
    Event(EventAgg),
    Spike(SpikeAgg),
}

pub fn lines(max: usize) -> Vec<Line> {
    let Ok(guard) = DIGEST.lock() else { return Vec::new() };
    let Some(d) = guard.as_ref() else { return Vec::new() };
    let mut evs: Vec<EventAgg> = d.events.values().cloned().collect();
    evs.sort_by(|a, b| {
        let rank = |k: Kind| if k == Kind::Err { 0 } else { 1 };
        rank(a.kind).cmp(&rank(b.kind)).then(b.last_hms.cmp(&a.last_hms))
    });
    let mut sps: Vec<SpikeAgg> = d.spikes.values().cloned().collect();
    sps.sort_by(|a, b| b.total_secs.cmp(&a.total_secs).then(b.count.cmp(&a.count)));

    let mut out: Vec<Line> = Vec::new();
    // Events get at most half the panel so a noisy warning cannot push every
    // spike out of view.
    for e in evs.into_iter().take(max.div_ceil(2)) {
        out.push(Line::Event(e));
    }
    for s in sps {
        if out.len() >= max {
            break;
        }
        out.push(Line::Spike(s));
    }
    out.truncate(max);
    out
}
