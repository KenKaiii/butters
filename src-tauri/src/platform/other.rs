//! Fallback for platforms Butters doesn't target (keeps `cargo check` working).

use std::collections::HashMap;

use super::{Pressure, Thermal};

pub fn frontmost_pid() -> Option<u32> {
    None
}
pub fn memory_pressure() -> Pressure {
    Pressure::Unknown
}
pub fn thermal_state() -> Thermal {
    Thermal::Unknown
}
pub fn terminal_idle(_pids: &[u32]) -> HashMap<u32, u64> {
    HashMap::new()
}
pub fn footprint(_pid: u32) -> Option<u64> {
    None
}
pub fn responsible_pid(_pid: u32) -> Option<u32> {
    None
}
/// Linux: field 22 of /proc/<pid>/stat, clock ticks since boot (10 ms on
/// most systems; finer than the second the rest of the engine uses).
pub fn start_us(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The name (field 2) is in parentheses and may hold spaces.
        let rest = &stat[stat.rfind(')')? + 2..];
        let ticks: u64 = rest.split(' ').nth(19)?.parse().ok()?;
        // SAFETY: sysconf has no preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        (hz > 0).then(|| ticks * 1_000_000 / hz as u64)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}
pub fn ask_app_to_quit(app_pid: u32, _pids: &[u32]) -> Result<(), String> {
    ask_process_to_stop(app_pid)
}
pub fn can_ask_politely(_app_pid: Option<u32>, _pids: &[u32]) -> bool {
    cfg!(unix)
}
pub fn ask_process_to_stop(pid: u32) -> Result<(), String> {
    #[cfg(unix)]
    return crate::actions::send_signal(pid, libc::SIGTERM);
    #[cfg(not(unix))]
    Err(format!("not supported for {pid}"))
}
