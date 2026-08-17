//! Latency, the golden signal the resource panels cannot show: one ICMP echo
//! to 1.1.1.1 every ten seconds on its own thread. The collector copies the
//! latest reading into the snapshot; a timeout is a data point too.

use std::sync::Mutex;
use std::time::Duration;

use windows::Win32::NetworkManagement::IpHelper::{IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho};

struct State {
    ms: f64,
    ok: bool,
    hist: Vec<f64>,
    timeouts: u32,
}

static STATE: Mutex<State> = Mutex::new(State { ms: 0.0, ok: false, hist: Vec::new(), timeouts: 0 });

/// (latest ms, reachable, history, consecutive timeouts)
pub fn current() -> (f64, bool, Vec<f64>, u32) {
    match STATE.lock() {
        Ok(s) => (s.ms, s.ok, s.hist.clone(), s.timeouts),
        Err(_) => (0.0, false, Vec::new(), 0),
    }
}

pub fn spawn() {
    std::thread::spawn(|| unsafe {
        let Ok(handle) = IcmpCreateFile() else { return };
        let dest: u32 = u32::from_le_bytes([1, 1, 1, 1]); // 1.1.1.1, network order
        let payload = [0x64u8; 16];
        // Reply buffer: ICMP_ECHO_REPLY + payload + slop, per the docs.
        let mut reply = [0u8; 512];

        loop {
            let n = IcmpSendEcho(
                handle,
                dest,
                payload.as_ptr() as *const _,
                payload.len() as u16,
                None,
                reply.as_mut_ptr() as *mut _,
                reply.len() as u32,
                2000,
            );
            // ICMP_ECHO_REPLY (x64): Address u32 @0, Status u32 @4,
            // RoundTripTime u32 @8.
            let (ok, ms) = if n > 0 {
                let status = u32::from_le_bytes(reply[4..8].try_into().unwrap());
                let rtt = u32::from_le_bytes(reply[8..12].try_into().unwrap());
                (status == 0, rtt as f64)
            } else {
                (false, 0.0)
            };
            if let Ok(mut s) = STATE.lock() {
                s.ok = ok;
                if ok {
                    s.ms = ms;
                    s.timeouts = 0;
                    s.hist.push(ms);
                } else {
                    s.timeouts += 1;
                    // A timeout charts as a spike at the timeout ceiling.
                    s.hist.push(2000.0);
                }
                if s.hist.len() > 90 {
                    let n = s.hist.len() - 90;
                    s.hist.drain(0..n);
                }
                if s.timeouts == 3 {
                    crate::events::post(
                        crate::events::Kind::Err,
                        "network unreachable",
                        "3 consecutive ping timeouts to 1.1.1.1",
                    );
                }
            }
            std::thread::sleep(Duration::from_secs(10));
        }
        #[allow(unreachable_code)]
        {
            let _ = IcmpCloseHandle(handle);
        }
    });
}
