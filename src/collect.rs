//! Native counters. No child processes, no shelling out, no parsing of
//! human-readable output — every number here comes from a system call.
//!
//! Per-core load and the process table both come from `NtQuerySystemInformation`.
//! It is technically undocumented, but it is what Task Manager itself uses, and
//! one call returns the whole table instead of opening a handle per process.

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::SystemInformation::*;

use crate::state::*;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        buffer: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: u32 = 8;
const SYSTEM_PROCESS_INFORMATION: u32 = 5;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcPerf {
    idle_time: i64,
    kernel_time: i64,
    user_time: i64,
    dpc_time: i64,
    interrupt_time: i64,
    interrupt_count: u32,
    _pad: u32,
}

/// Prefix of SYSTEM_PROCESS_INFORMATION — everything up to the fields we read.
/// The table is a chain of variable-length records linked by `next_offset`.
#[repr(C)]
struct SysProcInfo {
    next_offset: u32,
    number_of_threads: u32,
    _reserved: [i64; 3],
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name_len: u16,
    image_name_max: u16,
    _pad: u32,
    image_name_ptr: *mut u16,
    base_priority: i32,
    _pad2: u32,
    unique_process_id: isize,
    inherited_from: isize,
    handle_count: u32,
    session_id: u32,
    _pad3: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    _pad4: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    // VM_COUNTERS tail, then IO_COUNTERS — the part Task Manager's "I/O"
    // columns come from.
    quota_peak_paged: usize,
    quota_paged: usize,
    quota_peak_nonpaged: usize,
    quota_nonpaged: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_ops: i64,
    write_ops: i64,
    other_ops: i64,
    read_bytes: i64,
    write_bytes: i64,
    other_bytes: i64,
}

fn query(class: u32) -> Option<Vec<u8>> {
    unsafe {
        let mut cap: u32 = 256 * 1024;
        for _ in 0..8 {
            let mut buf = vec![0u8; cap as usize];
            let mut needed: u32 = 0;
            let st = NtQuerySystemInformation(class, buf.as_mut_ptr() as *mut c_void, cap, &mut needed);
            if st == 0 {
                return Some(buf);
            }
            // STATUS_INFO_LENGTH_MISMATCH — grow and retry.
            if st == 0xC0000004u32 as i32 {
                cap = needed.max(cap * 2) + 64 * 1024;
                continue;
            }
            return None;
        }
        None
    }
}

pub struct Collector {
    cores: usize,
    prev_cpu: Vec<ProcPerf>,
    prev_proc: HashMap<isize, i64>, // pid -> kernel+user ticks
    prev_io: HashMap<isize, i64>,   // pid -> read+write+other bytes
    prev_netio: HashMap<u32, u64>,  // pid -> etw sent+recv bytes
    prev_net: Option<(u64, u64, u64)>,
    /// (page reads, context switches, io read bytes, io write bytes).
    prev_perf: Option<(u64, u64, u64, u64)>,
    warn_cpu_tick: u64,
    warn_mem_tick: u64,
    load: (f64, f64, f64),
    last: Instant,
    proc_clock: Instant,
    tick: u64,
    boot_log: Vec<String>,
    /// Per-process CPU history for the hot panel, keyed by image name.
    proc_hist: HashMap<String, Vec<f32>>,
    /// "metric|name" -> in-flight excursion.
    spiking: HashMap<String, Track>,
    /// "metric|name" -> consecutive passes above enter, for metrics that
    /// need sustained pressure before they count (startup page-in bursts
    /// are chatter, not memory pressure).
    spike_pending: HashMap<String, u8>,
    spike_log: Vec<crate::state::Spike>,
}

/// One in-flight excursion of any metric above its enter threshold.
struct Track {
    peak: f64,
    hms: String,
    start_tick: u64,
    exit: f64,
    last: f64,
    seen: u64,
}

fn fmt_metric(metric: &str, v: f64) -> String {
    match metric {
        "cpu" | "gpu" => format!("{:.0}%", v),
        "io" => crate::state::fmt_bps(v),
        "net" => format!("{:.0} Mbit/s", v),
        "lat" => format!("{:.0}ms", v),
        "mem" => format!("{:.0}/s", v),
        _ => format!("{:.0}", v),
    }
}

