//! One function: HTTPS GET a small text body, via WinHTTP.
//!
//! Everything the feed worker eats is a few kilobytes of JSON, XML or ICS, so
//! there is no client abstraction here — open, connect, request, drain, close.
//! Failures return None and the caller shows stale data; a wallpaper never
//! gets to block or complain loudly about the network.

use windows::core::*;
use windows::Win32::Networking::WinHttp::*;

const MAX_BODY: usize = 1024 * 1024;

/// Split "https://host/path" into (host, path). Only https, only port 443 —
/// every endpoint we talk to is https.
fn split(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    match rest.split_once('/') {
        Some((host, path)) => Some((host.to_string(), format!("/{}", path))),
        None => Some((rest.to_string(), "/".to_string())),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn get(url: &str) -> Option<String> {
    let (host, path) = split(url)?;
    unsafe {
        let session = WinHttpOpen(
            w!("dcc-wallpaper/0.1"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return None;
        }
        let out = (|| {
            let _ = WinHttpSetTimeouts(session, 5000, 5000, 5000, 10000);
            let host_w = wide(&host);
            let conn = WinHttpConnect(session, PCWSTR(host_w.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0);
            if conn.is_null() {
                return None;
            }
            let body = (|| {
                let path_w = wide(&path);
                let req = WinHttpOpenRequest(
                    conn,
                    w!("GET"),
                    PCWSTR(path_w.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null_mut(),
                    WINHTTP_FLAG_SECURE,
                );
                if req.is_null() {
                    return None;
                }
                let body = (|| {
                    WinHttpSendRequest(req, None, None, 0, 0, 0).ok()?;
                    WinHttpReceiveResponse(req, std::ptr::null_mut()).ok()?;
                    let mut out: Vec<u8> = Vec::new();
                    loop {
                        let mut avail = 0u32;
                        WinHttpQueryDataAvailable(req, &mut avail).ok()?;
                        if avail == 0 {
                            break;
                        }
                        let take = (avail as usize).min(MAX_BODY - out.len());
                        if take == 0 {
                            break;
                        }
                        let start = out.len();
                        out.resize(start + take, 0);
                        let mut read = 0u32;
                        WinHttpReadData(req, out[start..].as_mut_ptr() as *mut _, take as u32, &mut read)
                            .ok()?;
                        out.truncate(start + read as usize);
                        if read == 0 {
                            break;
                        }
                    }
                    Some(String::from_utf8_lossy(&out).into_owned())
                })();
                let _ = WinHttpCloseHandle(req);
                body
            })();
            let _ = WinHttpCloseHandle(conn);
            body
        })();
        let _ = WinHttpCloseHandle(session);
        out
    }
}
