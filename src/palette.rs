#![allow(static_mut_refs)]
//! The command palette — the one place keyboard input exists.
//!
//! The wallpaper itself never takes focus; Ctrl+` summons a small top-level
//! popup that does, and losing focus (or Esc) dismisses it instantly. One
//! input line, a result list:
//!
//!   - launcher entries filter as you type
//!   - `+text` appends to TODO.md
//!   - `http…` opens as a URL
//!   - anything else is also sent to Everything (if its window exists) for
//!     instant file search, replies arriving over WM_COPYDATA
//!
//! The palette runs on the main thread: the hotkey lands in the main
//! wndproc, every message here re-renders through its own small grid +
//! renderer pair.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::grid::Grid;
use crate::theme::Theme;

const COLS: usize = 58;
const ROWS: usize = 12;
const MAX_RESULTS: usize = 8;
/// Everything's reply lands as this WM_COPYDATA dwData tag.
const EV_REPLY: usize = 0xDCC0;

#[derive(Clone)]
enum Action {
    Open(String),
    AddTodo(String),
}

#[derive(Clone)]
struct Item {
    label: String,
    hint: String,
    action: Action,
}

struct State {
    query: String,
    items: Vec<Item>,
    sel: usize,
    dirty: bool,
    everything_seen: bool,
}

static OPEN: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State {
    query: String::new(),
    items: Vec::new(),
    sel: 0,
    dirty: true,
    everything_seen: false,
});
static mut HWND_PAL: HWND = HWND(std::ptr::null_mut());
static mut RENDERER: Option<crate::gfx::Renderer> = None;

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

fn rebuild_results() {
    let Ok(mut st) = STATE.lock() else { return };
    let q = st.query.clone();
    let ql = q.to_lowercase();
    let mut items = Vec::new();

    if let Some(text) = q.strip_prefix('+') {
        if !text.trim().is_empty() {
            items.push(Item {
                label: format!("todo 추가: {}", text.trim()),
                hint: "TODO.md".into(),
                action: Action::AddTodo(text.trim().to_string()),
            });
        }
    } else {
        if q.starts_with("http://") || q.starts_with("https://") {
            items.push(Item { label: q.clone(), hint: "open url".into(), action: Action::Open(q.clone()) });
        }
        for it in crate::config::launcher() {
            if ql.is_empty() || it.label.to_lowercase().contains(&ql) {
                items.push(Item {
                    label: it.label.clone(),
                    hint: it.target.clone(),
                    action: Action::Open(it.target.clone()),
                });
            }
        }
    }
    items.truncate(MAX_RESULTS);
    st.items = items;
    st.sel = 0;
    st.dirty = true;
}

/// Fire a query at Everything's IPC window; the answer comes back to us as
/// WM_COPYDATA with dwData == EV_REPLY.
fn query_everything(hwnd: HWND, q: &str) {
    if q.len() < 2 || q.starts_with('+') {
        return;
    }
    unsafe {
        let Ok(ev) = FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), PCWSTR::null()) else {
            if let Ok(mut st) = STATE.lock() {
                st.everything_seen = false;
            }
            return;
        };
        if let Ok(mut st) = STATE.lock() {
            st.everything_seen = true;
        }
        // EVERYTHING_IPC_QUERYW: reply_hwnd, reply_msg, flags, offset, max, search[]
        let search: Vec<u16> = q.encode_utf16().chain(std::iter::once(0)).collect();
        let head = [
            hwnd.0 as u32, // reply hwnd (32-bit hwnds are fine)
            EV_REPLY as u32,
            0u32, // search flags
            0u32, // offset
            MAX_RESULTS as u32,
        ];
        let mut buf: Vec<u8> = Vec::with_capacity(20 + search.len() * 2);
        for v in head {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(std::slice::from_raw_parts(search.as_ptr() as *const u8, search.len() * 2));
        let cds = COPYDATASTRUCT {
            dwData: 2, // EVERYTHING_IPC_COPYDATAQUERYW
            cbData: buf.len() as u32,
            lpData: buf.as_ptr() as *mut _,
        };
        let _ = SendMessageW(ev, WM_COPYDATA, Some(WPARAM(hwnd.0 as usize)), Some(LPARAM(&cds as *const _ as isize)));
    }
}

