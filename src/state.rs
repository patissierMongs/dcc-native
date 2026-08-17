//! The single struct the UI reads. Collectors fill it on a worker thread; the
//! render thread takes a snapshot clone and never blocks on I/O.

#[derive(Clone, Default)]
pub struct Disk {
    pub name: String,
    pub used_gib: f64,
    pub total_gib: f64,
}

#[derive(Clone, Default)]
pub struct Proc {
    pub name: String,
    pub count: u32,
    pub cpu_pct: f64,
    pub mem_mib: f64,
    /// Disk + network transfer, bytes per second, from the kernel's per-
    /// process IO counters.
    pub io_bps: f64,
    /// Established TCP connections owned by this process — the fallback
    /// network signal when the ETW session isn't available.
    pub net_conns: u32,
    /// Network bytes per second (send+recv) from the ETW kernel-network
    /// session; zero when tracing is off.
    pub net_bps: f64,
    /// Recent per-process CPU history (only filled for the hottest few).
    pub hist: Vec<f32>,
}

/// A burst worth remembering — any metric: when, what kind, who, how hard,
/// how long.
#[derive(Clone, Default)]
pub struct Spike {
    pub hms: String,
    /// "cpu", "io", "gpu", "net", "lat", "mem" — colours the tag.
    pub metric: String,
    pub name: String,
    /// Peak formatted with its unit at record time ("83%", "412M/s").
    pub peak_txt: String,
    pub dur_secs: u64,
    pub ongoing: bool,
}

pub fn fmt_bps(bps: f64) -> String {
    if bps >= 1e9 {
        format!("{:.1}G/s", bps / 1e9)
    } else if bps >= 1e6 {
        format!("{:.1}M/s", bps / 1e6)
    } else if bps >= 1e3 {
        format!("{:.0}K/s", bps / 1e3)
    } else {
        format!("{:.0}B/s", bps)
    }
}

#[derive(Clone, Default)]
pub struct Repo {
    pub name: String,
    pub branch: String,
    pub dirty: usize,
    pub ahead: usize,
    pub commits_today: usize,
}

#[derive(Clone, Default)]
pub struct Commit {
    pub repo: String,
    pub hash: String,
    pub secs_ago: u64,
    pub msg: String,
}

#[derive(Clone, Default)]
pub struct ClaudeSession {
    pub project: String,
    pub model: String,
    pub idle_secs: u64,
    pub today_tokens: u64,
}

#[derive(Clone, Default)]
pub struct Claude {
    pub ok: bool,
    pub today_tokens: u64,
    pub today_cost: f64,
    pub week_tokens: u64,
    pub week_cost: f64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub sparkline: Vec<f64>,
    pub top_model: String,
    pub session_project: String,
    pub session_idle_secs: u64,
    /// The Herd-style board: recent sessions, freshest first.
    pub sessions: Vec<ClaudeSession>,
}

#[derive(Clone, Default)]
pub struct Gpu {
    pub ok: bool,
    pub name: String,
    pub util_pct: f64,
    pub mem_used_gib: f64,
    pub mem_total_gib: f64,
    pub temp_c: u32,
    pub power_w: f64,
    pub hist: Vec<f64>,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub host: String,
    pub user: String,
    pub os: String,
    pub cpu_model: String,
    pub cores: usize,
    pub cpu_pct: f64,
    pub per_core: Vec<f64>,
    /// vmstat's us/sy/id split plus the Windows-specific slices.
    pub cpu_user_pct: f64,
    pub cpu_sys_pct: f64,
    pub cpu_dpc_pct: f64,
    pub intr_ps: f64,
    /// System-wide IO transfer rates (all devices, from the kernel's own
    /// counters) — vmstat's bi/bo.
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    pub mem_used_gib: f64,
    pub mem_total_gib: f64,
    pub uptime_secs: u64,
    pub disks: Vec<Disk>,
    pub net_down_mbps: f64,
    pub net_up_mbps: f64,
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    pub procs: Vec<Proc>,
    pub task_count: usize,
    pub thread_count: usize,
    pub handle_count: usize,
    pub repos: Vec<Repo>,
    pub repos_scanned: usize,
    pub commits: Vec<Commit>,
    pub claude: Claude,
    pub log: Vec<String>,
    pub agent_uptime_secs: u64,
    pub collect_micros: u64,
    pub clock_hm: String,
    pub clock_s: String,
    pub date: String,
    pub cpu_hist: Vec<f64>,
    pub net_hist: Vec<f64>,
    pub mem_hist: Vec<f64>,
    /// 30-second cadence, two hours deep — the "what happened while I was
    /// away" rings behind the timeline panel.
    pub cpu_slow: Vec<f64>,
    pub gpu_slow: Vec<f64>,
    pub net_slow: Vec<f64>,
    pub spikes: Vec<Spike>,
    pub gpu: Gpu,
    // ── saturation & errors (the USE method's missing two thirds) ──
    /// Threads ready to run but waiting for a core, right now.
    pub run_queue: u32,
    pub ctx_switches_ps: f64,
    /// Page reads from disk per second — memory pressure, not soft faults.
    pub hard_faults_ps: f64,
    /// Commit charge against the commit limit.
    pub commit_pct: f64,
    /// Cumulative interface errors + discards, and their current rate.
    pub net_errs: u64,
    pub net_errs_ps: f64,
    /// True while the ETW per-process network session is delivering bytes.
    pub net_etw_active: bool,
    pub ping_ms: f64,
    pub ping_ok: bool,
    pub ping_hist: Vec<f64>,
    /// Synthesised load averages: EMA of (run queue + running work).
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    /// True while the git panels hold placeholder data — their titles carry
    /// a "demo" tag so nobody mistakes the dress rehearsal for fact.
    pub demo: bool,
    /// 26 weeks × 7 days of commit counts for the contribution heatmap,
    /// oldest week first.
    pub grass: Vec<u8>,
}

/// Push onto a fixed-length history ring used for the sparklines.
pub fn push_hist(v: &mut Vec<f64>, value: f64, cap: usize) {
    v.push(value);
    if v.len() > cap {
        let excess = v.len() - cap;
        v.drain(0..excess);
    }
}

pub fn fmt_duration(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{}d {:02}:{:02}:{:02}", d, h, m, s)
    } else {
        format!("{:02}:{:02}:{:02}", h, m, s)
    }
}

pub fn fmt_tokens(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        format!("{:.2}B", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1}M", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1}K", f / 1e3)
    } else {
        format!("{}", n)
    }
}

pub fn fmt_ago(secs: u64) -> String {
    if secs < 45 {
        "now".into()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}
