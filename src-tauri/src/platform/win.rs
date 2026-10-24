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

pub fn footprint(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // Private bytes: memory the process owns whether or not Windows has
    // trimmed it out of the working set (idle processes get trimmed, so the
    // working set alone under-reports them, like RSS on macOS).
    // SAFETY: plain Win32 calls; the handle is closed before returning and the
    // counters struct is sized for the call.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
        let ok = GetProcessMemoryInfo(
            h,
            (&raw mut c).cast::<PROCESS_MEMORY_COUNTERS>(),
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        );
        let _ = CloseHandle(h);
        ok.ok()?;
        let private = c.PrivateUsage as u64;
        (private > 0).then_some(private.max(c.WorkingSetSize as u64))
    }
}

pub fn start_us(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: plain Win32 calls; the handle is closed before returning and
    // every FILETIME is a valid out-pointer.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
        let _ = CloseHandle(h);
        ok.ok()?;
        // 100 ns steps since 1601.
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        (ticks > 0).then_some(ticks / 10)
    }
}

pub fn responsible_pid(_pid: u32) -> Option<u32> {
    None
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
