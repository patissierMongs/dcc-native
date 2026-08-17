//! GPU metrics from nvml.dll, loaded at runtime.
//!
//! The NVIDIA driver installs nvml.dll into System32 on every machine it
//! touches, so linking the SDK — or pulling a wrapper crate — buys nothing.
//! Five function pointers cover the whole panel. A machine without the DLL
//! (or without an NVIDIA card) just reports `ok = false` and the panel says
//! so instead of the process failing to start.

use std::ffi::c_void;
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

#[repr(C)]
struct Utilization {
    gpu: u32,
    mem: u32,
}

#[repr(C)]
struct MemoryInfo {
    total: u64,
    free: u64,
    used: u64,
}

type FnInit = unsafe extern "C" fn() -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
type FnName = unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> i32;
type FnUtil = unsafe extern "C" fn(*mut c_void, *mut Utilization) -> i32;
type FnMem = unsafe extern "C" fn(*mut c_void, *mut MemoryInfo) -> i32;
type FnTemp = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
type FnPower = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;

struct Api {
    device: *mut c_void,
    name: String,
    util: FnUtil,
    mem: FnMem,
    temp: FnTemp,
    power: FnPower,
}

// NVML is documented thread-safe; the raw pointers are opaque driver handles.
unsafe impl Send for Api {}

/// None = not tried yet; Some(None) = tried and unavailable.
static API: Mutex<Option<Option<Api>>> = Mutex::new(None);

unsafe fn load() -> Option<Api> {
    let lib = LoadLibraryW(w!("nvml.dll")).ok()?;
    macro_rules! sym {
        ($name:literal, $ty:ty) => {
            std::mem::transmute::<_, $ty>(GetProcAddress(lib, s!($name))?)
        };
    }
    let init: FnInit = sym!("nvmlInit_v2", FnInit);
    let handle: FnHandle = sym!("nvmlDeviceGetHandleByIndex_v2", FnHandle);
    let get_name: FnName = sym!("nvmlDeviceGetName", FnName);
    let util: FnUtil = sym!("nvmlDeviceGetUtilizationRates", FnUtil);
    let mem: FnMem = sym!("nvmlDeviceGetMemoryInfo", FnMem);
    let temp: FnTemp = sym!("nvmlDeviceGetTemperature", FnTemp);
    let power: FnPower = sym!("nvmlDeviceGetPowerUsage", FnPower);

    if init() != 0 {
        return None;
    }
    let mut device: *mut c_void = std::ptr::null_mut();
    if handle(0, &mut device) != 0 {
        return None;
    }
    let mut buf = [0u8; 96];
    let name = if get_name(device, buf.as_mut_ptr(), buf.len() as u32) == 0 {
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).replace("NVIDIA ", "")
    } else {
        "gpu".into()
    };
    Some(Api { device, name, util, mem, temp, power })
}

/// Fill `into` with a fresh reading. Cheap enough to call every second.
pub fn sample(into: &mut crate::state::Gpu) {
    let mut guard = match API.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if guard.is_none() {
        *guard = Some(unsafe { load() });
    }
    let Some(Some(api)) = guard.as_ref() else {
        into.ok = false;
        return;
    };

    into.ok = true;
    into.name = api.name.clone();
    unsafe {
        let mut u = Utilization { gpu: 0, mem: 0 };
        if (api.util)(api.device, &mut u) == 0 {
            into.util_pct = u.gpu as f64;
        }
        let mut m = MemoryInfo { total: 0, free: 0, used: 0 };
        if (api.mem)(api.device, &mut m) == 0 && m.total > 0 {
            into.mem_used_gib = m.used as f64 / 1073741824.0;
            into.mem_total_gib = m.total as f64 / 1073741824.0;
        }
        let mut t = 0u32;
        // 0 = NVML_TEMPERATURE_GPU
        if (api.temp)(api.device, 0, &mut t) == 0 {
            into.temp_c = t;
        }
        let mut p = 0u32;
        if (api.power)(api.device, &mut p) == 0 {
            into.power_w = p as f64 / 1000.0;
        }
    }
}
