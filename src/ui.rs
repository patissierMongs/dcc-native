//! Layout. Everything here is integer cell arithmetic — no pixels, no CSS.
//!
//! Four zones, by what a glance is for:
//!   left        resources — what the machine has and how it is doing
//!   middle      tracking — who did what, when; the place to look when
//!               something goes wrong
//!   top right   the day — weather, money, time, calendar, headlines
//!   bottom right tools — the things you actually click

use crate::events;
use crate::grid::Grid;
use crate::input::{RegionId, Regions, UiState};
use crate::state::*;
use crate::theme::{pct_color, Theme};

const SEP: char = '▏';

pub fn render(g: &mut Grid, t: &Theme, s: &Snapshot, frame: u64, ui: &UiState, regions: &mut Regions) {
    g.clear();
    regions.clear();
    let (cols, rows) = (g.cols, g.rows);
    if cols < 100 || rows < 30 {
        g.text(0, 0, "terminal too small", t.crit);
        return;
    }

    header(g, t, s, frame);

    let top = 2;
    let body_h = rows - 3;

    let lw = 44.min(cols / 5);
    let rw = 64.min(cols / 3);
    let mw = cols - lw - rw;

    // ── left: resources ──────────────────────────────────────────────────
    let sys_h = 10;
    let gpu_h = 9;
    let mem_h = 7;
    let net_h = 9;
    let clk_h = 8;
    let vol_h = body_h.saturating_sub(sys_h + gpu_h + mem_h + net_h + clk_h);
    let mut ly = top;
    panel_sys(g, t, s, 0, ly, lw, sys_h);
    ly += sys_h;
    panel_gpu(g, t, s, 0, ly, lw, gpu_h);
    ly += gpu_h;
    panel_mem(g, t, s, 0, ly, lw, mem_h);
    ly += mem_h;
    panel_net(g, t, s, 0, ly, lw, net_h);
    ly += net_h;
    panel_disk(g, t, s, 0, ly, lw, vol_h);
    ly += vol_h;
    panel_clock(g, t, s, 0, ly, lw, clk_h);

    // ── middle: tracking. The hot table spans the full width; below it,
    // two sub-columns.
    let hot_h = 11;
    panel_hot(g, t, s, lw, top, mw, hot_h);

    let maw = mw / 2;
    let mbw = mw - maw;
    let ax = lw;
    let bx = lw + maw;
    let y2 = top + hot_h;
    let sub_h = body_h.saturating_sub(hot_h);

    let spikes_h = 14;
    let use_h = 14;
    let journal_h = sub_h.saturating_sub(spikes_h + use_h);
    panel_spikes(g, t, s, ax, y2, maw, spikes_h);
    panel_use(g, t, s, ax, y2 + spikes_h, maw, use_h);
    panel_journal(g, t, ax, y2 + spikes_h + use_h, maw, journal_h, ui, regions);

    let tl_h = 14;
    let git_h = 19;
    let claude_h = sub_h.saturating_sub(tl_h + git_h);
    panel_timeline(g, t, s, bx, y2, mbw, tl_h);
    panel_git(g, t, s, bx, y2 + tl_h, mbw, git_h);
    panel_claude(g, t, s, bx, y2 + tl_h + git_h, mbw, claude_h);

    // ── right: the day on top, tools below ───────────────────────────────
    let feeds = crate::feeds::get();
    let rx = lw + mw;
    let day_h = 10;
    let news_h = 9;
    panel_day(g, t, &feeds, rx, top, rw, day_h);
    panel_news(g, t, &feeds, rx, top + day_h, rw, news_h, ui, regions);

    let ty = top + day_h + news_h;
    let tools_h = body_h.saturating_sub(day_h + news_h);
    let rsl = rw / 2;
    let rsr = rw - rsl;
    // left tool stack: run / clip / drop
    let items = crate::config::launcher();
    let run_h = (items.len() + 2).clamp(3, 9);
    let drop_h = 8.min(tools_h.saturating_sub(run_h + 4));
    let clip_h = tools_h.saturating_sub(run_h + drop_h);
    panel_launcher(g, t, &items, rx, ty, rsl, run_h, ui, regions);
    panel_clip(g, t, rx, ty + run_h, rsl, clip_h, ui, regions);
    panel_drop(g, t, rx, ty + run_h + clip_h, rsl, drop_h);
    // right tool stack: todo / pomodoro
    let pomo_h = 15.min(tools_h.saturating_sub(6));
    let todo_h = tools_h.saturating_sub(pomo_h);
    panel_todo(g, t, rx + rsl, ty, rsr, todo_h, ui, regions);
    panel_pomo(g, t, rx + rsl, ty + todo_h, rsr, pomo_h, ui, regions);

    footer(g, t, s, rows - 1, frame);

    // Drawn last so they overlay whatever panel is underneath.
    toasts_overlay(g, t);
}

fn kind_color(t: &Theme, k: events::Kind) -> u32 {
    match k {
        events::Kind::Info => t.fg,
        events::Kind::Ok => t.ok,
        events::Kind::Warn => t.warn,
        events::Kind::Err => t.crit,
    }
}

use crate::state::fmt_bps;

fn metric_color(t: &Theme, metric: &str) -> u32 {
    match metric {
        "cpu" => t.warn,
        "gpu" => t.ok,
        "io" => t.magenta,
        "net" => t.cyan,
        "lat" => t.cyan,
        "mem" => t.crit,
        _ => t.label,
    }
}

