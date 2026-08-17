//! Per-process network bytes, the real thing — an ETW real-time session on
//! Microsoft-Windows-Kernel-Network. Every TCP/UDP send and receive event
//! carries the owning PID and the byte count; summing them is exactly what
//! Task Manager's network column does. Needs elevation: without it the
//! session fails to start, `active()` stays false, and the hot panel falls
//! back to connection counts.
//!
//! The consumer thread parks inside ProcessTrace for the life of the
//! process. A stale session from a previous run is stopped before starting —
//! kernel sessions outlive their creator.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::System::Diagnostics::Etw::*;

const KERNEL_NETWORK: GUID = GUID::from_u128(0x7DD42A49_5329_4832_8DFD_43D979153A88);
const SESSION: &str = "dcc-net-trace";

/// pid -> (bytes sent, bytes received), cumulative since the session began.
static TOTALS: Mutex<Option<HashMap<u32, (u64, u64)>>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn totals() -> HashMap<u32, (u64, u64)> {
    TOTALS.lock().ok().and_then(|t| t.clone()).unwrap_or_default()
}

/// EVENT_TRACE_PROPERTIES needs its session name appended in the same
/// allocation; a raw byte buffer is the honest way to build one.
fn make_props() -> (Vec<u8>, Vec<u16>) {
    let name_w: Vec<u16> = SESSION.encode_utf16().chain(std::iter::once(0)).collect();
    let head = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
    let total = head + name_w.len() * 2;
    let mut buf = vec![0u8; total];
    unsafe {
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        (*p).Wnode.BufferSize = total as u32;
        (*p).Wnode.Flags = 0x0002_0000; // WNODE_FLAG_TRACED_GUID
        (*p).Wnode.ClientContext = 1; // QPC timestamps
        (*p).BufferSize = 64;
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).LoggerNameOffset = head as u32;
        std::ptr::copy_nonoverlapping(name_w.as_ptr() as *const u8, buf.as_mut_ptr().add(head), name_w.len() * 2);
    }
    (buf, name_w)
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let r = &*rec;
    if r.EventHeader.ProviderId != KERNEL_NETWORK {
        return;
    }
    let id = r.EventHeader.EventDescriptor.Id;
    // TCP send/recv v4: 10/11, v6: 26/27. UDP v4: 42/43, v6: 58/59.
    let send = matches!(id, 10 | 26 | 42 | 58);
    let recv = matches!(id, 11 | 27 | 43 | 59);
    if (!send && !recv) || (r.UserDataLength as usize) < 8 {
        return;
    }
    let p = r.UserData as *const u32;
    let pid = *p;
    let size = *p.add(1) as u64;
    if let Ok(mut guard) = TOTALS.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        let e = map.entry(pid).or_insert((0, 0));
        if send {
            e.0 += size;
        } else {
            e.1 += size;
        }
    }
}

pub fn spawn() {
    std::thread::spawn(|| unsafe {
        // Stop any session a crashed previous run left behind.
        let (mut stale, name_w) = make_props();
        let _ = ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            PCWSTR(name_w.as_ptr()),
            stale.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
            EVENT_TRACE_CONTROL_STOP,
        );

        let (mut props, name_w) = make_props();
        let mut handle = CONTROLTRACE_HANDLE::default();
        let rc = StartTraceW(
            &mut handle,
            PCWSTR(name_w.as_ptr()),
            props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
        );
        if rc.is_err() {
            crate::events::journal(
                crate::events::Kind::Warn,
                "net io: etw session failed — running unelevated? using conn counts",
            );
            return;
        }
        let rc = EnableTraceEx2(
            handle,
            &KERNEL_NETWORK,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
            4, // TRACE_LEVEL_INFORMATION
            0,
            0,
            0,
            None,
        );
        if rc.is_err() {
            crate::events::journal(crate::events::Kind::Warn, "net io: provider enable failed");
            return;
        }

        let mut name_mut: Vec<u16> = SESSION.encode_utf16().chain(std::iter::once(0)).collect();
        let mut log = EVENT_TRACE_LOGFILEW::default();
        log.LoggerName = PWSTR(name_mut.as_mut_ptr());
        log.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        log.Anonymous2.EventRecordCallback = Some(on_event);
        let trace = OpenTraceW(&mut log);
        if trace.Value == u64::MAX {
            crate::events::journal(crate::events::Kind::Warn, "net io: opentrace failed");
            return;
        }
        ACTIVE.store(true, Ordering::Relaxed);
        crate::events::journal(crate::events::Kind::Ok, "net io: etw tracing active (per-process bytes)");
        // Blocks for the life of the process, invoking on_event.
        let _ = ProcessTrace(&[trace], None, None);
        ACTIVE.store(false, Ordering::Relaxed);
    });
}
