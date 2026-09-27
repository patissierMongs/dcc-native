//! Drop zones: the deny-cursor turned into a feature.
//!
//! The wallpaper registers itself as an OLE drop target, so dragging files
//! onto the desktop now means something: three zones in the drop panel —
//! copy to the Google Drive mirror, file into per-extension folders, or the
//! recycle bin. While a drag is in flight the panel highlights whichever
//! zone the cursor is over; dropping anywhere else defaults to the Drive
//! zone. The actual file work happens on a worker thread; a toast reports
//! the outcome either way.

#![allow(non_snake_case)]

use std::path::PathBuf;
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::events;

/// Cell-space rects of the three zones, refreshed by the panel each frame.
/// (x, y, w, h, zone) — zone: 0 drive, 1 sort, 2 trash.
static ZONES: Mutex<Vec<(usize, usize, usize, usize, u8)>> = Mutex::new(Vec::new());
/// Cursor cell while an OLE drag is over the window.
static DRAG: Mutex<Option<(usize, usize)>> = Mutex::new(None);
/// Cell metrics, updated when the display changes and the grid is refitted.
static CELL: Mutex<(f32, f32)> = Mutex::new((1.0, 1.0));

pub fn set_cell_size(w: f32, h: f32) {
    if let Ok(mut c) = CELL.lock() {
        *c = (w.max(1.0), h.max(1.0));
    }
}

pub fn set_zones(z: Vec<(usize, usize, usize, usize, u8)>) {
    if let Ok(mut g) = ZONES.lock() {
        *g = z;
    }
}

/// The zone currently under a drag, for panel highlighting.
pub fn hover_zone() -> Option<u8> {
    let cell = (*DRAG.lock().ok()?)?;
    zone_at(cell)
}

pub fn dragging() -> bool {
    DRAG.lock().map(|d| d.is_some()).unwrap_or(false)
}

fn zone_at((cx, cy): (usize, usize)) -> Option<u8> {
    let zones = ZONES.lock().ok()?;
    zones
        .iter()
        .find(|(x, y, w, h, _)| cx >= *x && cx < x + w && cy >= *y && cy < y + h)
        .map(|(_, _, _, _, z)| *z)
}

/// Where the "drive" zone copies to: `[drop] drive = …` in dcc.conf, else
/// the usual Google Drive for Desktop mirror locations.
fn drive_dir() -> Option<PathBuf> {
    if let Some(p) = crate::config::drive() {
        let p = PathBuf::from(p.replace("%USERPROFILE%", &std::env::var("USERPROFILE").unwrap_or_default()));
        let _ = std::fs::create_dir_all(&p);
        if p.is_dir() {
            return Some(p);
        }
    }
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from("G:\\내 드라이브"),
        PathBuf::from("G:\\My Drive"),
    ];
    if let Some(p) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(&p).join("Google Drive"));
        candidates.push(PathBuf::from(&p).join("내 드라이브"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

fn sort_dir(ext: &str) -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Documents").join("dcc-sorted"))
        .unwrap_or_else(|| "dcc-sorted".into());
    base.join(if ext.is_empty() { "etc" } else { ext })
}

fn unique_dest(dir: &PathBuf, name: &str) -> PathBuf {
    let mut dest = dir.join(name);
    let mut n = 1;
    while dest.exists() {
        dest = dir.join(format!("{}_{}", n, name));
        n += 1;
    }
    dest
}

/// Send to the recycle bin, not oblivion: SHFileOperation with ALLOWUNDO.
fn recycle(paths: &[PathBuf]) -> bool {
    use windows::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW};
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        list.extend(p.as_os_str().to_string_lossy().encode_utf16());
        list.push(0);
    }
    list.push(0); // double-null terminated
    unsafe {
        let mut op = SHFILEOPSTRUCTW {
            wFunc: FO_DELETE as u32,
            pFrom: PCWSTR(list.as_ptr()),
            fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_SILENT.0) as u16,
            ..Default::default()
        };
        SHFileOperationW(&mut op) == 0
    }
}

