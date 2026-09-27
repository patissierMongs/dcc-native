//! Canned data for `dcc --shot out.png --demo`.
//!
//! Screenshots for the README should show every panel populated — without
//! the real machine's process list, hostname, calendar, clipboard or screen
//! time. Nothing here reads the system or the network; the series are
//! deterministic sums of sines, so the same command renders the same image.

use crate::events::{self, Kind};
use crate::feeds::{AwApp, AwDay, CalEvent, Feeds, Headline, Ticker, Weather};
use crate::state::*;

fn wave(i: usize, base: f64, amp: f64, seed: f64) -> f64 {
    let x = i as f64;
    let v = (x * 0.21 + seed).sin() * 0.55 + (x * 0.057 + seed * 2.0).sin() * 0.3 + (x * 0.93 + seed * 3.0).sin() * 0.15;
    (base + amp * v).max(0.0)
}

fn series(n: usize, base: f64, amp: f64, seed: f64) -> Vec<f64> {
    (0..n).map(|i| wave(i, base, amp, seed)).collect()
}

/// "HH:MM:SS" some minutes before now, so the logs read as today.
fn ago(mins: u32) -> String {
    let (_, _, _, h, m, s) = crate::collect::local_time();
    let now = h as i64 * 3600 + m as i64 * 60 + s as i64;
    let t = (now - mins as i64 * 60).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", t / 3600, (t % 3600) / 60, t % 60)
}

fn proc(name: &str, cpu: f64, mem_mib: f64, io_bps: f64, conns: u32, net_bps: f64) -> Proc {
    Proc {
        name: name.into(),
        count: 1,
        cpu_pct: cpu,
        mem_mib,
        io_bps,
        net_conns: conns,
        net_bps,
        hist: Vec::new(),
    }
}