impl Collector {
    pub fn new() -> Self {
        let cores = unsafe {
            let mut si = SYSTEM_INFO::default();
            GetSystemInfo(&mut si);
            si.dwNumberOfProcessors as usize
        };
        Collector {
            cores,
            prev_cpu: Vec::new(),
            prev_proc: HashMap::new(),
            prev_io: HashMap::new(),
            prev_netio: HashMap::new(),
            prev_net: None,
            prev_perf: None,
            warn_cpu_tick: 0,
            warn_mem_tick: 0,
            load: (0.0, 0.0, 0.0),
            last: Instant::now(),
            proc_clock: Instant::now(),
            tick: 0,
            proc_hist: HashMap::new(),
            spiking: HashMap::new(),
            spike_pending: HashMap::new(),
            spike_log: Vec::new(),
            boot_log: vec![
                "[ok] ntdll     NtQuerySystemInformation ready".into(),
                format!("[ok] cpu       {} logical processors", cores),
                "[ok] collect   0 subprocesses, 0 sockets".into(),
            ],
        }
    }

    /// Per-core load plus the vmstat-style split: user/system/dpc shares and
    /// interrupts per second, all from the same counter deltas.
    fn per_core(&mut self, dt: f64, into: &mut Snapshot) {
        let Some(buf) = query(SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION) else {
            into.cpu_pct = 0.0;
            into.per_core = vec![0.0; self.cores];
            return;
        };
        let n = self.cores;
        let cur: Vec<ProcPerf> = unsafe {
            let p = buf.as_ptr() as *const ProcPerf;
            (0..n).map(|i| *p.add(i)).collect()
        };

        let mut out = vec![0.0f64; n];
        let (mut s_idle, mut s_kern, mut s_user, mut s_dpc, mut s_intr) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut intr_count = 0u64;
        if self.prev_cpu.len() == n {
            for i in 0..n {
                let d_idle = (cur[i].idle_time - self.prev_cpu[i].idle_time) as f64;
                let d_kern = (cur[i].kernel_time - self.prev_cpu[i].kernel_time) as f64;
                let d_user = (cur[i].user_time - self.prev_cpu[i].user_time) as f64;
                s_idle += d_idle;
                s_kern += d_kern;
                s_user += d_user;
                s_dpc += (cur[i].dpc_time - self.prev_cpu[i].dpc_time) as f64;
                s_intr += (cur[i].interrupt_time - self.prev_cpu[i].interrupt_time) as f64;
                intr_count += cur[i].interrupt_count.wrapping_sub(self.prev_cpu[i].interrupt_count) as u64;
                // kernel_time already includes idle_time on Windows.
                let total = d_kern + d_user;
                out[i] = if total > 0.0 {
                    ((1.0 - d_idle / total) * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
            }
            let total = (s_kern + s_user).max(1.0);
            into.cpu_user_pct = (s_user / total * 100.0).clamp(0.0, 100.0);
            // dpc and interrupt time are subsets of kernel time.
            into.cpu_sys_pct = ((s_kern - s_idle) / total * 100.0).clamp(0.0, 100.0);
            into.cpu_dpc_pct = ((s_dpc + s_intr) / total * 100.0).clamp(0.0, 100.0);
            if dt > 0.0 {
                into.intr_ps = intr_count as f64 / dt;
            }
        }
        self.prev_cpu = cur;
        let avg = if out.is_empty() { 0.0 } else { out.iter().sum::<f64>() / out.len() as f64 };
        into.cpu_pct = avg;
        into.per_core = out;
    }
}

impl Collector {
    fn processes(&mut self, dt: f64) -> (Vec<Proc>, usize, usize, u32) {
        let Some(buf) = query(SYSTEM_PROCESS_INFORMATION) else {
            return (Vec::new(), 0, 0, 0);
        };

        let mut agg: HashMap<String, Proc> = HashMap::new();
        let mut next: HashMap<isize, i64> = HashMap::new();
        let mut next_io: HashMap<isize, i64> = HashMap::new();
        let conns = tcp_conns_by_pid();
        let netio = if crate::net_etw::active() { crate::net_etw::totals() } else { HashMap::new() };
        let mut count = 0usize;
        let mut threads = 0usize;
        let mut ready = 0u32;

        unsafe {
            let mut off = 0usize;
            loop {
                let p = buf.as_ptr().add(off) as *const SysProcInfo;
                let rec = &*p;

                let name = if rec.image_name_ptr.is_null() || rec.image_name_len == 0 {
                    if rec.unique_process_id == 0 { "Idle".to_string() } else { "System".to_string() }
                } else {
                    let len = (rec.image_name_len / 2) as usize;
                    let slice = std::slice::from_raw_parts(rec.image_name_ptr, len);
                    String::from_utf16_lossy(slice)
                        .trim_end_matches(".exe")
                        .to_string()
                };

                let ticks = rec.kernel_time + rec.user_time;
                let pid = rec.unique_process_id;
                next.insert(pid, ticks);
                let delta = self.prev_proc.get(&pid).map(|p| ticks - p).unwrap_or(0).max(0) as f64;
                // 100ns ticks over wall-clock seconds, normalised by core count.
                let cpu = if dt > 0.0 {
                    (delta / 1e7 / dt / self.cores as f64 * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };

                let io_total = rec.read_bytes.max(0) + rec.write_bytes.max(0) + rec.other_bytes.max(0);
                next_io.insert(pid, io_total);
                let io_delta = self.prev_io.get(&pid).map(|p| io_total - p).unwrap_or(0).max(0) as f64;
                let io_bps = if dt > 0.0 { io_delta / dt } else { 0.0 };

                count += 1;
                threads += rec.number_of_threads as usize;

                // The process record is followed by one SYSTEM_THREAD_INFORMATION
                // per thread (80 bytes on x64, state at +68). Threads in Ready
                // or DeferredReady are the run queue — work that wants a core
                // and hasn't got one. This is the saturation number that a
                // utilisation percentage cannot show.
                let rec_size = std::mem::size_of::<SysProcInfo>();
                let limit = if rec.next_offset != 0 { off + rec.next_offset as usize } else { buf.len() };
                for ti in 0..rec.number_of_threads as usize {
                    let toff = off + rec_size + ti * 80;
                    if toff + 80 > limit {
                        break;
                    }
                    let state = u32::from_le_bytes(buf[toff + 68..toff + 72].try_into().unwrap());
                    if state == 1 || state == 7 {
                        ready += 1;
                    }
                }

                let e = agg.entry(name.clone()).or_insert_with(|| Proc {
                    name,
                    ..Default::default()
                });
                e.count += 1;
                e.cpu_pct += cpu;
                e.mem_mib += rec.working_set_size as f64 / 1048576.0;
                e.io_bps += io_bps;
                e.net_conns += conns.get(&(pid as u32)).copied().unwrap_or(0);
                if let Some((sent, recv)) = netio.get(&(pid as u32)) {
                    let total = sent + recv;
                    let prev = self.prev_netio.insert(pid as u32, total).unwrap_or(total);
                    if dt > 0.0 {
                        e.net_bps += total.saturating_sub(prev) as f64 / dt;
                    }
                }

                if rec.next_offset == 0 {
                    break;
                }
                off += rec.next_offset as usize;
            }
        }

        self.prev_proc = next;
        self.prev_io = next_io;
        let mut list: Vec<Proc> = agg.into_values().filter(|p| p.name != "Idle").collect();
        list.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap().then(b.mem_mib.partial_cmp(&a.mem_mib).unwrap()));

        // Per-process CPU history for the hottest names, so the hot panel can
        // show each one's last couple of minutes, not just this second.
        let keep: Vec<String> = list.iter().take(12).map(|p| p.name.clone()).collect();
        for p in list.iter().take(12) {
            let ring = self.proc_hist.entry(p.name.clone()).or_default();
            ring.push(p.cpu_pct as f32);
            if ring.len() > 60 {
                let n = ring.len() - 60;
                ring.drain(0..n);
            }
        }
        self.proc_hist.retain(|k, _| keep.contains(k));
        for p in list.iter_mut().take(12) {
            if let Some(r) = self.proc_hist.get(&p.name) {
                p.hist = r.clone();
            }
        }

        // Per-process excursions: cpu bursts and IO bursts feed the same
        // spike log as the system-level metrics.
        for p in list.iter().take(20) {
            self.spike_track("cpu", &p.name.clone(), p.cpu_pct, 50.0, 35.0);
            self.spike_track("io", &p.name.clone(), p.io_bps, 200e6, 100e6);
            self.spike_track("net", &p.name.clone(), p.net_bps, 25e6, 10e6);
        }

        (list, count, threads, ready)
    }

    /// Hysteresis tracker for any metric: enter at `enter`, leave below
    /// `exit`, one record per excursion. A game that sits at 60% CPU for
    /// three hours is one "ongoing" line while it runs and one three-hour
    /// record when it stops — sustained load is the entry's duration, not a
    /// flood of entries.
    fn spike_track(&mut self, metric: &str, name: &str, value: f64, enter: f64, exit: f64) {
        self.spike_track_sustained(metric, name, value, enter, exit, 1);
    }

    fn spike_track_sustained(&mut self, metric: &str, name: &str, value: f64, enter: f64, exit: f64, sustain: u8) {
        let key = format!("{}|{}", metric, name);
        if let Some(tr) = self.spiking.get_mut(&key) {
            tr.last = value;
            tr.seen = self.tick;
            if value > tr.peak {
                tr.peak = value;
            }
            return;
        }
        if value < enter {
            self.spike_pending.remove(&key);
            return;
        }
        let hits = self.spike_pending.entry(key.clone()).or_insert(0);
        *hits += 1;
        if *hits < sustain {
            return;
        }
        self.spike_pending.remove(&key);
        let (_, _, _, hh, mi, ss) = local_time();
        self.spiking.insert(
            key,
            Track {
                peak: value,
                hms: format!("{:02}:{:02}:{:02}", hh, mi, ss),
                start_tick: self.tick,
                exit,
                last: value,
                seen: self.tick,
            },
        );
    }

    /// Close out excursions that fell below their exit threshold or whose
    /// subject vanished (a process that exited mid-spike still gets logged).
    fn spike_sweep(&mut self) {
        let tick = self.tick;
        let done: Vec<String> = self
            .spiking
            .iter()
            .filter(|(_, tr)| tick.saturating_sub(tr.seen) > 3 || tr.last < tr.exit)
            .map(|(k, _)| k.clone())
            .collect();
        for key in done {
            if let Some(tr) = self.spiking.remove(&key) {
                let (metric, name) = key.split_once('|').unwrap_or(("?", &key));
                self.spike_log.push(crate::state::Spike {
                    hms: tr.hms,
                    metric: metric.to_string(),
                    name: name.to_string(),
                    peak_txt: fmt_metric(metric, tr.peak),
                    dur_secs: tick.saturating_sub(tr.start_tick),
                    ongoing: false,
                });
                if self.spike_log.len() > 40 {
                    self.spike_log.remove(0);
                }
            }
        }
    }

    /// SystemPerformanceInformation, read as a flat u32 array — the struct is
    /// eighty consecutive counters and only four matter here. Offsets are the
    /// documented x64 layout: committed pages @12, commit limit @13, page
    /// reads (hard faults) @20, context switches @74.
    fn perf(&mut self, dt: f64, into: &mut Snapshot) {
        const SYSTEM_PERFORMANCE_INFORMATION: u32 = 2;
        let Some(buf) = query(SYSTEM_PERFORMANCE_INFORMATION) else { return };
        if buf.len() < 320 {
            return;
        }
        let u = |i: usize| u32::from_le_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap()) as u64;
        let committed = u(12);
        let limit = u(13);
        if limit > 0 {
            into.commit_pct = committed as f64 / limit as f64 * 100.0;
        }
        let page_reads = u(20);
        let ctx = u(74);
        // The first three LARGE_INTEGERs after IdleProcessTime are the
        // system-wide IO transfer totals — vmstat's bi/bo.
        let io_r = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let io_w = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        if let Some((pr, pc, pir, piw)) = self.prev_perf {
            if dt > 0.0 {
                into.hard_faults_ps = page_reads.saturating_sub(pr) as f64 / dt;
                into.ctx_switches_ps = ctx.saturating_sub(pc) as f64 / dt;
                into.disk_read_bps = io_r.saturating_sub(pir) as f64 / dt;
                into.disk_write_bps = io_w.saturating_sub(piw) as f64 / dt;
            }
        }
        self.prev_perf = Some((page_reads, ctx, io_r, io_w));
    }

    fn spikes_snapshot(&self) -> Vec<crate::state::Spike> {
        let mut out: Vec<crate::state::Spike> = self
            .spiking
            .iter()
            .map(|(key, tr)| {
                let (metric, name) = key.split_once('|').unwrap_or(("?", key));
                crate::state::Spike {
                    hms: tr.hms.clone(),
                    metric: metric.to_string(),
                    name: name.to_string(),
                    peak_txt: fmt_metric(metric, tr.peak),
                    dur_secs: self.tick.saturating_sub(tr.start_tick),
                    ongoing: true,
                }
            })
            .collect();
        out.sort_by(|a, b| b.dur_secs.cmp(&a.dur_secs));
        out.extend(self.spike_log.iter().rev().take(20).cloned());
        out
    }
}

fn memory() -> (f64, f64) {
    unsafe {
        let mut ms = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut ms).is_ok() {
            let total = ms.ullTotalPhys as f64 / 1073741824.0;
            let avail = ms.ullAvailPhys as f64 / 1073741824.0;
            (total - avail, total)
        } else {
            (0.0, 0.0)
        }
    }
}