fn perform(zone: u8, files: Vec<PathBuf>) {
    std::thread::spawn(move || {
        let n = files.len();
        match zone {
            0 => {
                let Some(dir) = drive_dir() else {
                    events::post(events::Kind::Err, "drive not found", "install Google Drive for Desktop?");
                    return;
                };
                let mut ok = 0;
                for f in &files {
                    let name = f.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    if std::fs::copy(f, unique_dest(&dir, &name)).is_ok() {
                        ok += 1;
                    }
                }
                events::post(
                    if ok == n { events::Kind::Ok } else { events::Kind::Warn },
                    "→ drive",
                    &format!("{}/{} copied · sync will upload", ok, n),
                );
            }
            1 => {
                let mut ok = 0;
                for f in &files {
                    let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    let dir = sort_dir(&ext);
                    let _ = std::fs::create_dir_all(&dir);
                    let name = f.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    if std::fs::rename(f, unique_dest(&dir, &name)).is_ok() {
                        ok += 1;
                    }
                }
                events::post(
                    if ok == n { events::Kind::Ok } else { events::Kind::Warn },
                    "sorted",
                    &format!("{}/{} moved to Documents\\dcc-sorted", ok, n),
                );
            }
            _ => {
                let done = recycle(&files);
                events::post(
                    if done { events::Kind::Ok } else { events::Kind::Err },
                    "recycled",
                    &format!("{} file(s) → recycle bin (undoable)", n),
                );
            }
        }
    });
}

#[implement(IDropTarget)]
struct DropTarget {
    hwnd: HWND,
}

impl DropTarget {
    fn update(&self, pt: &POINTL) {
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        let (cw, ch) = CELL.lock().map(|c| *c).unwrap_or((1.0, 1.0));
        let cell = ((p.x.max(0) as f32 / cw) as usize, (p.y.max(0) as f32 / ch) as usize);
        if let Ok(mut d) = DRAG.lock() {
            *d = Some(cell);
        }
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        _pdataobj: Ref<IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        self.update(pt);
        unsafe {
            *pdweffect = DROPEFFECT_COPY;
        }
        Ok(())
    }

    fn DragOver(&self, _grfkeystate: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        self.update(pt);
        unsafe {
            *pdweffect = DROPEFFECT_COPY;
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        if let Ok(mut d) = DRAG.lock() {
            *d = None;
        }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Ref<IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        self.update(pt);
        let zone = DRAG.lock().ok().and_then(|d| *d).and_then(zone_at).unwrap_or(0);
        if let Ok(mut d) = DRAG.lock() {
            *d = None;
        }
        unsafe {
            *pdweffect = DROPEFFECT_COPY;
        }

        let Some(data) = pdataobj.as_ref() else { return Ok(()) };
        let fmt = FORMATETC {
            cfFormat: 15, // CF_HDROP
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        unsafe {
            let mut medium = data.GetData(&fmt)?;
            let hdrop = HDROP(medium.u.hGlobal.0);
            let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
            let mut files = Vec::new();
            for i in 0..count {
                let mut buf = [0u16; 1024];
                let n = DragQueryFileW(hdrop, i, Some(&mut buf));
                if n > 0 {
                    files.push(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])));
                }
            }
            ReleaseStgMedium(&mut medium);
            if !files.is_empty() {
                perform(zone, files);
            }
        }
        Ok(())
    }
}

/// OleInitialize + RegisterDragDrop; the target lives for the process.
pub fn register(hwnd: HWND, cell_w: f32, cell_h: f32) {
    set_cell_size(cell_w, cell_h);
    unsafe {
        let target: IDropTarget = DropTarget { hwnd }.into();
        if RegisterDragDrop(hwnd, &target).is_ok() {
            events::journal(events::Kind::Ok, "drop zones armed — drag files onto the wall");
            std::mem::forget(target);
        } else {
            events::journal(events::Kind::Warn, "drop: RegisterDragDrop failed");
        }
    }
}
