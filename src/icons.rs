//! The icon herder: desktop icons don't get to live behind the wall.
//!
//! Windows lays new desktop icons out from the primary monitor's top-left —
//! which is exactly the monitor this wallpaper owns, so icons accumulate
//! invisibly underneath it. Every few minutes this sweeps the icon list:
//! anything positioned inside the wall's monitor is moved to the first free
//! grid slot on another monitor.
//!
//! Icon positions live inside Explorer's SysListView32, so reading them
//! means a classic cross-process dance: allocate a POINT in Explorer's
//! address space, ask the list to fill it, read it back. Moving uses
//! LVM_SETITEMPOSITION32 through the same remote buffer. Elevation makes
//! all of this permitted.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::Memory::{VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE};
use windows::Win32::UI::WindowsAndMessaging::*;

const LVM_GETITEMCOUNT: u32 = 0x1004;
const LVM_GETITEMPOSITION: u32 = 0x1010;
const LVM_SETITEMPOSITION32: u32 = 0x1031;
const LVM_GETITEMSPACING: u32 = 0x1033;
const LVS_AUTOARRANGE: isize = 0x0100;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Herding only makes sense when the wall owns one monitor of several.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Move every icon on the wall's monitor to another one. Returns how many
/// moved, or None when the machinery wasn't available.
pub fn herd() -> Option<usize> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let lv = crate::iconlist::find_list_view()?;
    unsafe {
        // Auto-arrange would snap everything straight back; respect it.
        if GetWindowLongPtrW(lv, GWL_STYLE) & LVS_AUTOARRANGE != 0 {
            return None;
        }

        let count = SendMessageW(lv, LVM_GETITEMCOUNT, None, None).0 as usize;
        if count == 0 || count > 512 {
            return Some(0);
        }

        // Geometry, in the list view's client space (origin = virtual
        // desktop top-left).
        let v = crate::monitors::virtual_rect();
        let mons = crate::monitors::list();
        let ours = mons.iter().copied().find(|m| m.primary).or_else(|| mons.first().copied())?;
        let others: Vec<_> = mons.iter().copied().filter(|m| m.rect != ours.rect).collect();
        if others.is_empty() {
            return Some(0);
        }
        let ban = RECT {
            left: ours.rect.left - v.left,
            top: ours.rect.top - v.top,
            right: ours.rect.right - v.left,
            bottom: ours.rect.bottom - v.top,
        };

        let spacing = SendMessageW(lv, LVM_GETITEMSPACING, Some(WPARAM(0)), None).0;
        let sx = ((spacing & 0xFFFF) as i32).max(48);
        let sy = (((spacing >> 16) & 0xFFFF) as i32).max(48);

        // Remote scratch POINT in Explorer's address space.
        let mut pid = 0u32;
        GetWindowThreadProcessId(lv, Some(&mut pid));
        let proc = OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE, false, pid).ok()?;
        let remote = VirtualAllocEx(proc, None, 16, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if remote.is_null() {
            let _ = CloseHandle(proc);
            return None;
        }

        let read_pos = |i: usize| -> Option<POINT> {
            if SendMessageW(lv, LVM_GETITEMPOSITION, Some(WPARAM(i)), Some(LPARAM(remote as isize))).0 == 0 {
                return None;
            }
            let mut p = POINT::default();
            ReadProcessMemory(proc, remote, &mut p as *mut _ as *mut _, std::mem::size_of::<POINT>(), None)
                .ok()?;
            Some(p)
        };

        let positions: Vec<Option<POINT>> = (0..count).map(read_pos).collect();

        // Occupancy grid of everything already outside the banned zone.
        let slot = |p: &POINT| (p.x / sx, p.y / sy);
        let mut occupied: std::collections::HashSet<(i32, i32)> = positions
            .iter()
            .flatten()
            .filter(|p| !(p.x >= ban.left && p.x < ban.right && p.y >= ban.top && p.y < ban.bottom))
            .map(|p| slot(p))
            .collect();

        // Free slots on the other monitors, column-major from each top-left.
        let mut free_slots: Vec<POINT> = Vec::new();
        for m in &others {
            let (mx, my) = (m.rect.left - v.left, m.rect.top - v.top);
            let cols = (m.w() / sx).max(1);
            let rows = (m.h() / sy).max(1);
            for c in 0..cols {
                for r in 0..rows {
                    let p = POINT { x: mx + c * sx + 8, y: my + r * sy + 8 };
                    if !occupied.contains(&slot(&p)) {
                        free_slots.push(p);
                    }
                }
            }
        }
        free_slots.reverse(); // pop() from the front of the scan order

        let mut moved = 0usize;
        for (i, pos) in positions.iter().enumerate() {
            let Some(p) = pos else { continue };
            let inside = p.x >= ban.left && p.x < ban.right && p.y >= ban.top && p.y < ban.bottom;
            if !inside {
                continue;
            }
            let Some(dest) = free_slots.pop() else { break };
            if WriteProcessMemory(proc, remote, &dest as *const _ as *const _, std::mem::size_of::<POINT>(), None)
                .is_ok()
            {
                SendMessageW(lv, LVM_SETITEMPOSITION32, Some(WPARAM(i)), Some(LPARAM(remote as isize)));
                occupied.insert(slot(&dest));
                moved += 1;
            }
        }

        let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
        let _ = CloseHandle(proc);
        Some(moved)
    }
}
