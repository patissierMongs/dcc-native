//! Toasts and the journal — the single channel every subsystem reports
//! through.
//!
//! A toast is a transient overlay in the top-right corner; a journal entry is
//! the permanent record of the same event. `post` produces both, `journal`
//! only the record. Collectors on other threads may post too, so the bus
//! lives behind a Mutex; render passes take a cheap snapshot instead of
//! holding the lock while painting.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Ok,
    Warn,
    Err,
}

pub struct Toast {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    born: Instant,
}

#[derive(Clone)]
pub struct Entry {
    /// HH:MM:SS at the moment of the event.
    pub hms: String,
    pub kind: Kind,
    pub text: String,
    /// Something to ShellExecute when the row is clicked (a file, a folder).
    pub action: Option<String>,
}

pub struct Bus {
    toasts: Vec<Toast>,
    journal: VecDeque<Entry>,
}

static BUS: Mutex<Bus> = Mutex::new(Bus { toasts: Vec::new(), journal: VecDeque::new() });

const TOAST_LIFE: Duration = Duration::from_secs(8);
const TOAST_STACK: usize = 4;
const JOURNAL_CAP: usize = 400;

fn hms() -> String {
    let (_, _, _, h, m, s) = crate::collect::local_time();
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn push_entry(bus: &mut Bus, kind: Kind, text: String, action: Option<String>) {
    bus.journal.push_back(Entry { hms: hms(), kind, text, action });
    while bus.journal.len() > JOURNAL_CAP {
        bus.journal.pop_front();
    }
}

/// Record the event and raise a toast for it.
pub fn post(kind: Kind, title: &str, body: &str) {
    post_action_inner(kind, title, body, None);
}

/// Like `post`, but the journal entry opens `action` when clicked.
pub fn post_action(kind: Kind, title: &str, body: &str, action: &str) {
    post_action_inner(kind, title, body, Some(action.to_string()));
}

fn post_action_inner(kind: Kind, title: &str, body: &str, action: Option<String>) {
    let Ok(mut bus) = BUS.lock() else { return };
    let text = if body.is_empty() { title.to_string() } else { format!("{} — {}", title, body) };
    push_entry(&mut bus, kind, text, action);
    bus.toasts.push(Toast {
        kind,
        title: title.to_string(),
        body: body.to_string(),
        born: Instant::now(),
    });
    let excess = bus.toasts.len().saturating_sub(TOAST_STACK);
    if excess > 0 {
        bus.toasts.drain(0..excess);
    }
}

/// Record the event without raising a toast.
pub fn journal(kind: Kind, text: &str) {
    if let Ok(mut bus) = BUS.lock() {
        push_entry(&mut bus, kind, text.to_string(), None);
    }
}

/// The click action of the journal row drawn at `row` for a view that showed
/// `take` lines ending `back_off` before the tail — same slicing as
/// `journal_tail`, so what was clicked is what was on screen.
pub fn action_for(take: usize, back_off: usize, row: usize) -> Option<String> {
    let bus = BUS.lock().ok()?;
    let len = bus.journal.len();
    let end = len.saturating_sub(back_off);
    let start = end.saturating_sub(take);
    bus.journal.get(start + row).and_then(|e| e.action.clone())
}

/// Drop expired toasts. The render loop calls this every pass; the grid diff
/// takes care of repainting only when something actually disappeared.
pub fn expire() {
    if let Ok(mut bus) = BUS.lock() {
        bus.toasts.retain(|t| t.born.elapsed() < TOAST_LIFE);
    }
}

/// Live toasts, oldest first, as plain data for the painter.
pub fn toasts() -> Vec<(Kind, String, String)> {
    match BUS.lock() {
        Ok(bus) => bus.toasts.iter().map(|t| (t.kind, t.title.clone(), t.body.clone())).collect(),
        Err(_) => Vec::new(),
    }
}

/// The journal tail: up to `take` entries ending `back_off` lines before the
/// newest. Returns (entries oldest-first, total length).
pub fn journal_tail(take: usize, back_off: usize) -> (Vec<Entry>, usize) {
    match BUS.lock() {
        Ok(bus) => {
            let len = bus.journal.len();
            let end = len.saturating_sub(back_off);
            let start = end.saturating_sub(take);
            (bus.journal.range(start..end).cloned().collect(), len)
        }
        Err(_) => (Vec::new(), 0),
    }
}
