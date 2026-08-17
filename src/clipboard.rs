//! Clipboard history: the last few text snippets, click to copy again.
//!
//! `AddClipboardFormatListener` delivers WM_CLIPBOARDUPDATE to the wallpaper
//! window whenever anything copies; the handler snapshots CF_UNICODETEXT into
//! a small ring. Re-copying from the panel sets the clipboard ourselves, which
//! fires the same notification — `SELF_SET` swallows exactly one update so the
//! ring does not reorder itself every time it is used.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;

pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;
const CF_UNICODETEXT: u32 = 13;
const KEEP: usize = 12;
/// Full texts above this are still re-copyable, just not stored verbatim.
const MAX_STORE: usize = 64 * 1024;

#[derive(Clone)]
pub struct Clip {
    /// Single-line, trimmed, for the panel.
    pub preview: String,
    /// The full text that goes back on the clipboard.
    pub full: String,
}

static ITEMS: Mutex<VecDeque<Clip>> = Mutex::new(VecDeque::new());
static SELF_SET: AtomicBool = AtomicBool::new(false);
/// Retries left for a clipboard update we have not managed to read yet.
/// Windows' own clipboard history (Win+V) and every other listener race us
/// for OpenClipboard the instant something is copied — losing that race must
/// mean "try again shortly", not "drop the entry".
static PENDING: AtomicU32 = AtomicU32::new(0);

pub fn register(hwnd: HWND) {
    unsafe {
        let _ = AddClipboardFormatListener(hwnd);
    }
}

fn preview_of(s: &str) -> String {
    let mut out = String::new();
    for part in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
        if out.chars().count() > 120 {
            break;
        }
    }
    out
}

enum Read {
    Text(String),
    /// Clipboard holds no text (files, image) — nothing for us, stop trying.
    NoText,
    /// Someone else has the clipboard open — try again.
    Busy,
}

fn read_clipboard_text(hwnd: HWND) -> Read {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return Read::NoText;
        }
        if OpenClipboard(Some(hwnd)).is_err() {
            return Read::Busy;
        }
        let text = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let hg = HGLOBAL(h.0 as *mut core::ffi::c_void);
            let p = GlobalLock(hg) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut len = 0usize;
            while *p.add(len) != 0 && len < MAX_STORE {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(hg);
            Some(s)
        })();
        let _ = CloseClipboard();
        match text {
            Some(s) => Read::Text(s),
            None => Read::NoText,
        }
    }
}

/// WM_CLIPBOARDUPDATE handler — just flags the work; the render loop polls.
pub fn on_update(_hwnd: HWND) {
    if SELF_SET.swap(false, Ordering::Relaxed) {
        return;
    }
    PENDING.store(10, Ordering::Relaxed);
}

/// Called every loop pass (~200ms). Retries until the clipboard frees up.
pub fn poll(hwnd: HWND) {
    let left = PENDING.load(Ordering::Relaxed);
    if left == 0 {
        return;
    }
    match read_clipboard_text(hwnd) {
        Read::Text(full) => {
            PENDING.store(0, Ordering::Relaxed);
            let preview = preview_of(&full);
            if preview.is_empty() {
                return;
            }
            if let Ok(mut items) = ITEMS.lock() {
                // A repeat of the newest entry is the same copy echoing.
                if items.front().map(|c| c.full == full).unwrap_or(false) {
                    return;
                }
                items.retain(|c| c.full != full);
                items.push_front(Clip { preview, full });
                while items.len() > KEEP {
                    items.pop_back();
                }
            }
        }
        Read::NoText => PENDING.store(0, Ordering::Relaxed),
        Read::Busy => {
            PENDING.store(left - 1, Ordering::Relaxed);
            if left == 1 {
                crate::events::journal(
                    crate::events::Kind::Warn,
                    "clip: clipboard stayed busy, entry skipped",
                );
            }
        }
    }
}

pub fn items() -> Vec<Clip> {
    ITEMS.lock().map(|i| i.iter().cloned().collect()).unwrap_or_default()
}

/// Put entry `idx` back on the clipboard. Returns its preview on success.
pub fn recopy(hwnd: HWND, idx: usize) -> Option<String> {
    let clip = ITEMS.lock().ok()?.get(idx).cloned()?;
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let done = (|| {
            EmptyClipboard().ok()?;
            let wide: Vec<u16> = clip.full.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = wide.len() * 2;
            let hg = GlobalAlloc(GMEM_MOVEABLE, bytes).ok()?;
            let p = GlobalLock(hg) as *mut u8;
            if p.is_null() {
                let _ = GlobalFree(Some(hg));
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, p, bytes);
            let _ = GlobalUnlock(hg);
            // Ownership moves to the system on success.
            SELF_SET.store(true, Ordering::Relaxed);
            if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hg.0))).is_err() {
                SELF_SET.store(false, Ordering::Relaxed);
                let _ = GlobalFree(Some(hg));
                return None;
            }
            Some(())
        })();
        let _ = CloseClipboard();
        done.map(|_| clip.preview)
    }
}
