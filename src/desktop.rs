//! Parenting a window into the desktop, behind the icons.
//!
//! Explorer keeps the wallpaper in a window of class `WorkerW`. That window
//! does not exist until Progman is nudged with the undocumented message
//! `0x052C`, which asks it to spawn a second desktop-drawing worker. After the
//! nudge there are two `WorkerW` siblings: one holding `SHELLDLL_DefView` (the
//! icon host) and one directly behind it — the wallpaper layer.
//!
//! In practice a machine can have a dozen `WorkerW` windows: leftovers from
//! previous Explorer sessions, per-monitor workers, and small helper windows
//! that have nothing to do with the wallpaper. Parenting into one of those
//! makes the child invisible, because a child window is clipped to its parent.
//! So every candidate is checked for being desktop-sized before it is used,
//! and the result is verified after the fact.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Layer {
    pub hwnd: HWND,
    pub note: String,
}

fn class_of(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 128];
        let n = GetClassNameW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn rect_of(hwnd: HWND) -> (i32, i32, i32, i32) {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        (r.left, r.top, r.right - r.left, r.bottom - r.top)
    }
}

/// Big enough to actually hold the wallpaper: at least half the virtual desktop
/// in each axis. This is what rules out the 136x38 leftovers.
fn desktop_sized(hwnd: HWND) -> bool {
    unsafe {
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let (_, _, w, h) = rect_of(hwnd);
        w * 2 >= vw && h * 2 >= vh
    }
}

#[derive(Default)]
struct Scan {
    /// WorkerW that follows the one hosting SHELLDLL_DefView — the textbook
    /// wallpaper layer.
    sibling: HWND,
    /// Any desktop-sized, visible WorkerW, as a second choice.
    any_worker: HWND,
    /// Every top-level WorkerW seen, for the log when nothing matches.
    seen: Vec<String>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let scan = &mut *(lparam.0 as *mut Scan);
    if class_of(hwnd) != "WorkerW" {
        return BOOL(1);
    }

    let (_, _, w, h) = rect_of(hwnd);
    if scan.seen.len() < 12 {
        scan.seen.push(format!("{}x{}{}", w, h, if IsWindowVisible(hwnd).as_bool() { "v" } else { "" }));
    }

    if scan.any_worker.0.is_null() && IsWindowVisible(hwnd).as_bool() && desktop_sized(hwnd) {
        scan.any_worker = hwnd;
    }

    // The icon host lives inside one WorkerW; the wallpaper layer is the next.
    if FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None).is_ok() {
        if let Ok(next) = FindWindowExW(None, Some(hwnd), w!("WorkerW"), None) {
            if desktop_sized(next) {
                scan.sibling = next;
                return BOOL(0);
            }
        }
    }
    BOOL(1)
}

/// Nudge Progman if it is reachable, then pick the best wallpaper layer.
/// A missing Progman is not fatal: another wallpaper app may have taken the
/// desktop over, and the WorkerW scan still runs.
/// Which surface to become a child of.
#[derive(Clone, Copy, PartialEq)]
pub enum Strategy {
    /// Classic: the WorkerW that Explorer keeps behind the icon host.
    WorkerW,
    /// Newer shells paint the wallpaper in SHELLDLL_DefView itself, so the
    /// WorkerW behind it is never composited. Going *inside* DefView and
    /// sitting underneath SysListView32 puts us above the wallpaper and below
    /// the icons — which is what "behind the icons" actually means here.
    DefView,
}

pub fn find_defview() -> Option<Layer> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok()?;
        let dv = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None).ok()?;
        if !desktop_sized(dv) {
            return None;
        }
        let (x, y, w, h) = rect_of(dv);
        Some(Layer {
            hwnd: dv,
            note: format!("SHELLDLL_DefView [{:?} {}x{} at {},{}]", dv.0, w, h, x, y),
        })
    }
}