fn volumes() -> Vec<Disk> {
    let mut out = Vec::new();
    unsafe {
        let mut buf = [0u16; 512];
        let n = GetLogicalDriveStringsW(Some(&mut buf)) as usize;
        if n == 0 {
            return out;
        }
        for part in buf[..n].split(|c| *c == 0) {
            if part.is_empty() {
                continue;
            }
            let root: Vec<u16> = part.iter().copied().chain(std::iter::once(0)).collect();
            const DRIVE_FIXED: u32 = 3;
            if GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) != DRIVE_FIXED {
                continue;
            }
            let mut free = 0u64;
            let mut total = 0u64;
            if GetDiskFreeSpaceExW(
                windows::core::PCWSTR(root.as_ptr()),
                None,
                Some(&mut total),
                Some(&mut free),
            )
            .is_ok()
            {
                let name = String::from_utf16_lossy(part).trim_end_matches('\\').to_string();
                out.push(Disk {
                    name,
                    used_gib: (total - free) as f64 / 1073741824.0,
                    total_gib: total as f64 / 1073741824.0,
                });
            }
        }
    }
    out
}

/// Established TCP connections per owning PID, v4 and v6. Parsed from the
/// raw table bytes — the rows are plain fixed-stride structs.
fn tcp_conns_by_pid() -> HashMap<u32, u32> {
    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;
    const TCP_TABLE_OWNER_PID_ALL: TCP_TABLE_CLASS = TCP_TABLE_CLASS(5);
    const MIB_TCP_STATE_ESTAB: u32 = 5;

    let mut out: HashMap<u32, u32> = HashMap::new();
    unsafe {
        for af in [AF_INET, AF_INET6] {
            let mut size = 0u32;
            let _ = GetExtendedTcpTable(None, &mut size, false, af, TCP_TABLE_OWNER_PID_ALL, 0);
            if size == 0 {
                continue;
            }
            let mut buf = vec![0u8; size as usize + 256];
            let mut cap = buf.len() as u32;
            if GetExtendedTcpTable(
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut cap,
                false,
                af,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            ) != 0
            {
                continue;
            }
            let count = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
            // v4 row: state,laddr,lport,raddr,rport,pid — 24 bytes, state @0,
            // pid @20. v6 row: 56 bytes, state @48, pid @52.
            let (stride, state_off, pid_off) = if af == AF_INET { (24, 0, 20) } else { (56, 48, 52) };
            for i in 0..count {
                let base = 4 + i * stride;
                if base + stride > buf.len() {
                    break;
                }
                let state = u32::from_le_bytes(buf[base + state_off..base + state_off + 4].try_into().unwrap());
                if state != MIB_TCP_STATE_ESTAB {
                    continue;
                }
                let pid = u32::from_le_bytes(buf[base + pid_off..base + pid_off + 4].try_into().unwrap());
                *out.entry(pid).or_insert(0) += 1;
            }
        }
    }
    out
}

