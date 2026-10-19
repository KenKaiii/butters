//! The small amount of OS-specific code: which app is in front, memory and
//! heat state, and asking an app to quit the way the user would.

use std::collections::HashMap;

use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as imp;

#[cfg(not(any(target_os = "macos", windows)))]
mod other;
#[cfg(not(any(target_os = "macos", windows)))]
use other as imp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pressure {
    Normal,
    Warn,
    Critical,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Thermal {
    Nominal,
    Fair,
    Serious,
    Critical,
    Unknown,
}

/// pid of the app in front, as the OS sees it.
pub fn frontmost_pid() -> Option<u32> {
    imp::frontmost_pid()
}

/// The OS's own memory-pressure signal, where it has one.
pub fn memory_pressure() -> Pressure {
    imp::memory_pressure()
}

pub fn thermal_state() -> Thermal {
    imp::thermal_state()
}

/// Seconds since each process's terminal last saw input or output. Only
/// processes attached to a terminal appear. Empty where unsupported.
pub fn terminal_idle(pids: &[u32]) -> HashMap<u32, u64> {
    imp::terminal_idle(pids)
}

/// Asks a desktop app to quit like the user would (it may ask to save).
/// `pids` are all the app's processes; `app_pid` is its main one.
pub fn ask_app_to_quit(app_pid: u32, pids: &[u32]) -> Result<(), String> {
    imp::ask_app_to_quit(app_pid, pids)
}

/// Whether `ask_app_to_quit` / a polite signal can work for these processes.
/// On Windows, programs without a window can only be stopped forcefully.
pub fn can_ask_politely(app_pid: Option<u32>, pids: &[u32]) -> bool {
    imp::can_ask_politely(app_pid, pids)
}

/// The app macOS holds responsible for a helper process (e.g. Docker Desktop
/// for its virtual machine). `None` where unknown or unsupported.
pub fn responsible_pid(pid: u32) -> Option<u32> {
    imp::responsible_pid(pid)
}

/// Polite request for a command-line process to stop (SIGTERM on Mac).
pub fn ask_process_to_stop(pid: u32) -> Result<(), String> {
    imp::ask_process_to_stop(pid)
}
