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