/// Top-right toast stack. Each toast is a small 3-row box in its kind colour.
fn toasts_overlay(g: &mut Grid, t: &Theme) {
    let toasts = events::toasts();
    if toasts.is_empty() {
        return;
    }
    let w = 44.min(g.cols.saturating_sub(4));
    if w < 16 {
        return;
    }
    let x = g.cols - w - 2;
    let mut y = 2;
    for (kind, title, body) in toasts {
        if y + 3 >= g.rows {
            break;
        }
        let c = kind_color(t, kind);
        g.shade(x, y, w, 3, ' ', t.fg);
        g.panel(x, y, w, 3, "", c, c);
        let mut cx = g.text(x + 2, y + 1, "▪ ", c);
        cx = g.text_clip_end(cx, y + 1, &title, x + w - 2, t.bright);
        if !body.is_empty() {
            cx = g.text(cx, y + 1, "  ", t.fg);
            g.text_clip_end(cx, y + 1, &body, x + w - 2, t.label);
        }
        y += 3;
    }
}

fn header(g: &mut Grid, t: &Theme, s: &Snapshot, frame: u64) {
    let mut x = g.text(1, 0, "▄▀", t.ok);
    x = g.text(x + 1, 0, "DCC", t.bright);
    x = g.text(x + 1, 0, "▀▄", t.ok);

    x = g.text(x + 2, 0, &s.host, t.cyan);
    x = g.text(x + 1, 0, &SEP.to_string(), t.frame);
    x = g.text(x + 1, 0, &s.cpu_model, t.fg);
    x = g.text(x + 1, 0, &SEP.to_string(), t.frame);
    x = g.text(x + 1, 0, &format!("{} threads", s.cores), t.label);
    x = g.text(x + 1, 0, &SEP.to_string(), t.frame);
    g.text(x + 1, 0, &format!("up {}", fmt_duration(s.uptime_secs)), t.label);

    let right = format!("{}  {}  ", s.os, SEP);
    g.text_right(g.cols.saturating_sub(1), 0, &right, t.label);

    let dot = if (frame / 5) % 2 == 0 { '█' } else { ' ' };
    g.put(g.cols.saturating_sub(2), 0, dot, t.ok);

    g.hline(0, 1, g.cols, '━', t.frame);
}

// ── left: resources ───────────────────────────────────────────────────────

fn panel_sys(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "cpu", t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;

    let cpu = s.cpu_pct;
    g.text(ix, y + 1, "load", t.label);
    g.meter(ix + 5, y + 1, iw.saturating_sub(13), cpu / 100.0, pct_color(t, cpu), t.frame);
    g.text_right(x + w - 2, y + 1, &format!("{:>5.1}%", cpu), pct_color(t, cpu));

    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    g.text(ix, y + 2, "core", t.label);
    for (i, v) in s.per_core.iter().enumerate() {
        let lvl = ((v / 100.0) * 8.0).round().clamp(0.0, 8.0) as usize;
        let ch = if lvl == 0 { '▁' } else { BLOCKS[lvl] };
        if ix + 6 + i * 2 + 1 < x + w - 1 {
            g.put(ix + 6 + i * 2, y + 2, ch, pct_color(t, *v));
            g.put(ix + 7 + i * 2, y + 2, ch, pct_color(t, *v));
        }
    }

    // vmstat's us/sy plus the Windows slices: dpc+interrupt share and
    // interrupt rate.
    g.text(ix, y + 3, "us", t.label);
    g.text(ix + 3, y + 3, &format!("{:>2.0}", s.cpu_user_pct), t.fg);
    g.text(ix + 6, y + 3, "sy", t.label);
    g.text(ix + 9, y + 3, &format!("{:>2.0}", s.cpu_sys_pct), t.fg);
    g.text(ix + 12, y + 3, "dpc", t.label);
    g.text(ix + 16, y + 3, &format!("{:>3.1}", s.cpu_dpc_pct), if s.cpu_dpc_pct > 10.0 { t.warn } else { t.fg });
    g.text_right(x + w - 2, y + 3, &format!("int {:.0}k/s", s.intr_ps / 1000.0), t.label);

    let gh = h.saturating_sub(6);
    g.graph(ix, y + 4, iw, gh, &s.cpu_hist, 100.0, t.ok, t.frame);
    g.text_right(x + w - 2, y + h - 2, "2m", t.frame);
}

fn panel_gpu(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    let gp = &s.gpu;
    g.panel(x, y, w, h, "gpu", t.frame, t.ok);
    let ix = x + 2;
    if !gp.ok {
        g.text(ix, y + 1, "no nvml — nvidia only", t.frame);
        return;
    }
    let iw = w - 4;
    g.text_clip(ix, y + 1, &gp.name, iw.saturating_sub(12), t.bright);
    let temp_fg = if gp.temp_c >= 83 { t.crit } else if gp.temp_c >= 70 { t.warn } else { t.label };
    g.text_right(x + w - 2, y + 1, &format!("{}°C {:.0}W", gp.temp_c, gp.power_w), temp_fg);

    g.text(ix, y + 2, "load", t.label);
    g.meter(ix + 5, y + 2, iw.saturating_sub(13), gp.util_pct / 100.0, pct_color(t, gp.util_pct), t.frame);
    g.text_right(x + w - 2, y + 2, &format!("{:>4.0}%", gp.util_pct), pct_color(t, gp.util_pct));

    let vpct = if gp.mem_total_gib > 0.0 { gp.mem_used_gib / gp.mem_total_gib * 100.0 } else { 0.0 };
    g.text(ix, y + 3, "vram", t.label);
    g.meter(ix + 5, y + 3, iw.saturating_sub(13), vpct / 100.0, pct_color(t, vpct), t.frame);
    g.text_right(
        x + w - 2,
        y + 3,
        &format!("{:.1}/{:.0}G", gp.mem_used_gib, gp.mem_total_gib),
        t.label,
    );

    let gh = h.saturating_sub(6);
    g.graph(ix, y + 4, iw, gh, &gp.hist, 100.0, t.ok, t.frame);
    g.text_right(x + w - 2, y + h - 2, "2m", t.frame);
}