pub fn find_wallpaper_layer() -> Option<Layer> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok();
        if let Some(pm) = progman {
            // Two known spellings of the same nudge. The classic (0, 0) form
            // works up to Windows 10; recent Windows 11 builds only spawn the
            // second worker for (0x0D, 0x01). Sending both is harmless.
            let mut ignored: usize = 0;
            for (wp, lp) in [(0usize, 0isize), (0x0D, 0x01)] {
                let _ = SendMessageTimeoutW(
                    pm, 0x052C, WPARAM(wp), LPARAM(lp), SMTO_NORMAL, 2000, Some(&mut ignored),
                );
            }
        }

        // On current Windows 10/11 the worker that paints the wallpaper is a
        // *child* of Progman rather than a top-level window, so scanning only
        // top-level windows finds nothing usable. Check here first.
        // (Approach cross-checked against Lively Wallpaper, which is GPL-3.0 —
        // the call sequence is reproduced from its behaviour, not its code.)
        if let Some(pm) = progman {
            let mut prev: Option<HWND> = None;
            for _ in 0..8 {
                match FindWindowExW(Some(pm), prev, w!("WorkerW"), None) {
                    Ok(w) if !w.0.is_null() => {
                        if desktop_sized(w) {
                            let (x, y, ww, wh) = rect_of(w);
                            return Some(Layer {
                                hwnd: w,
                                note: format!("WorkerW under Progman [{:?} {}x{} at {},{}]", w.0, ww, wh, x, y),
                            });
                        }
                        prev = Some(w);
                    }
                    _ => break,
                }
            }
        }

        let mut scan = Scan::default();
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut scan as *mut _ as isize));

        let (hwnd, how) = if !scan.sibling.0.is_null() {
            (scan.sibling, "WorkerW after SHELLDLL_DefView")
        } else if !scan.any_worker.0.is_null() {
            (scan.any_worker, "first desktop-sized WorkerW")
        } else if let Some(pm) = progman.filter(|p| desktop_sized(*p)) {
            (pm, "Progman itself")
        } else {
            return None;
        };

        let (x, y, w, h) = rect_of(hwnd);
        Some(Layer {
            hwnd,
            note: format!(
                "{} [{:?} {}x{} at {},{}] workers seen: {}",
                how,
                hwnd.0,
                w,
                h,
                x,
                y,
                if scan.seen.is_empty() { "none".to_string() } else { scan.seen.join(",") }
            ),
        })
    }
}

/// Attach `hwnd` to the wallpaper layer, then verify it really took: the
/// parent must be what we asked for and must still be desktop-sized. Anything
/// less and the window is put back at top level for the caller to handle as a
/// plain backdrop.
pub fn attach(hwnd: HWND, under_icons: bool, how: Strategy) -> (bool, String) {
    let found = match how {
        Strategy::DefView => find_defview(),
        Strategy::WorkerW => find_wallpaper_layer(),
    };
    let Some(layer) = found else {
        return (false, "no usable desktop layer found".into());
    };

    unsafe {
        // SetParent on a WS_POPUP window makes it *owned*, not a child: it stays
        // top level, keeps competing for z-order with real applications, and
        // gets buried. Switching to WS_CHILD first is what actually welds it
        // into the desktop.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let child_style = (style & !WS_POPUP.0) | WS_CHILD.0 | WS_VISIBLE.0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, child_style as isize);

        let _ = SetParent(hwnd, Some(layer.hwnd));

        match GetParent(hwnd) {
            Ok(p) if p == layer.hwnd && desktop_sized(p) => {
                // Where to sit among the parent's children. Behind
                // SHELLDLL_DefView keeps the desktop icons on top, which is the
                // nicer result — but only works when that window leaves its
                // background transparent. On shells that paint the wallpaper
                // there instead, being behind it means being invisible, so the
                // default is to sit in front and own the surface outright.
                // Default is to sit above the icon list. On this generation of
                // the shell the list view composites the wallpaper itself, so
                // anything beneath it is never seen — above is the only place
                // that renders. `--under-icons` picks the other order for
                // shells where the layer beneath is real.
                let z = if under_icons { HWND_BOTTOM } else { HWND_TOP };
                let _ = SetWindowPos(hwnd, Some(z), 0, 0, 0, 0,
                                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                (true, layer.note)
            }
            _ => {
                let _ = SetParent(hwnd, None);
                SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
                (false, format!("parent did not stick: {}", layer.note))
            }
        }
    }
}

/// True when the window is no longer parented to a usable wallpaper layer.
pub fn parent_lost(hwnd: HWND) -> bool {
    unsafe {
        match GetParent(hwnd) {
            Ok(p) => !IsWindow(Some(p)).as_bool() || !desktop_sized(p),
            Err(_) => true,
        }
    }
}