pub fn snapshot() -> Snapshot {
    let mut s = Snapshot::default();
    s.host = "workstation".into();
    s.user = "demo".into();
    s.os = "Windows 11 26200".into();
    s.cpu_model = "AMD Ryzen 7 7700X 8-Core Processor".into();
    s.cores = 16;
    s.uptime_secs = 3 * 86400 + 4 * 3600 + 12 * 60 + 55;
    s.agent_uptime_secs = 5 * 3600 + 21 * 60;
    s.collect_micros = 4380;

    // ── cpu
    s.per_core = (0..16).map(|i| wave(i * 5, 34.0, 40.0, 1.3).min(100.0)).collect();
    s.cpu_pct = 31.4;
    s.cpu_user_pct = 24.0;
    s.cpu_sys_pct = 6.0;
    s.cpu_dpc_pct = 0.8;
    s.intr_ps = 21_000.0;
    s.cpu_hist = series(120, 30.0, 24.0, 0.4).into_iter().map(|v| v.min(100.0)).collect();

    // ── gpu
    s.gpu = Gpu {
        ok: true,
        name: "NVIDIA GeForce RTX 4080 SUPER".into(),
        util_pct: 23.0,
        mem_used_gib: 6.1,
        mem_total_gib: 16.0,
        temp_c: 54,
        power_w: 96.0,
        hist: series(120, 22.0, 18.0, 2.2),
    };

    // ── memory, network, volumes
    s.mem_used_gib = 21.7;
    s.mem_total_gib = 63.6;
    s.mem_hist = series(120, 34.0, 3.0, 0.9);
    s.net_down_mbps = 48.3;
    s.net_up_mbps = 2.1;
    s.net_rx_total = 41 * 1_073_741_824;
    s.net_tx_total = 4 * 1_073_741_824;
    s.net_hist = series(120, 30.0, 32.0, 3.1);
    s.disks = vec![
        Disk { name: "C:".into(), used_gib: 412.0, total_gib: 931.0 },
        Disk { name: "D:".into(), used_gib: 1210.0, total_gib: 1863.0 },
        Disk { name: "E:".into(), used_gib: 2890.0, total_gib: 3726.0 },
        Disk { name: "F:".into(), used_gib: 118.0, total_gib: 476.0 },
    ];

    // ── hot
    s.procs = vec![
        proc("rustc", 38.2, 1450.0, 42e6, 0, 0.0),
        proc("cargo", 6.1, 210.0, 3.1e6, 2, 1.2e5),
        proc("code", 4.8, 2150.0, 0.4e6, 12, 3.2e4),
        proc("chrome", 3.9, 4380.0, 1.8e6, 41, 2.4e6),
        proc("dwm", 2.2, 310.0, 0.0, 0, 0.0),
        proc("obs64", 1.9, 820.0, 12e6, 3, 0.0),
        proc("Discord", 1.1, 640.0, 0.1e6, 9, 8.5e4),
        proc("Spotify", 0.8, 390.0, 0.3e6, 6, 3.1e5),
        proc("explorer", 0.6, 280.0, 2e4, 2, 0.0),
        proc("MsMpEng", 0.5, 260.0, 5.4e6, 1, 0.0),
        proc("steam", 0.4, 350.0, 22e6, 14, 21.6e6),
        proc("svchost", 0.4, 1240.0, 0.2e6, 22, 4e3),
        proc("WindowsTerminal", 0.3, 150.0, 0.0, 0, 0.0),
        proc("dcc", 0.3, 45.0, 0.0, 8, 1.5e4),
    ];
    s.task_count = 312;
    s.thread_count = 4480;
    s.net_etw_active = true;

    // ── spikes: one still running, the rest closed
    s.spikes = vec![
        Spike { hms: ago(2), metric: "cpu".into(), name: "rustc".into(), peak_txt: "97%".into(), dur_secs: 142, ongoing: true },
        Spike { hms: ago(18), metric: "io".into(), name: "obs64".into(), peak_txt: "412.0M/s".into(), dur_secs: 48, ongoing: false },
        Spike { hms: ago(35), metric: "net".into(), name: "steam".into(), peak_txt: "86.4M/s".into(), dur_secs: 312, ongoing: false },
        Spike { hms: ago(61), metric: "cpu".into(), name: "chrome".into(), peak_txt: "63%".into(), dur_secs: 12, ongoing: false },
        Spike { hms: ago(97), metric: "lat".into(), name: "ping".into(), peak_txt: "212ms".into(), dur_secs: 20, ongoing: false },
        Spike { hms: ago(140), metric: "gpu".into(), name: "gpu".into(), peak_txt: "98%".into(), dur_secs: 1320, ongoing: false },
        Spike { hms: ago(205), metric: "mem".into(), name: "hard faults".into(), peak_txt: "1420/s".into(), dur_secs: 6, ongoing: false },
    ];

    // ── saturation & errors
    s.run_queue = 3;
    s.ctx_switches_ps = 48_000.0;
    s.load1 = 3.1;
    s.load5 = 2.6;
    s.load15 = 2.2;
    s.hard_faults_ps = 12.0;
    s.commit_pct = 41.0;
    s.net_errs = 0;
    s.net_errs_ps = 0.0;
    s.disk_read_bps = 38e6;
    s.disk_write_bps = 12e6;
    s.ping_ok = true;
    s.ping_ms = 31.0;
    s.ping_hist = (0..90).map(|i| if i == 61 { 212.0 } else { wave(i, 31.0, 7.0, 0.3) }).collect();

    // ── two-hour timeline: an evening game session shows up in the gpu row
    s.cpu_slow = series(240, 26.0, 18.0, 1.7).into_iter().map(|v| v.min(100.0)).collect();
    s.gpu_slow = (0..240)
        .map(|i| if (178..214).contains(&i) { wave(i, 88.0, 8.0, 0.2).min(99.0) } else { wave(i, 12.0, 8.0, 0.6) })
        .collect();

    // ── git
    s.demo = false;
    s.repos_scanned = 6;
    s.repos = vec![
        Repo { name: "dcc".into(), branch: "main".into(), dirty: 2, ahead: 1, commits_today: 4 },
        Repo { name: "dotfiles".into(), branch: "main".into(), dirty: 0, ahead: 0, commits_today: 1 },
        Repo { name: "blog".into(), branch: "master".into(), dirty: 1, ahead: 0, commits_today: 0 },
    ];
    s.commits = vec![
        Commit { repo: "dcc".into(), hash: "3f9c2a1".into(), secs_ago: 420, msg: "feat: day log + anomaly digest".into() },
        Commit { repo: "dcc".into(), hash: "b81e0d4".into(), secs_ago: 2700, msg: "fix: weather reads the current block".into() },
        Commit { repo: "dotfiles".into(), hash: "a0c77e3".into(), secs_ago: 9000, msg: "chore: terminal font 13 → 12".into() },
        Commit { repo: "blog".into(), hash: "5d12f9b".into(), secs_ago: 86_400, msg: "post: 바탕화면을 대시보드로".into() },
    ];
    // Busier recent weeks, quiet weekends.
    s.grass = (0..26 * 7)
        .map(|i| {
            let (wk, dow) = (i / 7, i % 7);
            let n = ((i * 2_654_435_761usize) >> 9) % 7;
            let n = if dow >= 5 { n / 3 } else { n };
            (n + wk / 9).min(8) as u8 % 5
        })
        .collect();

    // ── claude
    s.claude = Claude {
        ok: true,
        today_tokens: 4_820_000,
        today_cost: 3.91,
        week_tokens: 31_600_000,
        week_cost: 27.40,
        cache_read: 18_200_000,
        cache_write: 1_900_000,
        // Two working weeks with quiet weekends, in millions of tokens.
        sparkline: vec![1.1, 2.8, 3.4, 0.6, 0.4, 2.2, 3.9, 4.1, 2.7, 3.1, 0.9, 0.5, 3.6, 4.8],
        top_model: "opus-4-5".into(),
        sessions: vec![
            ClaudeSession { project: "dcc".into(), model: "opus-4-5".into(), idle_secs: 12, today_tokens: 2_100_000 },
            ClaudeSession { project: "blog".into(), model: "sonnet-4-5".into(), idle_secs: 5400, today_tokens: 640_000 },
            ClaudeSession { project: "dotfiles".into(), model: "haiku-4-5".into(), idle_secs: 14_400, today_tokens: 88_000 },
        ],
    };
    s
}