/// Cumulative (rx bytes, tx bytes, errors+discards) over every physical,
/// connected interface. The error sum is the E in USE, and it comes free
/// with the same table read.
fn net_totals() -> (u64, u64, u64) {
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != NO_ERROR {
            return (0, 0, 0);
        }
        let mut rx = 0u64;
        let mut tx = 0u64;
        let mut errs = 0u64;
        let t = &*table;
        let rows = std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize);
        for r in rows {
            let virtual_or_loopback = r.InterfaceAndOperStatusFlags._bitfield & 0x1 == 0
                || r.Type == 24; // IF_TYPE_SOFTWARE_LOOPBACK
            if virtual_or_loopback {
                continue;
            }
            // NET_IF_MEDIA_CONNECT_STATE::MediaConnectStateConnected
            if r.MediaConnectState.0 != 1 {
                continue;
            }
            rx += r.InOctets;
            tx += r.OutOctets;
            errs += r.InErrors + r.OutErrors + r.InDiscards + r.OutDiscards;
        }
        FreeMibTable(table as *const _);
        (rx, tx, errs)
    }
}

fn hostname() -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        if GetComputerNameExW(ComputerNameDnsHostname, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len).is_ok() {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            "localhost".into()
        }
    }
}

pub fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    unsafe {
        let st = GetLocalTime();
        (st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond)
    }
}

