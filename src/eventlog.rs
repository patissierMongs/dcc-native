//! The Windows event log, curated. Not a firehose — a short allowlist of
//! event IDs that mean "your machine just had a moment": app crashes,
//! service deaths, disk complaints, unexpected reboots, Defender detections.
//! EvtSubscribe delivers them push-style on a system thread; each becomes a
//! toast + journal line with the interesting fields pulled from the XML.

use windows::core::*;
use windows::Win32::System::EventLog::*;

use crate::events;

fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let a = xml.find(&open)? + open.len();
    let b = xml[a..].find(&close)? + a;
    Some(xml[a..b].trim().to_string())
}

/// First few <Data> payload values, which is where the useful names live.
fn data_values(xml: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while out.len() < max {
        let Some(open) = rest.find("<Data") else { break };
        let after = &rest[open..];
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find("</Data>") else { break };
        let v = body[..end].trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
        rest = &body[end + 7..];
    }
    out
}

unsafe extern "system" fn on_event(
    action: EVT_SUBSCRIBE_NOTIFY_ACTION,
    _ctx: *const core::ffi::c_void,
    handle: EVT_HANDLE,
) -> u32 {
    if action != EvtSubscribeActionDeliver {
        return 0;
    }
    let mut used = 0u32;
    let mut props = 0u32;
    let _ = EvtRender(None, handle, EvtRenderEventXml.0, 0, None, &mut used, &mut props);
    if used == 0 {
        return 0;
    }
    let mut buf = vec![0u16; used as usize / 2 + 2];
    if EvtRender(
        None,
        handle,
        EvtRenderEventXml.0,
        (buf.len() * 2) as u32,
        Some(buf.as_mut_ptr() as *mut _),
        &mut used,
        &mut props,
    )
    .is_err()
    {
        return 0;
    }
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    let xml = String::from_utf16_lossy(&buf[..end]);

    let id: u32 = tag_text(&xml, "EventID").and_then(|s| s.parse().ok()).unwrap_or(0);
    let data = data_values(&xml, 2);
    let first = data.first().cloned().unwrap_or_default();

    let (kind, title, body) = match id {
        41 => (events::Kind::Err, "unexpected reboot", "kernel-power 41 — the machine lost power or crashed".to_string()),
        6008 => (events::Kind::Err, "unclean shutdown", "eventlog 6008".to_string()),
        7031 | 7034 => (events::Kind::Warn, "service crashed", first),
        7 | 51 | 153 => (events::Kind::Warn, "disk warning", format!("event {} — check the vol panel", id)),
        1000 => (events::Kind::Warn, "app crashed", first),
        1002 => (events::Kind::Warn, "app hang", first),
        1116 | 1117 => (events::Kind::Err, "defender detection", first),
        _ => return 0,
    };
    events::post(kind, title, &body);
    0
}

fn subscribe(channel: PCWSTR, query: PCWSTR) {
    unsafe {
        if let Ok(h) = EvtSubscribe(
            None,
            None,
            channel,
            query,
            None,
            None,
            Some(on_event),
            EvtSubscribeToFutureEvents.0,
        ) {
            // Never closed: the subscription lives as long as the wallpaper.
            let _ = h;
        }
    }
}

pub fn start() {
    subscribe(
        w!("System"),
        w!("*[System[EventID=41 or EventID=6008 or EventID=7031 or EventID=7034 or EventID=7 or EventID=51 or EventID=153]]"),
    );
    subscribe(w!("Application"), w!("*[System[EventID=1000 or EventID=1002]]"));
    subscribe(
        w!("Microsoft-Windows-Windows Defender/Operational"),
        w!("*[System[EventID=1116 or EventID=1117]]"),
    );
    events::journal(events::Kind::Info, "event log: watching crashes · services · disk · defender");
}
