//! Monitor geometry, read DPI-aware so the numbers are physical pixels.

use windows::core::BOOL;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

/// MONITORINFOF_PRIMARY
const PRIMARY_FLAG: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub rect: RECT,
    pub primary: bool,
}

impl Monitor {
    pub fn w(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn h(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
}

unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let out = &mut *(data.0 as *mut Vec<Monitor>);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        out.push(Monitor {
            rect: mi.rcMonitor,
            primary: mi.dwFlags & PRIMARY_FLAG != 0,
        });
    }
    BOOL(1)
}

pub fn list() -> Vec<Monitor> {
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    // left-to-right, top-to-bottom so indices are stable and intuitive
    out.sort_by_key(|m| (m.rect.top, m.rect.left));
    out
}

/// Bounding box of every monitor — the coordinate space WorkerW/Progman uses.
pub fn virtual_rect() -> RECT {
    let all = list();
    if all.is_empty() {
        return RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    }
    RECT {
        left: all.iter().map(|m| m.rect.left).min().unwrap(),
        top: all.iter().map(|m| m.rect.top).min().unwrap(),
        right: all.iter().map(|m| m.rect.right).max().unwrap(),
        bottom: all.iter().map(|m| m.rect.bottom).max().unwrap(),
    }
}

pub fn describe() -> String {
    let v = virtual_rect();
    let mut s = format!(
        "virtual {}x{} at {},{}",
        v.right - v.left,
        v.bottom - v.top,
        v.left,
        v.top
    );
    for (i, m) in list().iter().enumerate() {
        s.push_str(&format!(
            " | #{} {}x{} at {},{}{}",
            i,
            m.w(),
            m.h(),
            m.rect.left,
            m.rect.top,
            if m.primary { " PRIMARY" } else { "" }
        ));
    }
    s
}
