//! Making room underneath the desktop icons.
//!
//! On this generation of the shell the desktop wallpaper is painted by
//! `SysListView32` — the icon list itself — which fills the whole virtual
//! desktop. Anything parented below it is therefore never seen, and anything
//! above it covers the icons. Neither is a wallpaper.
//!
//! The way out is to tell that list view to stop painting a background at all
//! (`LVM_SETBKCOLOR` with `CLR_NONE`), which is the same switch the old
//! "transparent desktop icons" tweaks used. The icons and their labels keep
//! drawing; the background becomes whatever sits behind — us.
//!
//! This changes shell state, so it is reversible: `restore()` puts the colour
//! back, and an Explorer restart clears it regardless.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_GETBKCOLOR: u32 = 0x1000;
const CLR_NONE: isize = 0xFFFF_FFFFu32 as isize;

pub fn find_list_view() -> Option<HWND> {
    unsafe {
        let pm = FindWindowW(w!("Progman"), None).ok()?;
        let dv = FindWindowExW(Some(pm), None, w!("SHELLDLL_DefView"), None).ok()?;
        FindWindowExW(Some(dv), None, w!("SysListView32"), None).ok()
    }
}

/// Returns the colour the list view had before, so it can be put back.
pub fn previous_colour() -> isize {
    match find_list_view() {
        Some(lv) => unsafe { SendMessageW(lv, LVM_GETBKCOLOR, None, None).0 },
        None => 0,
    }
}

fn apply(colour: isize) -> bool {
    let Some(lv) = find_list_view() else { return false };
    unsafe {
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(colour)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(colour)));
        let _ = InvalidateRect(Some(lv), None, true);
        let _ = UpdateWindow(lv);
    }
    true
}

/// Stop the icon list painting its background.
pub fn make_transparent() -> bool {
    apply(CLR_NONE)
}

/// Put the background back the way the shell had it.
pub fn restore(colour: isize) -> bool {
    apply(colour)
}