fn panel_mem(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "mem", t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;
    let mpct = if s.mem_total_gib > 0.0 { s.mem_used_gib / s.mem_total_gib * 100.0 } else { 0.0 };
    g.text(ix, y + 1, "used", t.label);
    g.meter(ix + 5, y + 1, iw.saturating_sub(13), mpct / 100.0, pct_color(t, mpct), t.frame);
    g.text_right(x + w - 2, y + 1, &format!("{:>5.1}%", mpct), pct_color(t, mpct));
    g.text(ix + 5, y + 2, &format!("{:.1} / {:.1} GiB", s.mem_used_gib, s.mem_total_gib), t.fg);

    let gh = h.saturating_sub(5);
    g.graph(ix, y + 3, iw, gh, &s.mem_hist, 100.0, t.cyan, t.frame);
}

fn panel_net(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "net", t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;
    g.text(ix, y + 1, "rx", t.label);
    g.text(ix + 3, y + 1, &format!("{:>8.2} Mbit/s", s.net_down_mbps), t.cyan);
    g.text_right(x + w - 2, y + 1, &format!("{:.1} GiB", s.net_rx_total as f64 / 1073741824.0), t.frame);
    g.text(ix, y + 2, "tx", t.label);
    g.text(ix + 3, y + 2, &format!("{:>8.2} Mbit/s", s.net_up_mbps), t.magenta);
    g.text_right(x + w - 2, y + 2, &format!("{:.1} GiB", s.net_tx_total as f64 / 1073741824.0), t.frame);

    let peak = s.net_hist.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let gh = h.saturating_sub(5);
    g.graph(ix, y + 3, iw, gh, &s.net_hist, peak, t.cyan, t.frame);
    g.text_right(x + w - 2, y + h - 2, &format!("peak {:.0} Mbit/s", peak), t.frame);
}

fn panel_disk(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "vol", t.frame, t.ok);
    let ix = x + 2;
    for (i, d) in s.disks.iter().enumerate() {
        let cy = y + 1 + i;
        if cy >= y + h - 1 {
            break;
        }
        let pct = if d.total_gib > 0.0 { d.used_gib / d.total_gib * 100.0 } else { 0.0 };
        g.text(ix, cy, &d.name, t.bright);
        g.meter(ix + 4, cy, w.saturating_sub(22), pct / 100.0, pct_color(t, pct), t.frame);
        g.text_right(x + w - 2, cy, &format!("{:>4.0}%  {:>5.0}G", pct, d.total_gib - d.used_gib), t.label);
    }
}

fn panel_clock(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "clk", t.frame, t.ok);
    let inner_w = w - 4;
    let bw = crate::bigtext::measure(&s.clock_hm);
    let bx = x + 2 + inner_w.saturating_sub(bw) / 2;
    crate::bigtext::draw(g, bx, y + 1, &s.clock_hm, t.ok, t.frame);
    g.text(x + 2, y + h - 2, &s.date, t.label);
    g.text_right(x + w - 2, y + h - 2, &format!(":{}", s.clock_s), t.bright);
}

// ── middle A: who is doing what ───────────────────────────────────────────