/// Parse EVERYTHING_IPC_LISTW out of the reply block and append the files.
unsafe fn on_everything_reply(cds: &COPYDATASTRUCT) {
    let base = cds.lpData as *const u8;
    let len = cds.cbData as usize;
    if len < 12 {
        return;
    }
    let u32_at = |off: usize| -> u32 {
        let mut b = [0u8; 4];
        std::ptr::copy_nonoverlapping(base.add(off), b.as_mut_ptr(), 4);
        u32::from_le_bytes(b)
    };
    let num = u32_at(4) as usize;
    let mut files = Vec::new();
    for i in 0..num.min(MAX_RESULTS) {
        let item_off = 12 + i * 12;
        if item_off + 12 > len {
            break;
        }
        let name_off = u32_at(item_off + 4) as usize;
        let path_off = u32_at(item_off + 8) as usize;
        let read_wstr = |off: usize| -> String {
            if off >= len {
                return String::new();
            }
            let p = base.add(off) as *const u16;
            let mut n = 0usize;
            while off + n * 2 + 1 < len && *p.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        };
        let name = read_wstr(name_off);
        let dir = read_wstr(path_off);
        if !name.is_empty() {
            files.push((name, dir));
        }
    }
    if let Ok(mut st) = STATE.lock() {
        // Keep the non-file entries, replace previous file results.
        st.items.retain(|i| !i.hint.starts_with("file:"));
        for (name, dir) in files {
            if st.items.len() >= MAX_RESULTS {
                break;
            }
            let full = format!("{}\\{}", dir, name);
            st.items.push(Item {
                label: name,
                hint: format!("file:{}", dir),
                action: Action::Open(full),
            });
        }
        st.dirty = true;
    }
}

fn execute(hwnd: HWND) {
    let action = {
        let Ok(st) = STATE.lock() else { return };
        st.items.get(st.sel).map(|i| i.action.clone())
    };
    match action {
        Some(Action::Open(target)) => {
            crate::events::journal(crate::events::Kind::Info, &format!("palette: open {}", target));
            super::launch(&target, "");
        }
        Some(Action::AddTodo(text)) => crate::todo::add(&text),
        None => {}
    }
    hide(hwnd);
}

