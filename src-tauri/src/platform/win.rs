use std::collections::{HashMap, HashSet};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    WM_CLOSE,
};
use windows::core::BOOL;

use super::{Pressure, Thermal};

pub fn frontmost_pid() -> Option<u32> {
    // SAFETY: plain Win32 queries with no pointers held past the call.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

pub fn memory_pressure() -> Pressure {
    // Windows has no single pressure flag; the sampler falls back to used/total.
    Pressure::Unknown
}

pub fn thermal_state() -> Thermal {
    // Not available to normal (non-admin) apps on Windows.
    Thermal::Unknown
}

pub fn terminal_idle(_pids: &[u32]) -> HashMap<u32, u64> {
    HashMap::new()
}

struct Collect {
    pids: HashSet<u32>,
    found: Vec<HWND>,
}

unsafe extern "system" fn collect_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the &mut Collect passed to EnumWindows below, alive for the whole call.
    let state = unsafe { &mut *(lparam.0 as *mut Collect) };
    let mut pid = 0u32;
    // SAFETY: hwnd comes from EnumWindows; the out-pointer is a local.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let top_level = GetWindow(hwnd, GW_OWNER).map(|o| o.is_invalid()).unwrap_or(true);
        if state.pids.contains(&pid) && IsWindowVisible(hwnd).as_bool() && top_level {
            state.found.push(hwnd);
        }
    }
    BOOL(1)
}

fn visible_windows(pids: &[u32]) -> Vec<HWND> {
    let mut state = Collect { pids: pids.iter().copied().collect(), found: Vec::new() };
    // SAFETY: the callback only touches `state`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect_windows), LPARAM(&raw mut state as isize));
    }
    state.found
}

pub fn ask_app_to_quit(_app_pid: u32, pids: &[u32]) -> Result<(), String> {
    let windows = visible_windows(pids);
    if windows.is_empty() {
        return Err("No window, force close only".into());
    }
    for hwnd in windows {
        // Same as clicking the window's close button: the app may ask to save.
        // SAFETY: posting a message to a window handle; no memory is shared.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    Ok(())
}

pub fn can_ask_politely(_app_pid: Option<u32>, pids: &[u32]) -> bool {
    !visible_windows(pids).is_empty()
}

pub fn ask_process_to_stop(pid: u32) -> Result<(), String> {
    // Console programs have no polite stop signal we can send safely from
    // outside their console, so try their windows and otherwise report back.
    ask_app_to_quit(pid, &[pid])
}
