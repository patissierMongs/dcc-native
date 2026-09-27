//! Where DCC keeps its files.
//!
//! Everything the user owns — settings, the diagnostic log, the day logs —
//! lives in one per-user folder rather than next to the executable, so a
//! `cargo clean` or a fresh download never takes the settings with it:
//!
//! ```text
//! %LOCALAPPDATA%\dcc\          dcc.conf, dcc.log, dcc-iconbg.txt
//! %LOCALAPPDATA%\dcc\logs\     dcc-day-YYYYMMDD.log
//! ```
//!
//! `DCC_HOME` overrides the location (a portable install on a USB stick, a
//! second profile for testing). Files that older builds left beside the
//! executable are carried over on first start by `migrate_legacy`.
//!
//! Folders come from SHGetKnownFolderPath rather than environment variables,
//! so relocated or redirected profile folders are honoured.
//!
//! One trap when testing: a process started from inside an MSIX-packaged app
//! (some terminals and launchers are) sees a virtualized AppData — its writes
//! land under `%LOCALAPPDATA%\Packages\<app>\LocalCache`, invisible to the
//! scheduled-task instance. Start DCC from Explorer or a regular shell.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::core::GUID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_Downloads, FOLDERID_LocalAppData, KF_FLAG_DEFAULT};

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

/// A shell known folder for the current user's token.
fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty()).map(PathBuf::from)
    }
}

/// The user's Downloads folder, wherever they moved it.
pub fn downloads() -> Option<PathBuf> {
    known_folder(&FOLDERID_Downloads).or_else(|| {
        std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Downloads"))
    })
}

/// The data folder, created on first use.
pub fn data_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::var_os("DCC_HOME")
            .map(PathBuf::from)
            .or_else(|| known_folder(&FOLDERID_LocalAppData).map(|p| p.join("dcc")))
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("dcc")))
            .or_else(exe_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

/// A file directly inside the data folder.
pub fn file(name: &str) -> PathBuf {
    data_dir().join(name)
}

/// Day logs get their own subfolder so the data folder stays readable.
pub fn logs_dir() -> PathBuf {
    let d = data_dir().join("logs");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Carry over what earlier builds kept next to the executable. Copies, never
/// deletes: the old `dcc.conf` is renamed to `dcc.conf.moved` so an edit to
/// the stale copy cannot silently go nowhere. Returns the names carried over.
pub fn migrate_legacy() -> Vec<String> {
    let Some(old) = exe_dir() else { return Vec::new() };
    let new = data_dir();
    if old == new {
        return Vec::new();
    }
    let mut moved = Vec::new();
    for name in ["dcc.conf", "dcc-iconbg.txt"] {
        let src = old.join(name);
        let dst = new.join(name);
        if src.is_file() && !dst.exists() && std::fs::copy(&src, &dst).is_ok() {
            if name == "dcc.conf" {
                let _ = std::fs::rename(&src, old.join("dcc.conf.moved"));
            }
            moved.push(name.to_string());
        }
    }
    if let Ok(rd) = std::fs::read_dir(&old) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("dcc-day-") && n.ends_with(".log") {
                let dst = logs_dir().join(&n);
                if !dst.exists() && std::fs::copy(e.path(), &dst).is_ok() {
                    moved.push(n);
                }
            }
        }
    }
    moved
}

/// Keep `dcc.log` from growing forever: past 2 MB it rolls over to
/// `dcc.log.1` at startup.
pub fn rotate_log() {
    let p = file("dcc.log");
    if std::fs::metadata(&p).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&p, file("dcc.log.1"));
    }
}