/// Four ways to be hot, one column each: compute, memory, disk+net transfer,
/// and open connections. The same process showing up in three columns is
/// itself information.
fn panel_hot(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    let title = format!("hot · {} tasks · {} threads", s.task_count, s.thread_count);
    g.panel(x, y, w, h, &title, t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;
    let rows = h.saturating_sub(3);
    let cw = iw / 5;

    // by cpu
    let col_x = ix;
    g.text(col_x, y + 1, "CPU%", t.label);
    for (i, p) in s.procs.iter().take(rows).enumerate() {
        let cy = y + 2 + i;
        g.text(col_x, cy, &format!("{:>5.1}", p.cpu_pct), pct_color(t, p.cpu_pct * 4.0));
        g.text_clip(col_x + 6, cy, &p.name, cw.saturating_sub(8), if i < 2 { t.bright } else { t.fg });
    }

    // by memory
    let mut by_mem: Vec<&Proc> = s.procs.iter().collect();
    by_mem.sort_by(|a, b| b.mem_mib.partial_cmp(&a.mem_mib).unwrap());
    let col_x = ix + cw;
    g.text(col_x, y + 1, "MEM", t.cyan);
    for (i, p) in by_mem.iter().take(rows).enumerate() {
        let cy = y + 2 + i;
        let mem = if p.mem_mib >= 1024.0 {
            format!("{:>5.1}G", p.mem_mib / 1024.0)
        } else {
            format!("{:>5.0}M", p.mem_mib)
        };
        g.text(col_x, cy, &mem, t.cyan);
        g.text_clip(col_x + 7, cy, &p.name, cw.saturating_sub(9), t.fg);
    }

    // by io transfer
    let mut by_io: Vec<&Proc> = s.procs.iter().filter(|p| p.io_bps > 10_000.0).collect();
    by_io.sort_by(|a, b| b.io_bps.partial_cmp(&a.io_bps).unwrap());
    let col_x = ix + cw * 2;
    g.text(col_x, y + 1, "IO/s", t.magenta);
    if by_io.is_empty() {
        g.text(col_x, y + 2, "—", t.frame);
    }
    for (i, p) in by_io.iter().take(rows).enumerate() {
        let cy = y + 2 + i;
        g.text(col_x, cy, &format!("{:>7}", fmt_bps(p.io_bps)), t.magenta);
        g.text_clip(col_x + 8, cy, &p.name, cw.saturating_sub(10), t.fg);
    }

    // by network bytes (etw)
    let col_x = ix + cw * 3;
    g.text(col_x, y + 1, "NET/s", t.yellow);
    if s.net_etw_active {
        let mut by_net: Vec<&Proc> = s.procs.iter().filter(|p| p.net_bps > 2_000.0).collect();
        by_net.sort_by(|a, b| b.net_bps.partial_cmp(&a.net_bps).unwrap());
        if by_net.is_empty() {
            g.text(col_x, y + 2, "—", t.frame);
        }
        for (i, p) in by_net.iter().take(rows).enumerate() {
            let cy = y + 2 + i;
            g.text(col_x, cy, &format!("{:>7}", fmt_bps(p.net_bps)), t.yellow);
            g.text_clip(col_x + 8, cy, &p.name, cw.saturating_sub(10), t.fg);
        }
    } else {
        g.text(col_x, y + 2, "needs admin", t.frame);
    }

    // by open connections
    let mut by_conn: Vec<&Proc> = s.procs.iter().filter(|p| p.net_conns > 0).collect();
    by_conn.sort_by(|a, b| b.net_conns.cmp(&a.net_conns));
    let col_x = ix + cw * 4;
    g.text(col_x, y + 1, "CONN", t.label);
    if by_conn.is_empty() {
        g.text(col_x, y + 2, "—", t.frame);
    }
    for (i, p) in by_conn.iter().take(rows).enumerate() {
        let cy = y + 2 + i;
        g.text(col_x, cy, &format!("{:>4}", p.net_conns), t.yellow);
        g.text_clip(col_x + 5, cy, &p.name, cw.saturating_sub(7), t.fg);
    }
}

/// Saturation and errors — the two thirds of the USE method that a
/// utilisation meter cannot show. Green here means genuinely healthy.
fn panel_use(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "saturation · errors", t.frame, t.warn);
    let ix = x + 2;
    let iw = w - 4;
    let mut cy = y + 1;

    // cpu: run queue against core count
    let rq = s.run_queue;
    let rq_fg = if rq as usize > s.cores * 2 { t.crit } else if rq as usize > s.cores { t.warn } else { t.ok };
    g.text(ix, cy, "cpu queue", t.label);
    g.text(ix + 13, cy, &format!("{:>4}", rq), rq_fg);
    g.text(ix + 19, cy, &format!("ready / {} cores", s.cores), t.frame);
    g.text_right(x + w - 2, cy, &format!("ctx {:.0}k/s", s.ctx_switches_ps / 1000.0), t.label);
    cy += 1;

    // memory: hard faults + commit
    let hf_fg = if s.hard_faults_ps > 500.0 { t.crit } else if s.hard_faults_ps > 100.0 { t.warn } else { t.ok };
    g.text(ix, cy, "hard faults", t.label);
    g.text(ix + 13, cy, &format!("{:>6.0}/s", s.hard_faults_ps), hf_fg);
    let cm_fg = if s.commit_pct >= 90.0 { t.crit } else if s.commit_pct >= 75.0 { t.warn } else { t.ok };
    g.text_right(x + w - 2, cy, &format!("commit {:.0}%", s.commit_pct), cm_fg);
    cy += 1;

    // network errors
    let ne_fg = if s.net_errs_ps > 1.0 { t.warn } else { t.ok };
    g.text(ix, cy, "net errors", t.label);
    g.text(ix + 13, cy, &format!("{:>6.1}/s", s.net_errs_ps), ne_fg);
    g.text_right(x + w - 2, cy, &format!("total {}", s.net_errs), t.frame);
    cy += 1;

    // system-wide io — vmstat's bi/bo
    g.text(ix, cy, "sys io", t.label);
    g.text(ix + 13, cy, &format!("r {:>9}", fmt_bps(s.disk_read_bps)), t.cyan);
    g.text(ix + 25, cy, &format!("w {:>9}", fmt_bps(s.disk_write_bps)), t.magenta);
    cy += 1;

    // latency
    let ping_fg = if !s.ping_ok {
        t.crit
    } else if s.ping_ms > 100.0 {
        t.warn
    } else {
        t.ok
    };
    g.text(ix, cy, "latency", t.label);
    if s.ping_hist.is_empty() {
        g.text(ix + 13, cy, "measuring…", t.frame);
    } else {
        g.text(
            ix + 13,
            cy,
            &if s.ping_ok { format!("{:>4.0}ms", s.ping_ms) } else { "lost".to_string() },
            ping_fg,
        );
        g.text_right(x + w - 2, cy, "1.1.1.1 · 10s", t.frame);
    }
    cy += 1;

    // latency history graph fills the rest (timeouts chart as full columns)
    let gh = (y + h - 1).saturating_sub(cy);
    if gh > 0 && !s.ping_hist.is_empty() {
        let peak = s.ping_hist.iter().cloned().fold(0.0_f64, f64::max).max(50.0);
        g.graph(ix, cy, iw, gh, &s.ping_hist, peak, t.cyan, t.frame);
        g.text_right(x + w - 2, cy, &format!("peak {:.0}ms · 15m", peak), t.frame);
    }
}