impl Collector {
    /// One full pass. Everything below is a handful of system calls; the whole
    /// thing lands well under two milliseconds on a modern machine.
    pub fn sample(&mut self, into: &mut Snapshot) {
        let t0 = Instant::now();
        let dt = self.last.elapsed().as_secs_f64().max(0.001);
        self.last = Instant::now();

        self.per_core(dt, into);
        into.cores = self.cores;

        let (used, total) = memory();
        into.mem_used_gib = used;
        into.mem_total_gib = total;

        // Tiered intervals: load moves every second, the process table every
        // other second, and volume sizes barely at all. Polling them all at the
        // fastest rate is most of what a monitor like this wastes.
        self.tick = self.tick.wrapping_add(1);
        if self.tick % 2 == 0 || into.procs.is_empty() {
            let (procs, count, threads, ready) = self.processes(self.proc_clock.elapsed().as_secs_f64().max(0.001));
            self.proc_clock = Instant::now();
            into.task_count = count;
            into.thread_count = threads;
            into.procs = procs;
            into.run_queue = ready;
        }
        self.perf(dt, into);
        if self.tick % 30 == 0 || into.disks.is_empty() {
            into.disks = volumes();
        }

        crate::gpu::sample(&mut into.gpu);
        push_hist(&mut into.gpu.hist, into.gpu.util_pct, 120);

        let (rx, tx, errs) = net_totals();
        if let Some((prx, ptx, perr)) = self.prev_net {
            into.net_down_mbps = (rx.saturating_sub(prx) as f64 * 8.0) / dt / 1e6;
            into.net_up_mbps = (tx.saturating_sub(ptx) as f64 * 8.0) / dt / 1e6;
            into.net_errs_ps = errs.saturating_sub(perr) as f64 / dt;
        }
        self.prev_net = Some((rx, tx, errs));
        into.net_rx_total = rx;
        into.net_tx_total = tx;
        into.net_errs = errs;

        into.net_etw_active = crate::net_etw::active();
        into.claude = crate::claude::current();

        let (ping_ms, ping_ok, ping_hist, _timeouts) = crate::ping::current();
        into.ping_ms = ping_ms;
        into.ping_ok = ping_ok;
        into.ping_hist = ping_hist;

        // Saturation alarms, with a two-minute cooldown so a busy spell is
        // one journal line, not a flood.
        if into.run_queue > self.cores as u32 * 2 && self.tick.saturating_sub(self.warn_cpu_tick) > 120 {
            self.warn_cpu_tick = self.tick;
            crate::events::post(
                crate::events::Kind::Warn,
                "cpu saturated",
                &format!("run queue {} on {} cores", into.run_queue, self.cores),
            );
        }
        // Only alarm when the sustained tracker agrees this is a real
        // excursion (4s above 1000/s) — asset-streaming blips during game
        // loads read as pressure to a raw threshold but aren't.
        if self.spiking.contains_key("mem|hard faults")
            && self.tick.saturating_sub(self.warn_mem_tick) > 300
        {
            self.warn_mem_tick = self.tick;
            crate::events::post(
                crate::events::Kind::Warn,
                "memory pressure",
                &format!("{:.0} hard faults/s · commit {:.0}%", into.hard_faults_ps, into.commit_pct),
            );
        }

        if into.host.is_empty() {
            into.host = hostname();
            into.os = format!("Windows {}", os_build());
            into.cpu_model = cpu_brand();
            into.log = self.boot_log.clone();
        }
        into.uptime_secs = unsafe { GetTickCount64() } / 1000;
        push_hist(&mut into.cpu_hist, into.cpu_pct, 120);
        push_hist(&mut into.net_hist, into.net_down_mbps, 120);
        let mem_pct = if into.mem_total_gib > 0.0 {
            into.mem_used_gib / into.mem_total_gib * 100.0
        } else {
            0.0
        };
        push_hist(&mut into.mem_hist, mem_pct, 120);
        // The slow rings: one point per 30s, 240 deep — two hours of context
        // for "what happened while I was away".
        if self.tick % 30 == 0 {
            push_hist(&mut into.cpu_slow, into.cpu_pct, 240);
            push_hist(&mut into.gpu_slow, into.gpu.util_pct, 240);
            push_hist(&mut into.net_slow, into.net_down_mbps, 240);
        }
        // System-level excursions join the same spike log as processes.
        self.spike_track_sustained("gpu", "gpu", into.gpu.util_pct, 85.0, 60.0, 2);
        self.spike_track("net", "download", into.net_down_mbps, 250.0, 120.0);
        // Sustained only: process launches produce page-in bursts that are
        // noise, real thrashing holds the rate for several seconds.
        self.spike_track_sustained("mem", "hard faults", into.hard_faults_ps, 1000.0, 300.0, 4);
        if !into.ping_hist.is_empty() {
            let lat = if into.ping_ok { into.ping_ms } else { 2000.0 };
            self.spike_track("lat", "ping", lat, 200.0, 100.0);
        }
        self.spike_sweep();

        // Unix-style load averages, synthesised: instantaneous load is the
        // run queue plus the currently-executing share of the cores.
        let inst = into.run_queue as f64 + into.cpu_pct / 100.0 * self.cores as f64;
        let ema = |cur: f64, tau: f64| {
            let a = (-dt / tau).exp();
            cur * a + inst * (1.0 - a)
        };
        self.load = (ema(self.load.0, 60.0), ema(self.load.1, 300.0), ema(self.load.2, 900.0));
        into.load1 = self.load.0;
        into.load5 = self.load.1;
        into.load15 = self.load.2;

        into.spikes = self.spikes_snapshot();
        into.collect_micros = t0.elapsed().as_micros() as u64;
    }
}

#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
}

/// GetVersionEx lies to unmanifested processes; RtlGetVersion does not.
fn os_build() -> String {
    unsafe {
        let mut v: OsVersionInfoW = std::mem::zeroed();
        v.size = std::mem::size_of::<OsVersionInfoW>() as u32;
        if RtlGetVersion(&mut v) != 0 {
            return "unknown".into();
        }
        let name = if v.build >= 22000 { "11" } else { "10" };
        format!("{} {}", name, v.build)
    }
}

/// CPU brand string straight from CPUID leaves 0x80000002..0x80000004.
fn cpu_brand() -> String {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::__cpuid;
        if __cpuid(0x8000_0000).eax < 0x8000_0004 {
            return "unknown cpu".into();
        }
        let mut bytes = Vec::with_capacity(48);
        for leaf in 0x8000_0002u32..=0x8000_0004 {
            let r = __cpuid(leaf);
            for v in [r.eax, r.ebx, r.ecx, r.edx] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let s = String::from_utf8_lossy(&bytes);
        return s.trim_end_matches('\0').trim().replace("  ", " ");
    }
    #[allow(unreachable_code)]
    "unknown cpu".into()
}
