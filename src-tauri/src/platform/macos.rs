use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::time::{SystemTime, UNIX_EPOCH};

use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};

use super::{Pressure, Thermal};

pub fn frontmost_pid() -> Option<u32> {
    // Called from a background thread every few seconds: the pool stops
    // autoreleased Objective-C objects from piling up over days.
    autoreleasepool(|_| {
        let ws = NSWorkspace::sharedWorkspace();
        let app = ws.frontmostApplication()?;
        let pid = app.processIdentifier();
        (pid > 0).then_some(pid as u32)
    })
}

pub fn memory_pressure() -> Pressure {
    // kern.memorystatus_vm_pressure_level: 1 normal, 2 warn, 4 critical.
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let name = c"kern.memorystatus_vm_pressure_level";
    // SAFETY: `level` and `size` are valid for writes and sized for a c_int,
    // and no new value is passed (null, 0).
    let rc = unsafe {
        libc::sysctlbyname(name.as_ptr(), (&raw mut level).cast(), &mut size, std::ptr::null_mut(), 0)
    };
    match (rc, level) {
        (0, 1) => Pressure::Normal,
        (0, 2) => Pressure::Warn,
        (0, 4) => Pressure::Critical,
        _ => Pressure::Unknown,
    }
}

pub fn thermal_state() -> Thermal {
    autoreleasepool(|_| match NSProcessInfo::processInfo().thermalState() {
        NSProcessInfoThermalState::Nominal => Thermal::Nominal,
        NSProcessInfoThermalState::Fair => Thermal::Fair,
        NSProcessInfoThermalState::Serious => Thermal::Serious,
        NSProcessInfoThermalState::Critical => Thermal::Critical,
        _ => Thermal::Unknown,
    })
}

/// Controlling terminal device of a process (NODEV when it has none).
fn tty_dev(pid: u32) -> Option<u32> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a properly sized, writable proc_bsdinfo; the kernel
    // writes at most `size` bytes and returns how many it wrote.
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size && info.e_tdev != u32::MAX && info.e_tdev != 0).then_some(info.e_tdev)
}

pub fn terminal_idle(pids: &[u32]) -> HashMap<u32, u64> {
    let mut wanted: HashMap<u32, Vec<u32>> = HashMap::new();
    for &pid in pids {
        if let Some(dev) = tty_dev(pid) {
            wanted.entry(dev).or_default().push(pid);
        }
    }
    let mut out = HashMap::new();
    if wanted.is_empty() {
        return out;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let Ok(dir) = std::fs::read_dir("/dev") else { return out };
    for entry in dir.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("ttys") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Some(owners) = wanted.get(&(meta.rdev() as u32)) else { continue };
        // atime: last input read by the program; mtime: last output written.
        let last = meta.atime().max(meta.mtime());
        let idle = (now - last).max(0) as u64;
        for pid in owners {
            out.insert(*pid, idle);
        }
    }
    out
}

pub fn footprint(pid: u32) -> Option<u64> {
    let raw = libc::pid_t::try_from(pid).ok()?;
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a properly sized, writable rusage_info_v2 and the
    // flavor asks for exactly that struct.
    let rc = unsafe { libc::proc_pid_rusage(raw, libc::RUSAGE_INFO_V2, (&raw mut info).cast()) };
    (rc == 0 && info.ri_phys_footprint > 0).then_some(info.ri_phys_footprint)
}

pub fn responsible_pid(pid: u32) -> Option<u32> {
    use std::sync::OnceLock;
    type Responsible = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
    // A long-standing libsystem function (used by Activity Monitor) that has no
    // public header; looked up at runtime so a future macOS without it just
    // means "unknown" instead of a crash.
    static FUNC: OnceLock<Option<Responsible>> = OnceLock::new();
    let f = FUNC.get_or_init(|| {
        // SAFETY: dlsym with RTLD_DEFAULT and a valid C string; a null result means absent.
        let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr()) };
        // SAFETY: the symbol has this C signature (pid_t -> pid_t) on every macOS that exports it.
        (!sym.is_null()).then(|| unsafe { std::mem::transmute::<*mut libc::c_void, Responsible>(sym) })
    });
    let f = (*f)?;
    let raw = libc::pid_t::try_from(pid).ok()?;
    // SAFETY: plain call with a pid; returns -1 on failure.
    let r = unsafe { f(raw) };
    (r > 1 && r != raw).then_some(r as u32)
}

pub fn ask_app_to_quit(app_pid: u32, _pids: &[u32]) -> Result<(), String> {
    autoreleasepool(|_| {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(app_pid as libc::pid_t);
        match app {
            // The same request as Quit in the app's menu: it can save or ask first.
            Some(app) => {
                if app.terminate() {
                    Ok(())
                } else {
                    Err("Quit was refused".into())
                }
            }
            // Background helpers that aren't registered apps get a polite signal instead.
            None => ask_process_to_stop(app_pid),
        }
    })
}

pub fn can_ask_politely(_app_pid: Option<u32>, _pids: &[u32]) -> bool {
    true
}

pub fn ask_process_to_stop(pid: u32) -> Result<(), String> {
    crate::actions::send_signal(pid, libc::SIGTERM)
}
