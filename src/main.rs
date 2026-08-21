// No console window: this is a wallpaper, not a command. Startup diagnostics
// go to dcc.log next to the executable.
#![windows_subsystem = "windows"]

mod bigtext;
mod claude;
mod clipboard;
mod collect;
mod config;
mod desktop;
mod drop;
mod eventlog;
mod events;
mod feeds;
mod gfx;
mod git;
mod gpu;
mod http;
mod grid;
mod iconlist;
mod input;
mod monitors;
mod net_etw;
mod palette;
mod ping;
mod pomo;
mod probe;
mod todo;
mod state;
mod theme;
mod ui;
mod watch;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, TrackMouseEvent, HOT_KEY_MODIFIERS, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use grid::Grid;
use input::Raw;
use state::*;

/// Columns to aim for regardless of panel size — a 5120px monitor and a 1920px
/// one then show the same layout at the same apparent density.
const TARGET_COLS: f32 = 260.0;
const FONT: &str = "Cascadia Mono";

/// Set while running as an unparented desktop backdrop: every attempt to move
/// the window in the z-order gets forced back to the bottom.
static PIN_TO_BOTTOM: AtomicBool = AtomicBool::new(false);

/// Whether a WM_MOUSELEAVE is currently armed via TrackMouseEvent.
static MOUSE_TRACKED: AtomicBool = AtomicBool::new(false);

/// Lives in Win32::UI::Controls in the bindings; not worth the extra feature.
const WM_MOUSELEAVE: u32 = 0x02A3;

