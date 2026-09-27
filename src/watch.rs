//! Background watcher: the Downloads folder and the config file.
//!
//! One thread, one 2-second cadence. Downloads are diffed by directory
//! snapshot rather than ReadDirectoryChangesW — a single folder listing every
//! two seconds is nothing, and polling sidesteps the whole async-completion
//! dance. A file counts as "arrived" once it exists with a stable size and is
//! not a browser temp file, so half-written downloads do not toast early.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::events;

const TEMP_EXT: [&str; 5] = ["crdownload", "tmp", "part", "partial", "download"];

fn downloads_dir() -> Option<PathBuf> {
    crate::paths::downloads().filter(|p| p.is_dir())
}

fn is_temp(p: &PathBuf) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) => TEMP_EXT.iter().any(|t| ext.eq_ignore_ascii_case(t)),
        None => false,
    }
}

fn snapshot(dir: &PathBuf) -> HashMap<PathBuf, u64> {
    let mut out = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_file() && !is_temp(&path) {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                out.insert(path, size);
            }
        }
    }
    out
}

fn fmt_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

pub fn spawn() {
    std::thread::spawn(|| {
        crate::config::load();
        let mut conf_mtime = std::fs::metadata(crate::config::path())
            .and_then(|m| m.modified())
            .ok();

        let dir = downloads_dir();
        let mut known = dir.as_ref().map(|d| snapshot(d)).unwrap_or_default();
        // Files seen once but still growing; announced when the size holds.
        let mut settling: HashMap<PathBuf, u64> = HashMap::new();

        crate::todo::refresh();
        let mut pass: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            pass += 1;

            crate::todo::refresh();

            // Icon herding: shortly after start, then every five minutes.
            if pass == 4 || pass % 150 == 0 {
                if let Some(moved) = crate::icons::herd() {
                    if moved > 0 {
                        events::post(
                            events::Kind::Ok,
                            "icons herded",
                            &format!("{} moved off the wall's monitor", moved),
                        );
                    }
                }
            }

            // Config reload on mtime change.
            if let Ok(meta) = std::fs::metadata(crate::config::path()) {
                let m = meta.modified().ok();
                if m != conf_mtime {
                    conf_mtime = m;
                    crate::config::load();
                    events::journal(events::Kind::Info, "dcc.conf reloaded");
                }
            }

            let Some(dir) = dir.as_ref() else { continue };
            let now = snapshot(dir);
            for (path, size) in &now {
                if known.contains_key(path) {
                    continue;
                }
                match settling.get(path) {
                    Some(prev) if prev == size => {
                        // Stable across two passes — call it done.
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        events::post_action(
                            events::Kind::Ok,
                            "download",
                            &format!("{} · {}", name, fmt_size(*size)),
                            &path.to_string_lossy(),
                        );
                        known.insert(path.clone(), *size);
                        settling.remove(path);
                    }
                    _ => {
                        settling.insert(path.clone(), *size);
                    }
                }
            }
            // Forget deletions so a re-download announces again.
            known.retain(|p, _| now.contains_key(p));
            settling.retain(|p, _| now.contains_key(p));
        }
    });
}