/// The spike log: bursts of CPU with start time, peak and duration. This is
/// the "what just happened" panel — the fan spun up two minutes ago, here is
/// the name that did it.
fn panel_spikes(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "spikes", t.frame, t.warn);
    let ix = x + 2;
    let rows = h.saturating_sub(2);
    if s.spikes.is_empty() {
        g.text(ix, y + 1, "none recorded — quiet machine", t.frame);
        return;
    }
    for (i, sp) in s.spikes.iter().take(rows).enumerate() {
        let cy = y + 1 + i;
        g.text(ix, cy, &sp.hms, t.frame);
        g.put(ix + 9, cy, if sp.ongoing { '●' } else { '▪' }, if sp.ongoing { t.crit } else { t.warn });
        g.text(ix + 11, cy, &format!("{:<3}", sp.metric), metric_color(t, &sp.metric));
        g.text_clip(ix + 15, cy, &sp.name, (w / 2).saturating_sub(10), if sp.ongoing { t.bright } else { t.fg });
        g.text_right(
            x + w - 2,
            cy,
            &format!(
                "peak {} · {}",
                sp.peak_txt,
                if sp.ongoing { format!("{} ●", fmt_duration_short(sp.dur_secs)) } else { fmt_duration_short(sp.dur_secs) }
            ),
            if sp.ongoing { t.warn } else { t.label },
        );
    }
}

/// The journal: every event the bus has seen, newest at the bottom, wheel to
/// scroll back.
fn panel_journal(
    g: &mut Grid,
    t: &Theme,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ui: &UiState,
    regions: &mut Regions,
) {
    let take = h.saturating_sub(2);
    let (entries, total) = events::journal_tail(take, ui.journal_scroll);

    let title = if ui.journal_scroll > 0 {
        format!("journal · {} · ▲{}", total, ui.journal_scroll)
    } else {
        format!("journal · {}", total)
    };
    g.panel(x, y, w, h, &title, t.frame, t.ok);
    regions.add(RegionId::Journal, x + 1, y + 1, w - 2, take, total.saturating_sub(take));

    let ix = x + 2;
    let hover_row = ui.hovered_in(x + 1, y + 1, w - 2, take).map(|(_, ry)| ry);
    for (i, e) in entries.iter().enumerate() {
        let cy = y + 1 + i;
        if cy >= y + h - 1 {
            break;
        }
        let hovered = hover_row == Some(i);
        g.text(ix, cy, &e.hms, if hovered { t.bright } else { t.frame });
        g.put(ix + 9, cy, '▪', kind_color(t, e.kind));
        g.text_clip(ix + 11, cy, &e.text, w.saturating_sub(15), if hovered { t.bright } else { t.fg });
    }
    if entries.is_empty() {
        g.text(ix, y + 1, "(no events yet)", t.frame);
    }
}

// ── middle B: the long view ───────────────────────────────────────────────

/// Two hours of cpu and gpu at 30-second resolution — the "what happened
/// while I was away" panel.
fn panel_timeline(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "timeline · 2h", t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;
    let half = h.saturating_sub(2) / 2;

    g.graph(ix, y + 1, iw, half, &s.cpu_slow, 100.0, t.ok, t.frame);
    g.text(ix, y + 1, "cpu", t.label);
    let cy = y + 1 + half;
    g.graph(ix, cy, iw, half, &s.gpu_slow, 100.0, t.magenta, t.frame);
    g.text(ix, cy, "gpu", t.label);
    if s.cpu_slow.len() < 4 {
        g.text(ix + 5, y + 1, "(collecting — one dot per 30s)", t.frame);
    }
}