fn hide(hwnd: HWND) {
    OPEN.store(false, Ordering::Relaxed);
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_ACTIVATE => {
            if (w.0 & 0xFFFF) as u32 == WA_INACTIVE {
                hide(hwnd);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = w.0 as u32;
            match vk {
                0x1B => hide(hwnd), // esc
                0x26 => {
                    if let Ok(mut st) = STATE.lock() {
                        st.sel = st.sel.saturating_sub(1);
                        st.dirty = true;
                    }
                } // up
                0x28 => {
                    if let Ok(mut st) = STATE.lock() {
                        if st.sel + 1 < st.items.len() {
                            st.sel += 1;
                        }
                        st.dirty = true;
                    }
                } // down
                0x0D => execute(hwnd), // enter
                _ => {}
            }
            LRESULT(0)
        }
        WM_CHAR => {
            let c = char::from_u32(w.0 as u32).unwrap_or('\0');
            let mut requery = false;
            if c == '\u{8}' {
                if let Ok(mut st) = STATE.lock() {
                    st.query.pop();
                }
                requery = true;
            } else if c >= ' ' && c != '\u{7f}' {
                if let Ok(mut st) = STATE.lock() {
                    if st.query.chars().count() < 60 {
                        st.query.push(c);
                    }
                }
                requery = true;
            }
            if requery {
                rebuild_results();
                let q = STATE.lock().map(|s| s.query.clone()).unwrap_or_default();
                query_everything(hwnd, &q);
            }
            LRESULT(0)
        }
        WM_COPYDATA => {
            let cds = &*(l.0 as *const COPYDATASTRUCT);
            if cds.dwData == EV_REPLY {
                on_everything_reply(cds);
            }
            LRESULT(1)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

/// Create (once) and show the palette, focused, centred on the primary
/// monitor. Called from the main wndproc on WM_HOTKEY.
pub fn toggle(font: &str) {
    unsafe {
        if is_open() {
            hide(HWND_PAL);
            return;
        }
        if HWND_PAL.0.is_null() {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = w!("DccPalette");
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(std::ptr::null_mut()),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);

            // Sized from the cell metrics of a probe renderer on the main
            // window would be circular; just derive from the target cols.
            let list = crate::monitors::list();
            let m = list.iter().find(|m| m.primary).or(list.first());
            let (mx, my, mw, mh) = m
                .map(|m| (m.rect.left, m.rect.top, m.w(), m.h()))
                .unwrap_or((0, 0, 1920, 1080));
            let w_px = (mw / 4).clamp(700, 1200);
            let cell = w_px as f32 / COLS as f32;
            let h_px = (cell * 2.0 * ROWS as f32) as i32;
            let x = mx + (mw - w_px) / 2;
            let y = my + mh / 4;

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!("dcc palette"),
                WS_POPUP,
                x,
                y,
                w_px,
                h_px,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            HWND_PAL = hwnd;
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let cw = (rc.right - rc.left).max(64) as u32;
            let ch = (rc.bottom - rc.top).max(64) as u32;
            RENDERER = crate::gfx::Renderer::new(hwnd, (cw, ch), font, COLS as f32).ok();
        }
        if HWND_PAL.0.is_null() {
            return;
        }
        if let Ok(mut st) = STATE.lock() {
            st.query.clear();
            st.sel = 0;
            st.dirty = true;
        }
        rebuild_results();
        OPEN.store(true, Ordering::Relaxed);
        let _ = ShowWindow(HWND_PAL, SW_SHOW);
        let _ = SetForegroundWindow(HWND_PAL);
    }
}

/// Repaint when dirty — called every main-loop pass.
pub fn paint(t: &Theme) {
    if !is_open() {
        return;
    }
    unsafe {
        let Some(renderer) = RENDERER.as_mut() else { return };
        {
            let Ok(st) = STATE.lock() else { return };
            if !st.dirty {
                return;
            }
        }
        let (cols, rows) = renderer.cols_rows();
        let mut g = Grid::new(cols, rows, t.fg);
        {
            let Ok(mut st) = STATE.lock() else { return };
            g.panel(0, 0, cols, rows, "run · search · +todo", t.frame, t.ok);
            let prompt = format!("▸ {}", st.query);
            g.text(2, 1, &prompt, t.bright);
            g.put(2 + crate::grid::str_width(&prompt), 1, '▏', t.ok);
            g.hline(1, 2, cols - 2, '╌', t.frame);
            if st.items.is_empty() {
                let hint = if st.everything_seen || st.query.is_empty() {
                    "type to search · +text adds todo · esc closes"
                } else {
                    "everything not running — launcher + todo only"
                };
                g.text(2, 3, hint, t.frame);
            }
            for (i, item) in st.items.iter().enumerate() {
                let cy = 3 + i;
                if cy >= rows - 1 {
                    break;
                }
                let selc = i == st.sel;
                if selc {
                    g.put(1, cy, '▸', t.bright);
                }
                g.text_clip_end(3, cy, &item.label, cols * 2 / 3, if selc { t.bright } else { t.fg });
                let hint = item.hint.strip_prefix("file:").unwrap_or(&item.hint);
                g.text_right(cols - 2, cy, &crate::grid::clip_str(hint, cols / 3), t.frame);
            }
            st.dirty = false;
        }
        let _ = renderer.draw(&g, t.bg, (0.0, 0.0), 1.0);
    }
}
