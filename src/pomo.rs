//! Pomodoro: the one tool on the wall that is pure interaction.
//!
//! Click starts or pauses, right-click resets, and the finish announces
//! itself through the same toast/journal channel as everything else. State is
//! a deadline, not a countdown loop — the render pass derives the remaining
//! time whenever it happens to run.

use std::sync::Mutex;
use std::time::{Duration, Instant};

const WORK: Duration = Duration::from_secs(25 * 60);

struct Pomo {
    /// Set while running.
    deadline: Option<Instant>,
    /// Time left while paused (or the full session when idle).
    remaining: Duration,
    /// A session was running and has hit zero, but the finish has not been
    /// announced yet.
    finished: bool,
}

static STATE: Mutex<Pomo> = Mutex::new(Pomo { deadline: None, remaining: WORK, finished: false });

pub fn toggle() {
    let Ok(mut p) = STATE.lock() else { return };
    match p.deadline.take() {
        Some(dl) => {
            // Pause: bank whatever is left.
            p.remaining = dl.saturating_duration_since(Instant::now());
        }
        None => {
            if p.remaining.is_zero() {
                p.remaining = WORK;
            }
            p.deadline = Some(Instant::now() + p.remaining);
            crate::events::journal(crate::events::Kind::Info, "pomodoro started");
        }
    }
}

pub fn reset() {
    if let Ok(mut p) = STATE.lock() {
        p.deadline = None;
        p.remaining = WORK;
        p.finished = false;
    }
}

/// Called once per render pass; fires the finish toast exactly once.
pub fn tick() {
    let Ok(mut p) = STATE.lock() else { return };
    if let Some(dl) = p.deadline {
        if Instant::now() >= dl {
            p.deadline = None;
            p.remaining = Duration::ZERO;
            p.finished = true;
        }
    }
    if p.finished {
        p.finished = false;
        drop(p);
        crate::events::post(crate::events::Kind::Ok, "pomodoro done", "25 min — take a break");
    }
}

/// Freeze a running session `frac` of the way through, for `--shot --demo`.
pub fn set_demo(frac: f64) {
    if let Ok(mut p) = STATE.lock() {
        let left = WORK.mul_f64((1.0 - frac).clamp(0.0, 1.0));
        p.deadline = Some(Instant::now() + left);
        p.remaining = left;
    }
}

/// (mm:ss, fraction elapsed, running) for the panel.
pub fn status() -> (String, f64, bool) {
    let Ok(p) = STATE.lock() else { return ("--:--".into(), 0.0, false) };
    let (rem, running) = match p.deadline {
        Some(dl) => (dl.saturating_duration_since(Instant::now()), true),
        None => (p.remaining, false),
    };
    let secs = rem.as_secs();
    let frac = 1.0 - rem.as_secs_f64() / WORK.as_secs_f64();
    (format!("{:02}:{:02}", secs / 60, secs % 60), frac.clamp(0.0, 1.0), running)
}