/// Fill the module-level stores the panels read directly: feeds, clipboard,
/// todo, pomodoro, journal and the day digest.
pub fn seed() {
    crate::feeds::set_demo(Feeds {
        weather: Weather {
            ok: true,
            temp_c: 21.4,
            desc: "partly cloudy".into(),
            wind_ms: 2.1,
            rain_prob: 10.0,
            tmin: 15.0,
            tmax: 24.0,
        },
        ticker: Ticker {
            ok: true,
            fx_code: "KRW".into(),
            fx_rate: 1382.5,
            coin_market: "KRW-BTC".into(),
            coin_price: 142_300_000.0,
            coin_chg_pct: 1.8,
        },
        news: [
            "Show GN: 바탕화면을 터미널 대시보드로 바꿔봤습니다",
            "Why SQLite is everywhere",
            "로컬 LLM으로 코드 리뷰 자동화하기",
            "The quiet return of the status bar",
            "Postgres 18에서 달라진 것들",
            "A tour of ETW for application developers",
        ]
        .iter()
        .map(|t| Headline { title: t.to_string(), url: "https://example.com".into() })
        .collect(),
        cal: vec![
            CalEvent { hm: "09:30".into(), summary: "standup".into() },
            CalEvent { hm: "14:00".into(), summary: "디자인 리뷰".into() },
            CalEvent { hm: "19:30".into(), summary: "gym".into() },
        ],
        cal_configured: true,
        aw: AwDay {
            ok: true,
            active_secs: 6 * 3600 + 12 * 60,
            afk_secs: 3780,
            apps: [("code", 9660u64), ("chrome", 5880), ("windowsterminal", 3120), ("figma", 1860), ("discord", 1080), ("spotify", 720)]
                .iter()
                .map(|(a, s)| AwApp { app: a.to_string(), secs: *s })
                .collect(),
        },
    });

    crate::clipboard::set_demo(&[
        "cargo build --release",
        "https://doc.rust-lang.org/std/collections/struct.HashMap.html",
        "SELECT host, count(*) FROM events GROUP BY 1 ORDER BY 2 DESC;",
        "#35D06B",
        "회의록 링크 공유드립니다",
        "ssh -L 5432:localhost:5432 db-staging",
    ]);

    crate::todo::set_demo(&[
        (false, "README 스크린샷 교체"),
        (false, "palette: 한글 IME 조합"),
        (false, "renew the domain"),
        (true, "backup photos"),
        (true, "install DCC"),
    ]);

    crate::pomo::set_demo(0.62);

    // Journal — the Warn/Err lines also fold into the day digest.
    events::journal(Kind::Ok, "dcc online — grid 260x68 · 16 cores");
    events::journal(Kind::Info, "palette: Ctrl+` registered");
    events::journal(Kind::Ok, "net io: etw tracing active (per-process bytes)");
    events::journal(Kind::Ok, "git: 6 repos scanned");
    events::journal(Kind::Warn, "service stopped — Print Spooler");
    events::journal(Kind::Ok, "download — rustup-init.exe · 9.8 MB");
    events::journal(Kind::Info, "dcc.conf reloaded");
    events::journal(Kind::Ok, "icons herded — 3 moved off the wall's monitor");
    events::journal(Kind::Info, "pomodoro started");
    events::journal(Kind::Warn, "disk warning — event 153 — check the vol panel");

    for (metric, name, peak, txt, dur, mins) in [
        ("cpu", "rustc", 97.0, "97%", 372u64, 30u32),
        ("cpu", "rustc", 91.0, "91%", 120, 12),
        ("gpu", "gpu", 98.0, "98%", 1320, 140),
        ("net", "steam", 86.4e6, "86.4M/s", 312, 35),
        ("io", "obs64", 412e6, "412.0M/s", 48, 18),
        ("lat", "ping", 212.0, "212ms", 20, 97),
    ] {
        crate::daylog::spike(metric, name, peak, txt, dur, &ago(mins));
    }
}