fn panel_git(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    let title = if s.demo {
        "git · demo".to_string()
    } else {
        format!("git · {} scanned", s.repos_scanned)
    };
    g.panel(x, y, w, h, &title, t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;
    let mut cy = y + 1;

    // The heatmap owns the bottom 8 rows; lists get what is above it.
    let grass_top = y + h.saturating_sub(9);

    for r in s.repos.iter().take(3) {
        if cy >= grass_top {
            break;
        }
        g.text_clip(ix, cy, &r.name, iw / 2, t.bright);
        let mut flags = String::new();
        if r.dirty > 0 {
            flags.push_str(&format!("●{} ", r.dirty));
        }
        if r.ahead > 0 {
            flags.push_str(&format!("↑{} ", r.ahead));
        }
        if r.commits_today > 0 {
            flags.push_str(&format!("+{}", r.commits_today));
        }
        g.text_clip(ix + iw / 2, cy, &r.branch, iw / 4, t.cyan);
        g.text_right(x + w - 2, cy, &flags, if r.dirty > 0 { t.warn } else { t.label });
        cy += 1;
    }
    if cy < grass_top {
        g.hline(ix, cy, iw, '╌', t.frame);
        cy += 1;
    }
    for c in s.commits.iter() {
        if cy >= grass_top {
            break;
        }
        g.text(ix, cy, &format!("{:>3}", fmt_ago(c.secs_ago)), t.frame);
        g.text(ix + 4, cy, &c.hash, t.yellow);
        g.text_clip(ix + 12, cy, &c.msg, iw.saturating_sub(12), t.fg);
        cy += 1;
    }

    if !s.grass.is_empty() && h >= 12 {
        g.hline(ix, grass_top, iw, '╌', t.frame);
        g.text(ix + 1, grass_top, " 26w ", t.label);
        let ramp = [t.frame, t.label, t.fg, t.ok, t.bright];
        let weeks = (s.grass.len() / 7).min(iw / 2);
        for day in 0..7 {
            let ry = grass_top + 1 + day;
            if ry >= y + h - 1 {
                break;
            }
            for wk in 0..weeks {
                let lvl = s.grass[wk * 7 + day] as usize % ramp.len();
                let ch = if lvl == 0 { '·' } else { '■' };
                g.put(ix + wk * 2, ry, ch, ramp[lvl]);
            }
        }
    }
}

fn panel_claude(g: &mut Grid, t: &Theme, s: &Snapshot, x: usize, y: usize, w: usize, h: usize) {
    let c = &s.claude;
    g.panel(x, y, w, h, "claude", t.frame, t.magenta);
    let ix = x + 2;
    let iw = w - 4;

    if !c.ok {
        g.text(ix, y + 1, "reading transcripts…", t.frame);
        return;
    }

    g.text(ix, y + 1, "today", t.label);
    g.text(ix + 7, y + 1, &fmt_tokens(c.today_tokens), t.bright);
    g.text_right(x + w - 2, y + 1, &format!("${:.2}", c.today_cost), t.ok);

    g.text(ix, y + 2, "7d", t.label);
    g.text(ix + 7, y + 2, &fmt_tokens(c.week_tokens), t.fg);
    g.text_right(x + w - 2, y + 2, &format!("${:.2}", c.week_cost), t.ok);

    g.text(ix, y + 3, "cache", t.label);
    g.text(
        ix + 7,
        y + 3,
        &format!("{} r / {} w", fmt_tokens(c.cache_read), fmt_tokens(c.cache_write)),
        t.cyan,
    );

    // Sessions board (bottom) and the daily-tokens graph fill what remains.
    let board = c.sessions.len().min(3);
    if h > 7 + board && !c.sparkline.is_empty() {
        let gh = h.saturating_sub(7 + board);
        let data = &c.sparkline;
        let max = data.iter().cloned().fold(0.0_f64, f64::max).max(0.001);
        g.graph(ix, y + 5, iw, gh, data, max, t.magenta, t.frame);
        g.text(ix, y + 5, &format!("{}d", data.len()), t.frame);
        g.text_right(x + w - 2, y + 5, &c.top_model, t.label);
    }

    for (i, sess) in c.sessions.iter().take(board).enumerate() {
        let cy = y + h - 1 - board + i;
        let live = sess.idle_secs < 120;
        g.put(ix, cy, if live { '●' } else { '○' }, if live { t.ok } else { t.frame });
        g.text_clip(ix + 2, cy, &sess.project, iw / 2, if live { t.bright } else { t.fg });
        g.text(ix + 2 + iw / 2, cy, &sess.model, t.label);
        g.text_right(
            x + w - 2,
            cy,
            &format!("{} · {}", fmt_tokens(sess.today_tokens), if live { "live".to_string() } else { fmt_ago(sess.idle_secs) }),
            if live { t.ok } else { t.frame },
        );
    }
}

// ── right: the day ────────────────────────────────────────────────────────

fn time_progress() -> [(char, f64); 3] {
    let (y, mo, d, h, mi, s) = crate::collect::local_time();
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mdays = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let day_frac = (h as f64 * 3600.0 + mi as f64 * 60.0 + s as f64) / 86400.0;
    let dim = mdays[(mo as usize).clamp(1, 12) - 1] as f64;
    let month_frac = ((d as f64 - 1.0) + day_frac) / dim;
    let doy: f64 = mdays[..(mo as usize).clamp(1, 12) - 1].iter().map(|v| *v as f64).sum::<f64>()
        + (d as f64 - 1.0)
        + day_frac;
    let year_frac = doy / if leap { 366.0 } else { 365.0 };
    [('Y', year_frac), ('M', month_frac), ('D', day_frac)]
}

fn panel_day(g: &mut Grid, t: &Theme, f: &crate::feeds::Feeds, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "day", t.frame, t.ok);
    let ix = x + 2;
    let iw = w - 4;
    let mut cy = y + 1;

    let wx = &f.weather;
    if wx.ok {
        let mut cx = g.text(ix, cy, "seoul", t.label);
        cx = g.text(cx + 1, cy, &format!("{:.1}°", wx.temp_c), t.bright);
        g.text_clip_end(cx + 1, cy, &wx.desc, x + w - 20, t.fg);
        g.text_right(
            x + w - 2,
            cy,
            &format!("rain {:.0}% · {:.0}~{:.0}°", wx.rain_prob, wx.tmin, wx.tmax),
            if wx.rain_prob >= 50.0 { t.cyan } else { t.label },
        );
    } else {
        g.text(ix, cy, "seoul  — fetching…", t.frame);
    }
    cy += 1;

    let tk = &f.ticker;
    if tk.ok {
        let mut cx = g.text(ix, cy, "USD", t.label);
        cx = g.text(cx + 1, cy, &format!("{:.1}", tk.usd_krw), t.bright);
        cx = g.text(cx + 3, cy, "BTC", t.label);
        cx = g.text(cx + 1, cy, &format!("{:.1}M", tk.btc_krw / 1e6), t.bright);
        let up = tk.btc_chg_pct >= 0.0;
        g.text(
            cx + 1,
            cy,
            &format!("{}{:.1}%", if up { '▲' } else { '▼' }, tk.btc_chg_pct.abs()),
            if up { t.ok } else { t.crit },
        );
    } else {
        g.text(ix, cy, "ticker — fetching…", t.frame);
    }
    cy += 1;

    for (label, frac) in time_progress() {
        let pct = frac * 100.0;
        g.put(ix, cy, label, t.label);
        g.meter(ix + 2, cy, iw.saturating_sub(10), frac, t.ok, t.frame);
        g.text_right(x + w - 2, cy, &format!("{:>5.1}%", pct), t.fg);
        cy += 1;
    }

    if cy < y + h - 1 {
        g.hline(ix, cy, iw, '╌', t.frame);
        g.text(ix + 1, cy, " today ", t.label);
        cy += 1;
    }
    if !f.cal_configured {
        if cy < y + h - 1 {
            g.text(ix, cy, "calendar: add ics url to dcc.conf", t.frame);
        }
    } else if f.cal.is_empty() {
        if cy < y + h - 1 {
            g.text(ix, cy, "no events today", t.frame);
        }
    } else {
        for ev in f.cal.iter() {
            if cy >= y + h - 1 {
                break;
            }
            let hm = if ev.hm.is_empty() { "all·d" } else { &ev.hm };
            g.text(ix, cy, hm, t.cyan);
            g.text_clip_end(ix + 7, cy, &ev.summary, x + w - 2, t.fg);
            cy += 1;
        }
    }
}

