//! Mouse input queue, hit regions, and interactive UI state.
//!
//! wndproc runs on the same thread as the render loop, but re-entrancy makes a
//! queue the honest structure: message handlers push raw pixel events, the
//! loop drains them, converts pixels to grid cells, and applies them against
//! the hit regions the previous frame registered. One frame of latency at the
//! loop's cadence is imperceptible; sharing mutable layout state with wndproc
//! is not worth the aliasing risk.

use std::sync::Mutex;

/// Raw pixel-space events, exactly as the window messages delivered them.
#[derive(Clone, Copy, Debug)]
pub enum Raw {
    Move { x: i32, y: i32 },
    Leave,
    LeftDown { x: i32, y: i32 },
    RightDown { x: i32, y: i32 },
    /// `delta` is the wheel detent count: positive away from the user.
    Wheel { x: i32, y: i32, delta: i32 },
}

static QUEUE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());

pub fn push(e: Raw) {
    if let Ok(mut q) = QUEUE.lock() {
        // A stuck loop must not turn the queue into a leak.
        if q.len() < 512 {
            q.push(e);
        }
    }
}

pub fn drain() -> Vec<Raw> {
    QUEUE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// What a panel painted somewhere on the grid and wants events for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionId {
    /// The by-cpu process rows — wheel scrolls the list.
    ProcList,
    /// The journal panel — wheel scrolls history, click opens the action.
    Journal,
    /// Launcher tiles — click runs the configured target.
    Launcher,
    /// Clipboard history rows — click puts the entry back on the clipboard.
    ClipList,
    /// Headline rows — click opens the article.
    News,
    /// The pomodoro: click toggles, right-click resets.
    Pomo,
    /// TODO.md rows — click toggles the checkbox.
    Todo,
}

#[derive(Clone, Copy)]
pub struct Region {
    pub id: RegionId,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    /// Upper bound for this region's scroll offset (len - visible rows).
    pub max_scroll: usize,
}

/// Rebuilt by every `ui::render` pass; events are matched against the regions
/// of the frame the user was actually looking at.
#[derive(Default)]
pub struct Regions(pub Vec<Region>);

impl Regions {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn add(&mut self, id: RegionId, x: usize, y: usize, w: usize, h: usize, max_scroll: usize) {
        self.0.push(Region { id, x, y, w, h, max_scroll });
    }

    pub fn hit(&self, cx: usize, cy: usize) -> Option<Region> {
        self.0
            .iter()
            .copied()
            .find(|r| cx >= r.x && cx < r.x + r.w && cy >= r.y && cy < r.y + r.h)
    }
}

/// Interactive state the panels read while painting.
#[derive(Default)]
pub struct UiState {
    /// Grid cell currently under the cursor, None when it left the window.
    pub hover: Option<(usize, usize)>,
    /// First visible index into the by-cpu process list.
    pub proc_scroll: usize,
    /// How many lines back from the journal tail the view is scrolled.
    pub journal_scroll: usize,
}

impl UiState {
    /// If the cursor is inside the given rect, the (col, row) relative to its
    /// top-left. Panels use this for row highlights.
    pub fn hovered_in(&self, x: usize, y: usize, w: usize, h: usize) -> Option<(usize, usize)> {
        let (cx, cy) = self.hover?;
        if cx >= x && cx < x + w && cy >= y && cy < y + h {
            Some((cx - x, cy - y))
        } else {
            None
        }
    }
}