fn client_xy(l: LPARAM) -> (i32, i32) {
    ((l.0 & 0xFFFF) as i16 as i32, ((l.0 >> 16) & 0xFFFF) as i16 as i32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        // Anything that raises the window (a click landing on the desktop, a
        // foreground change) is undone here rather than fought after the fact.
        WM_WINDOWPOSCHANGING if PIN_TO_BOTTOM.load(Ordering::Relaxed) => {
            let wp = &mut *(l.0 as *mut WINDOWPOS);
            wp.hwndInsertAfter = HWND_BOTTOM;
            wp.flags &= !SWP_NOZORDER;
            LRESULT(0)
        }
        // The whole surface is repainted every frame; letting GDI erase first
        // only buys a flash of background colour.
        WM_ERASEBKGND => LRESULT(1),

        // A wallpaper must never steal focus, even when clicked.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),

        WM_MOUSEMOVE => {
            let (x, y) = client_xy(l);
            input::push(Raw::Move { x, y });
            // Arm a WM_MOUSELEAVE so hover state clears when the cursor goes.
            if !MOUSE_TRACKED.swap(true, Ordering::Relaxed) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            MOUSE_TRACKED.store(false, Ordering::Relaxed);
            input::push(Raw::Leave);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = client_xy(l);
            input::push(Raw::LeftDown { x, y });
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let (x, y) = client_xy(l);
            input::push(Raw::RightDown { x, y });
            LRESULT(0)
        }
        _ if msg == clipboard::WM_CLIPBOARDUPDATE => {
            clipboard::on_update(hwnd);
            LRESULT(0)
        }
        WM_HOTKEY => {
            palette::toggle(FONT);
            LRESULT(0)
        }
        // Wheel coordinates are in screen space, unlike every other mouse
        // message — convert before queueing.
        WM_MOUSEWHEEL => {
            let (sx, sy) = client_xy(l);
            let mut pt = POINT { x: sx, y: sy };
            let _ = ScreenToClient(hwnd, &mut pt);
            let delta = ((w.0 >> 16) & 0xFFFF) as u16 as i16 as i32 / 120;
            input::push(Raw::Wheel { x: pt.x, y: pt.y, delta });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

/// Target geometry in the desktop layer's coordinate space, whose origin is
/// the top-left corner of the whole virtual desktop — not the primary monitor.
///
/// `which` is None for "the primary monitor", Some(i) to pin to a specific
/// monitor, and the `--all` flag spans every monitor at once.
/// `as_child` picks the coordinate space. A child of the desktop layer is
/// placed relative to the virtual-desktop origin; a top-level window is placed
/// in screen coordinates. Using one for the other silently shifts the window
/// off the monitor by the height of whatever sits above it.
fn target_rect(which: Option<usize>, all: bool, as_child: bool) -> (i32, i32, i32, i32) {
    let v = monitors::virtual_rect();
    let (ox, oy) = if as_child { (v.left, v.top) } else { (0, 0) };

    if all {
        return (v.left - ox, v.top - oy, v.right - v.left, v.bottom - v.top);
    }
    let list = monitors::list();
    let m = which
        .and_then(|i| list.get(i).copied())
        .or_else(|| list.iter().copied().find(|m| m.primary))
        .or_else(|| list.first().copied());

    match m {
        Some(m) => (m.rect.left - ox, m.rect.top - oy, m.w(), m.h()),
        None => (0, 0, 1920, 1080),
    }
}

/// Diagnostics go to a file — there is no console to print to.
fn log_line(msg: &str) {
    use std::io::Write;
    let path = std::env::current_exe()
        .map(|p| p.with_file_name("dcc.log"))
        .unwrap_or_else(|_| "dcc.log".into());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let (_, mo, d, h, mi, s) = collect::local_time();
        let _ = writeln!(f, "{:02}-{:02} {:02}:{:02}:{:02}  {}", mo, d, h, mi, s, msg);
    }
}

/// The icon list's original background colour, saved before we touch it so
/// `--restore-icons` can put back what was actually there. Written once — a
/// second run must not overwrite the true original with our own CLR_NONE.
fn iconbg_path() -> std::path::PathBuf {
    std::env::current_exe()
        .map(|p| p.with_file_name("dcc-iconbg.txt"))
        .unwrap_or_else(|_| "dcc-iconbg.txt".into())
}

fn save_iconbg_once(colour: isize) {
    let p = iconbg_path();
    if !p.exists() {
        let _ = std::fs::write(p, colour.to_string());
    }
}

fn load_iconbg() -> Option<isize> {
    std::fs::read_to_string(iconbg_path()).ok()?.trim().parse().ok()
}

fn stamp_clock(s: &mut Snapshot) {
    let (y, mo, d, h, mi, se) = collect::local_time();
    s.clock_hm = format!("{:02}:{:02}", h, mi);
    s.clock_s = format!("{:02}", se);
    const MON: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
    s.date = format!("{:02} {} {}", d, MON[(mo as usize).clamp(1, 12) - 1], y);
}

/// Open a target the way Explorer would: exe, folder, or URL alike.
fn launch(target: &str, args: &str) {
    let target_w: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let args_w: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(target_w.as_ptr()),
            if args.is_empty() { PCWSTR::null() } else { PCWSTR(args_w.as_ptr()) },
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// Real collectors where they exist, placeholders where they don't. The git
/// side flips from demo to live as soon as the scanner has run once.
fn stub_side_panels(s: &mut Snapshot) {
    if let Some(git) = git::current() {
        s.demo = false;
        s.repos = git.repos;
        s.commits = git.commits;
        s.grass = git.grass;
        s.repos_scanned = git.scanned;
    } else if s.repos.is_empty() {
        s.demo = true;
        s.repos_scanned = 3;
        s.repos = vec![
            Repo { name: "dcc-native".into(), branch: "main".into(), dirty: 3, ahead: 1, commits_today: 5 },
            Repo { name: "scanops".into(), branch: "dev".into(), dirty: 0, ahead: 2, commits_today: 0 },
            Repo { name: "blog".into(), branch: "master".into(), dirty: 1, ahead: 0, commits_today: 1 },
        ];
        s.commits = vec![
            Commit { repo: "dcc-native".into(), hash: "a1b2c3d".into(), secs_ago: 540, msg: "feat: 전각 글리프 렌더링".into() },
            Commit { repo: "dcc-native".into(), hash: "e4f5a6b".into(), secs_ago: 4200, msg: "fix: clipboard retry queue".into() },
            Commit { repo: "blog".into(), hash: "c7d8e9f".into(), secs_ago: 86400 * 2, msg: "post: 벽지를 데스크톱으로".into() },
        ];
        // Deterministic fake grass — no RNG, so the frame diff stays quiet.
        s.grass = (0..26 * 7).map(|i| ((i * 2654435761usize) >> 7) as u8 % 5).collect();
    }
}

fn main() -> Result<()> {
    // `dcc --shot out.png [w h]` renders one frame offscreen and exits, using
    // the same painter as the live window — a faithful preview that does not
    // disturb whatever is on screen.
    let args: Vec<String> = std::env::args().collect();

    // Undo the icon-list tweak without starting the wallpaper.
    if args.iter().any(|a| a == "--restore-icons") {
        // CLR_NONE is the shell default on this generation; fall back to it
        // when no saved colour exists rather than painting the icons black.
        let colour = load_iconbg().unwrap_or(0xFFFF_FFFFu32 as isize);
        let ok = iconlist::restore(colour);
        log_line(&format!("icon list background restored to {:#x}: {}", colour, ok));
        return Ok(());
    }

    if args.iter().any(|a| a == "--probe") {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        log_line("---- probe before nudge ----");
        log_line(&probe::report(false));
        log_line("---- probe after nudge ----");
        log_line(&probe::report(true));
        return Ok(());
    }

    if args.get(1).map(|s| s.as_str()) == Some("--shot") {
        let path = args.get(2).cloned().unwrap_or_else(|| "shot.png".into());
        let w: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2560);
        let h: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1080);
        let th = theme::PHOSPHOR;

        // Several passes so the CPU deltas are real and the sparklines have
        // something in them.
        let mut c = collect::Collector::new();
        let mut snap = Snapshot::default();
        for _ in 0..14 {
            c.sample(&mut snap);
            std::thread::sleep(Duration::from_millis(120));
        }
        stamp_clock(&mut snap);
        stub_side_panels(&mut snap);
        snap.agent_uptime_secs = 1;

        // The preview should show the interactive chrome too, and prove the
        // full-width glyph path with real Hangul. Feeds are fetched inline so
        // the shot doubles as an end-to-end test of every fetcher.
        config::load();
        feeds::prefetch();
        events::journal(events::Kind::Ok, "dcc online — offscreen preview");
        events::journal(events::Kind::Info, "한글 렌더링 테스트 — 전각 2칸 정렬 확인");
        events::post(events::Kind::Ok, "preview", "toast + journal wired");

        let ui_state = input::UiState::default();
        let mut regions = input::Regions::default();
        return gfx::render_png(&path, (w, h), FONT, TARGET_COLS, th.bg, &mut |cols, rows| {
            let mut g = Grid::new(cols, rows, th.fg);
            ui::render(&mut g, &th, &snap, 7, &ui_state, &mut regions);
            g
        });
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // ShellExecute (launcher tiles, journal actions) wants an apartment.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        // How to get on screen:
        //   default     become a child of the desktop's own surface, underneath
        //               the icon list. A real wallpaper: icons keep working,
        //               Win+D cannot minimise a child window, and it never
        //               competes with application windows for z-order.
        //   --backdrop  fallback for shells that refuse the injection: a
        //               borderless sheet pinned to the bottom of the z-order.
        //   --window    an ordinary window, for looking at the layout.
        let windowed = args.iter().any(|a| a == "--window");
        let inject = !args.iter().any(|a| a == "--backdrop");
        // On this class of shell the WorkerW behind the icons is never
        // composited, so DefView is the default target for injection.
        let strategy = if args.iter().any(|a| a == "--workerw") {
            desktop::Strategy::WorkerW
        } else {
            desktop::Strategy::DefView
        };
        let span_all = args.iter().any(|a| a == "--all");
        let under_icons = args.iter().any(|a| a == "--under-icons");
        let pick = args
            .iter()
            .position(|a| a == "--monitor")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<usize>().ok());
        let cols_override = args
            .iter()
            .position(|a| a == "--cols")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<f32>().ok());

        // One wallpaper at a time. Two instances injected into the same
        // desktop layer paint over each other forever; refuse politely.
        // Windowed previews are exempt so the layout can be inspected while
        // the real one runs.
        let _single = if !windowed {
            let m = CreateMutexW(None, false, w!("dcc-wallpaper-instance"))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                log_line("another instance is already running — exiting");
                return Ok(());
            }
            Some(m)
        } else {
            None
        };

        let instance = GetModuleHandleW(None)?;
        let class_name = w!("DccWallpaperWindow");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(Error::from_thread());
        }

        log_line(&monitors::describe());

        // Created top level; the injected path re-places it after reparenting.
        let (mut x, mut y, mut w, mut h) = target_rect(pick, span_all, false);
        if windowed {
            w = (w as f32 * 0.55) as i32;
            h = (h as f32 * 0.55) as i32;
            x = 60;
            y = 60;
        }

        let hwnd = CreateWindowExW(
            if windowed { WINDOW_EX_STYLE(0) } else { WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW },
            class_name,
            w!("DevOps Command Center"),
            if windowed { WS_OVERLAPPEDWINDOW } else { WS_POPUP },
            x, y, w, h,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        if windowed {
            let _ = ShowWindow(hwnd, SW_SHOW);
            log_line("mode: windowed");
        } else {
            let attached = if inject {
                let (ok, note) = desktop::attach(hwnd, under_icons, strategy);
                log_line(&format!(
                    "mode: {} — {}",
                    if ok { "injected into wallpaper layer" } else { "injection failed, using backdrop" },
                    note
                ));
                if ok && under_icons {
                    // Only useful when we deliberately sit beneath the icons.
                    let before = iconlist::previous_colour();
                    save_iconbg_once(before);
                    let done = iconlist::make_transparent();
                    log_line(&format!("icon list transparent: {} (was {:#x})", done, before));
                }
                ok
            } else {
                log_line("mode: bottom-most full-screen backdrop");
                false
            };

            PIN_TO_BOTTOM.store(!attached, Ordering::Relaxed);
            let (px, py, pw, ph) = target_rect(pick, span_all, attached);
            // When injected, attach() already chose the z-order; do not undo it.
            if attached {
                let _ = SetWindowPos(hwnd, None, px, py, pw, ph, SWP_NOACTIVATE | SWP_NOZORDER);
            } else {
                let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), px, py, pw, ph, SWP_NOACTIVATE);
            }
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            log_line(&format!("placed at {},{} {}x{} ({})", px, py, pw, ph,
                              if attached { "layer coords" } else { "screen coords" }));
        }

        // The render target must match the client area, which is smaller than
        // the window in windowed mode.
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (cw, ch) = ((rc.right - rc.left).max(64) as u32, (rc.bottom - rc.top).max(64) as u32);

        // Roughly 19 physical pixels per cell at 100% scaling, divided by the
        // monitor's DPI scale so the wall's text tracks the system scale the
        // user chose — 125% scaling means 125% glyphs, like every other app.
        let dpi_scale = (GetDpiForWindow(hwnd) as f32 / 96.0).max(0.5);
        let cols = cols_override.unwrap_or_else(|| (cw as f32 / (19.0 * dpi_scale)).clamp(120.0, 320.0));

        // `--debug-bg` paints the surface magenta. If that colour is not on
        // screen, the problem is layering; if it is, the problem is drawing.
        let mut th = theme::PHOSPHOR;
        if args.iter().any(|a| a == "--debug-bg") {
            th.bg = 0xFF_00_FF;
        }
        let mut renderer = gfx::Renderer::new(hwnd, (cw, ch), FONT, cols)?;
        let (cols, rows) = renderer.cols_rows();
        log_line(&format!(
            "{}x{}px, {:.0}pt, cell {:.2}x{:.2} -> grid {}x{}",
            w, h, renderer.painter.pt, renderer.painter.cell_w, renderer.painter.cell_h, cols, rows
        ));
        let mut g = Grid::new(cols, rows, th.fg);

        events::post(events::Kind::Ok, "dcc online", &format!("grid {}x{} · {} cores", cols, rows, std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)));

        // Clipboard history feeds off WM_CLIPBOARDUPDATE; the watcher thread
        // covers Downloads and live dcc.conf reloads; the feed worker fetches
        // weather, ticker, headlines and the calendar.
        clipboard::register(hwnd);
        watch::spawn();
        feeds::spawn();
        ping::spawn();
        net_etw::spawn();
        claude::spawn();
        git::spawn();

        // Ctrl+` summons the palette — the only keyboard surface anywhere.
        // MOD_NOREPEAT | MOD_CONTROL, VK_OEM_3 (the backtick key).
        if RegisterHotKey(Some(hwnd), 1, HOT_KEY_MODIFIERS(0x4000 | 0x0002), 0xC0).is_ok() {
            events::journal(events::Kind::Info, "palette: Ctrl+` registered");
        } else {
            events::journal(events::Kind::Warn, "palette: hotkey Ctrl+` unavailable");
        }

        // Collectors run on their own thread; the render loop never blocks on
        // a system call, it just clones the latest snapshot.
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let mut c = collect::Collector::new();
                loop {
                    let mut snap = shared.lock().unwrap().clone();
                    c.sample(&mut snap);
                    stub_side_panels(&mut snap);
                    *shared.lock().unwrap() = snap;
                    std::thread::sleep(Duration::from_millis(1000));
                }
            });
        }

        let started = Instant::now();
        let mut frame: u64 = 0;
        let mut msg = MSG::default();
        let mut ui_state = input::UiState::default();
        let mut regions = input::Regions::default();
        let (mut cell_w, mut cell_h) = (renderer.painter.cell_w, renderer.painter.cell_h);
        let mut last_dpi = GetDpiForWindow(hwnd);

        // Drop zones need OLE proper (CoInitialize is not enough for
        // RegisterDragDrop), and the event-log subscriptions ride along.
        let _ = windows::Win32::System::Ole::OleInitialize(None);
        drop::register(hwnd, cell_w, cell_h);
        eventlog::start();

        loop {
            // Sleep until either the next tick or an input message: hover
            // feedback lands immediately instead of on the next 250ms beat.
            let _ = MsgWaitForMultipleObjects(None, false, 200, QS_ALLINPUT);

            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // Apply queued mouse events against the regions of the frame the
            // user was actually looking at.
            let cell_of = |px: i32, py: i32| -> (usize, usize) {
                (
                    (px.max(0) as f32 / cell_w) as usize,
                    (py.max(0) as f32 / cell_h) as usize,
                )
            };
            for e in input::drain() {
                match e {
                    Raw::Move { x, y } => ui_state.hover = Some(cell_of(x, y)),
                    Raw::Leave => ui_state.hover = None,
                    Raw::Wheel { x, y, delta } => {
                        let (cx, cy) = cell_of(x, y);
                        if let Some(r) = regions.hit(cx, cy) {
                            match r.id {
                                input::RegionId::ProcList => {
                                    // Wheel up shows earlier (hotter) rows.
                                    let step = (-delta * 3) as isize;
                                    let cur = ui_state.proc_scroll as isize;
                                    ui_state.proc_scroll =
                                        (cur + step).clamp(0, r.max_scroll as isize) as usize;
                                }
                                input::RegionId::Journal => {
                                    // Wheel up goes back through history.
                                    let step = (delta * 3) as isize;
                                    let cur = ui_state.journal_scroll as isize;
                                    ui_state.journal_scroll =
                                        (cur + step).clamp(0, r.max_scroll as isize) as usize;
                                }
                                // Launcher and clipboard fit their panels.
                                _ => {}
                            }
                        }
                    }
                    Raw::LeftDown { x, y } => {
                        let (cx, cy) = cell_of(x, y);
                        if let Some(r) = regions.hit(cx, cy) {
                            let row = cy - r.y;
                            match r.id {
                                input::RegionId::Launcher => {
                                    if let Some(it) = config::launcher().get(row) {
                                        launch(&it.target, &it.args);
                                        events::journal(
                                            events::Kind::Info,
                                            &format!("run {}", it.label),
                                        );
                                    }
                                }
                                input::RegionId::ClipList => {
                                    if let Some(p) = clipboard::recopy(hwnd, row) {
                                        let short: String = p.chars().take(40).collect();
                                        events::post(events::Kind::Ok, "copied", &short);
                                    }
                                }
                                input::RegionId::Journal => {
                                    if let Some(a) =
                                        events::action_for(r.h, ui_state.journal_scroll, row)
                                    {
                                        launch(&a, "");
                                    }
                                }
                                input::RegionId::News => {
                                    if let Some(hl) = feeds::get().news.get(row) {
                                        launch(&hl.url, "");
                                    }
                                }
                                input::RegionId::Pomo => pomo::toggle(),
                                input::RegionId::Todo => {
                                    let _ = todo::toggle(row);
                                }
                                input::RegionId::ProcList => {}
                            }
                        }
                    }
                    Raw::RightDown { x, y } => {
                        let (cx, cy) = cell_of(x, y);
                        if let Some(r) = regions.hit(cx, cy) {
                            if r.id == input::RegionId::Pomo {
                                pomo::reset();
                            }
                        }
                    }
                }
            }

            // "Show desktop" (Win+D) minimises every top-level window, and as a
            // backdrop we are one of them. Undo it: this is meant to *be* the
            // desktop, so there is nothing to reveal by hiding it.
            if PIN_TO_BOTTOM.load(Ordering::Relaxed) && IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let (x, y, w, h) = target_rect(pick, span_all, false);
                let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), x, y, w, h, SWP_NOACTIVATE);
            }

            // Explorer restarts and wallpaper apps both take the WorkerW away.
            // Retry every few seconds: promote to attached when the layer comes
            // back, drop to bottom-most backdrop when it does not.
            if inject && !windowed && frame % 20 == 0 && desktop::parent_lost(hwnd) {
                let (attached, note) = desktop::attach(hwnd, under_icons, strategy);
                if attached == PIN_TO_BOTTOM.load(Ordering::Relaxed) {
                    log_line(&format!(
                        "{} — {}",
                        if attached { "re-attached" } else { "dropped to backdrop" },
                        note
                    ));
                    events::post(
                        if attached { events::Kind::Ok } else { events::Kind::Warn },
                        if attached { "re-attached to desktop" } else { "dropped to backdrop" },
                        "",
                    );
                }
                PIN_TO_BOTTOM.store(!attached, Ordering::Relaxed);
                let (x, y, w, h) = target_rect(pick, span_all, attached);
                let flags = if attached { SWP_NOACTIVATE | SWP_NOZORDER } else { SWP_NOACTIVATE };
                let z = if attached { None } else { Some(HWND_BOTTOM) };
                let _ = SetWindowPos(hwnd, z, x, y, w, h, flags);
            }

            // Resolution, scaling and monitor-topology changes leave the
            // window sized (or densified) for a world that no longer exists.
            // A child window gets no WM_DISPLAYCHANGE broadcast, so poll the
            // geometry and DPI, and refit everything — position, render
            // target, font, grid — when either stops matching.
            if !windowed && frame % 20 == 5 {
                let attached = !PIN_TO_BOTTOM.load(Ordering::Relaxed);
                let (px, py, pw, ph) = target_rect(pick, span_all, attached);
                // Where the window should sit in *screen* coordinates — the
                // check that catches a transient monitor topology having
                // shoved us off-position while leaving the size intact.
                let (ex, ey, _, _) = target_rect(pick, span_all, false);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let mut wr = RECT::default();
                let _ = GetWindowRect(hwnd, &mut wr);
                let ndpi = GetDpiForWindow(hwnd);
                let size_ok = (rc.right - rc.left, rc.bottom - rc.top) == (pw, ph);
                let pos_ok = (wr.left, wr.top) == (ex, ey);
                if !size_ok || !pos_ok || ndpi != last_dpi {
                    let dpi_changed = ndpi != last_dpi;
                    last_dpi = ndpi;
                    let flags = if attached { SWP_NOACTIVATE | SWP_NOZORDER } else { SWP_NOACTIVATE };
                    let z = if attached { None } else { Some(HWND_BOTTOM) };
                    let _ = SetWindowPos(hwnd, z, px, py, pw, ph, flags);
                    if size_ok && !dpi_changed {
                        // Position-only drift: moved back, nothing to rebuild.
                        log_line(&format!("position drift fixed → {},{}", px, py));
                        frame += 1;
                        continue;
                    }
                    let _ = GetClientRect(hwnd, &mut rc);
                    let ncw = (rc.right - rc.left).max(64) as u32;
                    let nch = (rc.bottom - rc.top).max(64) as u32;
                    let nscale = (ndpi as f32 / 96.0).max(0.5);
                    let ncols = cols_override.unwrap_or_else(|| (ncw as f32 / (19.0 * nscale)).clamp(120.0, 320.0));
                    if let Ok(r) = gfx::Renderer::new(hwnd, (ncw, nch), FONT, ncols) {
                        renderer = r;
                        let (c2, r2) = renderer.cols_rows();
                        g = Grid::new(c2, r2, th.fg);
                        cell_w = renderer.painter.cell_w;
                        cell_h = renderer.painter.cell_h;
                        drop::set_cell_size(cell_w, cell_h);
                        ui_state = input::UiState::default();
                        log_line(&format!(
                            "display changed → {}x{}px, {:.0}pt, grid {}x{}",
                            ncw, nch, renderer.painter.pt, c2, r2
                        ));
                        events::post(
                            events::Kind::Ok,
                            "display changed",
                            &format!("re-fitted {}x{} · grid {}x{}", ncw, nch, c2, r2),
                        );
                    }
                }
            }

            let mut snap = shared.lock().unwrap().clone();
            stamp_clock(&mut snap);
            snap.agent_uptime_secs = started.elapsed().as_secs();

            events::expire();
            clipboard::poll(hwnd);
            pomo::tick();

            // Theme is re-read each pass so a dcc.conf edit swaps it live.
            th = if config::soft_theme() { theme::SOFT } else { theme::PHOSPHOR };
            if args.iter().any(|a| a == "--debug-bg") {
                th.bg = 0xFF_00_FF;
            }

            // OLED care: drift the whole render around an 8-position orbit
            // every 5 minutes, and dim everything during night hours.
            let (shift_on, night_dim, ns, ne) = config::display();
            let (_, _, _, hh, mm, _) = collect::local_time();
            let night = if ns <= ne { hh as u32 >= ns && (hh as u32) < ne } else { hh as u32 >= ns || (hh as u32) < ne };
            let dim = if night { night_dim as f32 } else { 1.0 };
            let offset = if shift_on {
                const POS: [(f32, f32); 8] =
                    [(0.0, 0.0), (2.0, 1.0), (3.0, -1.0), (1.0, 2.0), (-2.0, 2.0), (-3.0, 0.0), (-1.0, -2.0), (2.0, -2.0)];
                POS[((hh as usize * 60 + mm as usize) / 5) % 8]
            } else {
                (0.0, 0.0)
            };

            // Animations advance once a second, not once a frame, so the grid
            // is stable between clock ticks and most frames skip painting.
            ui::render(&mut g, &th, &snap, snap.agent_uptime_secs, &ui_state, &mut regions);
            renderer.draw_if_changed(&g, th.bg, offset, dim)?;
            palette::paint(&th);

            frame += 1;
        }
    }
}