fn panel_news(
    g: &mut Grid,
    t: &Theme,
    f: &crate::feeds::Feeds,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ui: &UiState,
    regions: &mut Regions,
) {
    g.panel(x, y, w, h, "news", t.frame, t.ok);
    let ix = x + 2;
    let rows = h.saturating_sub(2);
    let shown = rows.min(f.news.len());
    regions.add(RegionId::News, x + 1, y + 1, w - 2, shown, 0);
    let hover_row = ui.hovered_in(x + 1, y + 1, w - 2, shown).map(|(_, ry)| ry);

    if f.news.is_empty() {
        g.text(ix, y + 1, "no headlines yet", t.frame);
        return;
    }
    for (i, hl) in f.news.iter().take(rows).enumerate() {
        let cy = y + 1 + i;
        let hovered = hover_row == Some(i);
        g.put(ix, cy, if hovered { '▸' } else { '·' }, if hovered { t.bright } else { t.frame });
        g.text_clip_end(ix + 2, cy, &hl.title, x + w - 2, if hovered { t.bright } else { t.fg });
    }
}

// ── right: tools ──────────────────────────────────────────────────────────

fn panel_launcher(
    g: &mut Grid,
    t: &Theme,
    items: &[crate::config::LauncherItem],
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ui: &UiState,
    regions: &mut Regions,
) {
    g.panel(x, y, w, h, "run", t.frame, t.ok);
    let ix = x + 2;
    let rows = h.saturating_sub(2);
    regions.add(RegionId::Launcher, x + 1, y + 1, w - 2, rows.min(items.len()), 0);
    let hover_row = ui.hovered_in(x + 1, y + 1, w - 2, rows.min(items.len())).map(|(_, ry)| ry);

    if items.is_empty() {
        g.text(ix, y + 1, "(edit dcc.conf)", t.frame);
        return;
    }
    for (i, it) in items.iter().take(rows).enumerate() {
        let cy = y + 1 + i;
        let hovered = hover_row == Some(i);
        g.put(ix, cy, if hovered { '▸' } else { '·' }, if hovered { t.bright } else { t.frame });
        g.text_clip_end(ix + 2, cy, &it.label, x + w - 2, if hovered { t.bright } else { t.fg });
    }
}

fn panel_clip(
    g: &mut Grid,
    t: &Theme,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ui: &UiState,
    regions: &mut Regions,
) {
    let items = crate::clipboard::items();
    g.panel(x, y, w, h, "clip", t.frame, t.ok);
    let ix = x + 2;
    let rows = h.saturating_sub(2);
    regions.add(RegionId::ClipList, x + 1, y + 1, w - 2, rows.min(items.len()), 0);
    let hover_row = ui.hovered_in(x + 1, y + 1, w - 2, rows.min(items.len())).map(|(_, ry)| ry);

    if items.is_empty() {
        g.text(ix, y + 1, "(clipboard is empty)", t.frame);
        return;
    }
    for (i, c) in items.iter().take(rows).enumerate() {
        let cy = y + 1 + i;
        let hovered = hover_row == Some(i);
        g.put(ix, cy, if hovered { '▸' } else { '·' }, if hovered { t.bright } else { t.frame });
        let fg = if hovered { t.bright } else if i == 0 { t.fg } else { t.label };
        g.text_clip_end(ix + 2, cy, &c.preview, x + w - 2, fg);
    }
}

/// Placeholder for the phase-4 drop targets.
fn panel_drop(g: &mut Grid, t: &Theme, x: usize, y: usize, w: usize, h: usize) {
    g.panel(x, y, w, h, "drop · demo", t.frame, t.label);
    let ix = x + 2;
    if h > 3 {
        g.text(ix, y + 1, "⇩ drag files here", t.label);
        g.text(ix, y + 2, "→ drive · sort · trash", t.frame);
    }
    if h > 5 {
        g.text(ix, y + h - 2, "(phase 4)", t.frame);
    }
}

