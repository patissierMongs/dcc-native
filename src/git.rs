//! Local git activity: repo states, a recent-commit feed, and the real
//! contribution heatmap.
//!
//! This is the one collector that shells out — hand-parsing packfiles to
//! count ahead/behind would be a project of its own, and `git` is already on
//! any machine with repos worth watching. Two hidden-window calls per repo
//! every five minutes is not a subprocess habit, it's a survey.

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::state::{Commit, Repo};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Default)]
pub struct GitData {
    pub repos: Vec<Repo>,
    pub commits: Vec<Commit>,
    pub grass: Vec<u8>,
    pub scanned: usize,
}

static OUT: Mutex<Option<GitData>> = Mutex::new(None);

pub fn current() -> Option<GitData> {
    OUT.lock().ok().and_then(|o| o.clone())
}

fn run_git(repo: &PathBuf, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Directories containing .git, up to two levels below each configured root.
fn discover(roots: &[String]) -> Vec<PathBuf> {
    fn has_git(p: &PathBuf) -> bool {
        p.join(".git").exists()
    }
    let mut out = Vec::new();
    for root in roots {
        let root = PathBuf::from(root.replace("%USERPROFILE%", &std::env::var("USERPROFILE").unwrap_or_default()));
        if !root.is_dir() {
            continue;
        }
        if has_git(&root) {
            out.push(root.clone());
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                let p = e.path();
                if p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
                    continue;
                }
                if has_git(&p) {
                    out.push(p);
                } else if let Ok(rd2) = std::fs::read_dir(&p) {
                    for e2 in rd2.flatten().filter(|e| e.path().is_dir()) {
                        let p2 = e2.path();
                        if has_git(&p2) {
                            out.push(p2);
                        }
                    }
                }
            }
        }
        if out.len() >= 24 {
            break;
        }
    }
    out.truncate(24);
    out
}

fn today_serial(off_min: i64) -> i64 {
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    (epoch + off_min * 60).div_euclid(86400)
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

fn scan() -> GitData {
    let roots = crate::config::git_roots();
    let repos = discover(&roots);
    let off_min = utc_offset_minutes();
    let now_epoch = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let today = today_serial(off_min);
    let dow_today = (today + 3).rem_euclid(7); // Monday = 0
    let this_monday = today - dow_today;

    let mut out = GitData { scanned: repos.len(), ..Default::default() };
    let mut day_counts: HashMap<i64, u32> = HashMap::new();
    let mut all_commits: Vec<(u64, Commit)> = Vec::new();

    for path in &repos {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        // Branch, ahead/behind and dirty count in one porcelain call.
        let mut branch = String::new();
        let mut ahead = 0usize;
        let mut dirty = 0usize;
        if let Some(status) = run_git(path, &["status", "--porcelain=v2", "--branch"]) {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("# branch.head ") {
                    branch = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
                    ahead = rest
                        .split_whitespace()
                        .find_map(|t| t.strip_prefix('+'))
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0);
                } else if !line.starts_with('#') && !line.is_empty() {
                    dirty += 1;
                }
            }
        } else {
            continue;
        }

        // One log call covers grass, commits-today, and the feed.
        let mut commits_today = 0usize;
        if let Some(log) = run_git(
            path,
            &["log", "--since=26.weeks", "--pretty=%h%x09%ct%x09%s", "-n", "600"],
        ) {
            for line in log.lines() {
                let mut parts = line.splitn(3, '\t');
                let (Some(hash), Some(ct), msg) = (parts.next(), parts.next(), parts.next().unwrap_or("")) else {
                    continue;
                };
                let Ok(ct) = ct.parse::<i64>() else { continue };
                let serial = (ct + off_min * 60).div_euclid(86400);
                *day_counts.entry(serial).or_insert(0) += 1;
                if serial == today {
                    commits_today += 1;
                }
                all_commits.push((
                    ct as u64,
                    Commit {
                        repo: name.clone(),
                        hash: hash.to_string(),
                        secs_ago: now_epoch.saturating_sub(ct as u64),
                        msg: msg.to_string(),
                    },
                ));
            }
        }

        out.repos.push(Repo { name, branch, dirty, ahead, commits_today });
    }

    // Feed: freshest across every repo.
    all_commits.sort_by(|a, b| b.0.cmp(&a.0));
    out.commits = all_commits.into_iter().map(|(_, c)| c).take(6).collect();

    // Repos with activity first, then dirty, then name.
    out.repos.sort_by(|a, b| {
        b.commits_today
            .cmp(&a.commits_today)
            .then(b.dirty.cmp(&a.dirty))
            .then(a.name.cmp(&b.name))
    });

    // Grass: 26 weeks × 7 days, Monday-topped columns, oldest week first.
    let mut grass = vec![0u8; 26 * 7];
    for (serial, count) in day_counts {
        let dow = (serial + 3).rem_euclid(7);
        let monday = serial - dow;
        let weeks_back = (this_monday - monday) / 7;
        if !(0..26).contains(&weeks_back) {
            continue;
        }
        let wk = (25 - weeks_back) as usize;
        let lvl = match count {
            0 => 0,
            1 => 1,
            2..=3 => 2,
            4..=6 => 3,
            _ => 4,
        };
        grass[wk * 7 + dow as usize] = lvl;
    }
    out.grass = grass;
    out
}

pub fn spawn() {
    std::thread::spawn(|| {
        let mut announced = false;
        loop {
            let data = scan();
            if !announced && data.scanned > 0 {
                announced = true;
                crate::events::journal(
                    crate::events::Kind::Ok,
                    &format!("git: {} repos scanned", data.scanned),
                );
            }
            if let Ok(mut guard) = OUT.lock() {
                *guard = Some(data);
            }
            std::thread::sleep(Duration::from_secs(300));
        }
    });
}