/// TODO.md, click to toggle. The palette's `+text` appends here.
fn panel_todo(
    g: &mut Grid,
    t: &Theme,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ui: &UiState,
    regions: &mut Regions,
) {
    let todos = crate::todo::items();
    g.panel(x, y, w, h, "todo", t.frame, t.ok);
    let ix = x + 2;
    let rows = h.saturating_sub(2);
    regions.add(RegionId::Todo, x + 1, y + 1, w - 2, rows.min(todos.len()), 0);
    let hover_row = ui.hovered_in(x + 1, y + 1, w - 2, rows.min(todos.len())).map(|(_, ry)| ry);
    if todos.is_empty() {
        g.text(ix, y + 1, "(TODO.md is empty)", t.frame);
        return;
    }
    let open = todos.iter().filter(|(done, _)| !done).count();
    for (i, (done, text)) in todos.iter().take(rows).enumerate() {
        let cy = y + 1 + i;
        let hovered = hover_row == Some(i);
        if *done {
            g.text(ix, cy, "[x]", t.ok);
            g.text_clip_end(ix + 4, cy, text, x + w - 2, if hovered { t.fg } else { t.frame });
        } else {
            g.text(ix, cy, "[ ]", t.warn);
            g.text_clip_end(ix + 4, cy, text, x + w - 2, if hovered { t.bright } else { t.fg });
        }
    }
    g.text_right(x + w - 2, y + h - 1, &format!(" {} open ", open), t.label);
}

/// Braille dot layout within a cell: (col 0..2, row 0..4) → bit.
const BRAILLE_BITS: [u8; 8] = [0x01, 0x02, 0x04, 0x40, 0x08, 0x10, 0x20, 0x80];

/// A high-resolution pie in braille dots: each cell becomes a 2×4 dot
/// canvas, so the pomodoro's disc is drawn at eight times the grid's
/// resolution. The filled sector sweeps clockwise from twelve.
fn braille_pie(g: &mut Grid, x: usize, y: usize, wc: usize, hc: usize, frac: f64, fg: u32, dim: u32) {
    let dw = wc * 2;
    let dh = hc * 4;
    let r = (dw.min(dh) as f64) / 2.0 - 1.0;
    if r < 4.0 {
        return;
    }
    let cx = dw as f64 / 2.0;
    let cy = dh as f64 / 2.0;
    let mut fill = vec![0u8; wc * hc];
    let mut ring = vec![0u8; wc * hc];
    for dy in 0..dh {
        for dx in 0..dw {
            let fx = dx as f64 + 0.5 - cx;
            let fy = dy as f64 + 0.5 - cy;
            let dist = (fx * fx + fy * fy).sqrt();
            if dist > r {
                continue;
            }
            let cell = (dy / 4) * wc + dx / 2;
            let bit = BRAILLE_BITS[(dx % 2) * 4 + dy % 4];
            if dist >= r - 1.3 {
                ring[cell] |= bit;
            }
            // Angle from twelve o'clock, clockwise, 0..1.
            let mut a = fx.atan2(-fy) / std::f64::consts::TAU;
            if a < 0.0 {
                a += 1.0;
            }
            if a <= frac {
                fill[cell] |= bit;
            }
        }
    }
    for row in 0..hc {
        for col in 0..wc {
            let i = row * wc + col;
            if fill[i] != 0 {
                if let Some(ch) = char::from_u32(0x2800 + fill[i] as u32) {
                    g.put(x + col, y + row, ch, fg);
                }
            } else if ring[i] != 0 {
                if let Some(ch) = char::from_u32(0x2800 + ring[i] as u32) {
                    g.put(x + col, y + row, ch, dim);
                }
            }
        }
    }
}

/// The pomodoro: a dot-matrix disc that fills clockwise, no numbers — not
/// knowing is the point. Hover reveals the remaining time for the weak.
fn panel_pomo(
    g: &mut Grid,
    t: &Theme,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ui: &UiState,
    regions: &mut Regions,
) {
    let (time, frac, running) = crate::pomo::status();
    g.panel(x, y, w, h, "pomo", t.frame, if running { t.ok } else { t.label });
    regions.add(RegionId::Pomo, x + 1, y + 1, w - 2, h.saturating_sub(2), 0);

    let wc = w.saturating_sub(4);
    let hc = h.saturating_sub(3); // last row is the hint
    if wc < 8 || hc < 4 {
        g.text(x + 2, y + 1, &time, t.bright);
        return;
    }
    let fg = if running { t.ok } else { t.label };
    braille_pie(g, x + 2, y + 1, wc, hc, frac, fg, t.frame);

    if ui.hovered_in(x + 1, y + 1, w - 2, h.saturating_sub(2)).is_some() {
        g.text(x + w / 2 - 2, y + 1 + hc / 2, &time, t.bright);
    }

    g.text_clip(x + 2, y + h - 2, "click start·pause / rclick reset", w.saturating_sub(4), t.frame);
}

fn footer(g: &mut Grid, t: &Theme, s: &Snapshot, y: usize, frame: u64) {
    let mut x = g.text(1, y, "●", t.ok);
    x = g.text(x + 1, y, "live", t.ok);
    x = g.text(x + 1, y, &SEP.to_string(), t.frame);
    x = g.text(x + 1, y, &format!("collect {:.2}ms", s.collect_micros as f64 / 1000.0), t.label);
    x = g.text(x + 1, y, &SEP.to_string(), t.frame);
    x = g.text(x + 1, y, &format!("agent up {}", fmt_duration(s.agent_uptime_secs)), t.label);
    x = g.text(x + 1, y, &SEP.to_string(), t.frame);
    g.text(x + 1, y, &format!("frame {}", frame), t.frame);

    let spin = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    g.put(g.cols.saturating_sub(2), y, spin[(frame as usize) % spin.len()], t.ok);
}

fn fmt_duration_short(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}
