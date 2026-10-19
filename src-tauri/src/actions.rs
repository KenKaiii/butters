//! Closing things. Every request is re-checked against the live system at
//! the moment it runs:
//! - each pid must still belong to the *same* process (pid + start time),
//!   so a reused pid can never hit an unrelated program;
//! - every process must belong to the signed-in user;
//! - Butters, whatever launched it, and the app in front are refused;
//! - pids 0 and 1 are refused outright (signals to them mean "everyone").
//!
//! Polite first: apps get the same Quit request as their menu, command-line
//! tools get SIGTERM. Forcing is a separate, explicit request.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::platform;

/// What the panel asked to close, as captured by the scan that suggested it.
#[derive(Debug, Clone)]
pub struct ProcTarget {
    pub title: String,
    /// (pid, start_time) for every process in the group, root first.
    pub members: Vec<(u32, u64)>,
    pub app_pid: Option<u32>,
    pub memory: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    /// Everything exited. `freed` is the measured memory of what exited;
    /// `disk` is disk space given back (Docker clean-ups).
    Closed { message: String, freed: u64, disk: u64 },
    /// Asked politely, but it's still running (maybe asking to save).
    StillRunning { message: String },
    /// Can't be asked politely; only a forced close would work.
    NeedsForce { message: String },
    /// A safety rule blocked it.
    Refused { message: String },
    Failed { message: String },
}

/// pids that must never be touched, computed fresh at action time.
pub struct Guard {
    pub forbidden: HashSet<u32>,
}

impl Guard {
    pub fn current(frontmost: Option<u32>) -> Guard {
        let mut forbidden: HashSet<u32> = [0, 1].into_iter().collect();
        let me = std::process::id();
        forbidden.insert(me);
        // Butters' ancestors (the terminal it was started from, in dev).
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().without_tasks());
        let mut cur = Pid::from_u32(me);
        for _ in 0..64 {
            match sys.process(cur).and_then(|p| p.parent()) {
                Some(parent) if forbidden.insert(parent.as_u32()) => cur = parent,
                _ => break,
            }
        }
        if let Some(f) = frontmost {
            forbidden.insert(f);
        }
        Guard { forbidden }
    }
}

struct Live {
    pid: u32,
    memory: u64,
}

/// Checks every member against the live system. Returns the processes that
/// are still the same ones the scan saw, or why the request is refused.
fn validate(target: &ProcTarget, guard: &Guard) -> Result<Vec<Live>, String> {
    let pids: Vec<Pid> = target.members.iter().map(|(p, _)| Pid::from_u32(*p)).collect();
    let mut sys = System::new();
    let me = Pid::from_u32(std::process::id());
    let mut with_me = pids.clone();
    with_me.push(me);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&with_me),
        true,
        ProcessRefreshKind::nothing().with_memory().with_user(UpdateKind::Always).without_tasks(),
    );
    let my_uid = sys.process(me).and_then(|p| p.user_id().cloned());
    let Some(my_uid) = my_uid else {
        return Err("Can't verify owner, skipped".into());
    };
    let mut live = Vec::new();
    for (pid, start) in &target.members {
        if guard.forbidden.contains(pid) || *pid <= 1 {
            return Err("Protected, skipped".to_string());
        }
        let Some(p) = sys.process(Pid::from_u32(*pid)) else { continue }; // already gone
        if p.start_time() != *start {
            continue; // pid now belongs to a different program: leave it alone
        }
        if p.user_id() != Some(&my_uid) {
            return Err("Not yours, skipped".to_string());
        }
        live.push(Live { pid: *pid, memory: p.memory() });
    }
    Ok(live)
}

#[cfg(unix)]
pub fn send_signal(pid: u32, sig: libc::c_int) -> Result<(), String> {
    // pid 0 / 1 / negative values address whole process groups or every
    // process; never let one through, whatever the caller did.
    let Ok(raw) = libc::pid_t::try_from(pid) else { return Err("invalid process id".into()) };
    if raw <= 1 {
        return Err("refusing to signal a process group or init".into());
    }
    // SAFETY: kill(2) with a positive, validated pid.
    let rc = unsafe { libc::kill(raw, sig) };
    if rc == 0 {
        Ok(())
    } else {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) { Ok(()) } else { Err(err.to_string()) }
    }
}

fn force_kill(pid: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        send_signal(pid, libc::SIGKILL)
    }
    #[cfg(not(unix))]
    {
        if pid <= 4 {
            return Err("refusing to stop a system process".into());
        }
        let mut sys = System::new();
        let p = Pid::from_u32(pid);
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing().without_tasks());
        match sys.process(p) {
            Some(proc_) if proc_.kill() => Ok(()),
            Some(_) => Err("Windows refused to stop it".into()),
            None => Ok(()),
        }
    }
}

/// Which of `live` are still running and still the same processes.
fn still_running(target: &ProcTarget, live: &[Live]) -> Vec<u32> {
    let pids: Vec<Pid> = live.iter().map(|l| Pid::from_u32(l.pid)).collect();
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, ProcessRefreshKind::nothing().without_tasks());
    live.iter()
        .filter(|l| {
            let start = target.members.iter().find(|(p, _)| *p == l.pid).map(|(_, s)| *s);
            sys.process(Pid::from_u32(l.pid)).is_some_and(|p| Some(p.start_time()) == start && !is_zombie(p))
        })
        .map(|l| l.pid)
        .collect()
}

fn is_zombie(p: &sysinfo::Process) -> bool {
    matches!(p.status(), sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead)
}

fn wait_for_exit(target: &ProcTarget, live: &[Live], timeout: Duration) -> Vec<u32> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = still_running(target, live);
        if left.is_empty() || Instant::now() >= deadline {
            return left;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn freed(live: &[Live], left: &[u32]) -> u64 {
    live.iter().filter(|l| !left.contains(&l.pid)).map(|l| l.memory).sum()
}

pub fn close(target: &ProcTarget, force: bool, guard: &Guard) -> Outcome {
    let live = match validate(target, guard) {
        Ok(l) => l,
        Err(message) => return Outcome::Refused { message },
    };
    if live.is_empty() {
        return Outcome::Closed { message: "Already closed".to_string(), freed: 0, disk: 0 };
    }
    let pids: Vec<u32> = live.iter().map(|l| l.pid).collect();

    if force {
        let mut errors = Vec::new();
        // Children first, so a parent can't respawn them on its way out.
        for pid in pids.iter().rev() {
            if let Err(e) = force_kill(*pid) {
                errors.push(e);
            }
        }
        let left = wait_for_exit(target, &live, Duration::from_secs(3));
        return if left.is_empty() {
            let bytes = freed(&live, &left);
            Outcome::Closed { message: format!("Freed {}", crate::engine::assess::fmt_bytes(bytes)), freed: bytes, disk: 0 }
        } else {
            Outcome::Failed { message: errors.first().cloned().unwrap_or_else(|| "Still running".to_string()) }
        };
    }

    // Polite: the app's own Quit, or SIGTERM to each command-line process.
    let asked = match target.app_pid.filter(|a| pids.contains(a)) {
        Some(app_pid) => platform::ask_app_to_quit(app_pid, &pids),
        None => {
            if !platform::can_ask_politely(None, &pids) {
                return Outcome::NeedsForce {
                    message: "No window, force close only".to_string(),
                };
            }
            pids.iter().try_for_each(|pid| platform::ask_process_to_stop(*pid))
        }
    };
    if let Err(message) = asked {
        return Outcome::NeedsForce { message };
    }
    let wait = if target.app_pid.is_some() { Duration::from_secs(8) } else { Duration::from_secs(5) };
    let left = wait_for_exit(target, &live, wait);
    if left.is_empty() {
        let bytes = freed(&live, &left);
        Outcome::Closed { message: format!("Freed {}", crate::engine::assess::fmt_bytes(bytes)), freed: bytes, disk: 0 }
    } else {
        Outcome::StillRunning {
            message: if target.app_pid.is_some() {
                "Still open, may be asking to save".to_string()
            } else {
                "Didn't stop".to_string()
            },
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    fn spawn_sleeper() -> (Child, u64) {
        let child = Command::new("sleep").arg("300").spawn().expect("spawn sleep");
        let mut sys = System::new();
        let pid = Pid::from_u32(child.id());
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
        let start = sys.process(pid).expect("sleep visible").start_time();
        (child, start)
    }

    fn target(pid: u32, start: u64) -> ProcTarget {
        ProcTarget { title: "sleep".into(), members: vec![(pid, start)], app_pid: None, memory: 0 }
    }

    fn reap(mut c: Child) {
        let _ = c.kill();
        let _ = c.wait();
    }

    #[test]
    fn polite_close_stops_a_real_process() {
        let (child, start) = spawn_sleeper();
        let pid = child.id();
        let guard = Guard { forbidden: [0, 1, std::process::id()].into_iter().collect() };
        // Reap in the background so the exited child doesn't linger as a zombie.
        let reaper = std::thread::spawn(move || {
            let mut c = child;
            let _ = c.wait();
        });
        let out = close(&target(pid, start), false, &guard);
        assert!(matches!(out, Outcome::Closed { .. }), "{out:?}");
        reaper.join().unwrap();
    }

    #[test]
    fn reused_pid_is_never_signalled() {
        let (child, start) = spawn_sleeper();
        let pid = child.id();
        let guard = Guard { forbidden: [0, 1].into_iter().collect() };
        // Pretend the scan saw a different process with this pid.
        let out = close(&target(pid, start + 1), true, &guard);
        assert!(matches!(out, Outcome::Closed { .. }), "treated as already gone: {out:?}");
        // The real process must still be alive.
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, ProcessRefreshKind::nothing());
        assert!(sys.process(Pid::from_u32(pid)).is_some());
        reap(child);
    }

    #[test]
    fn forbidden_pids_are_refused() {
        let (child, start) = spawn_sleeper();
        let pid = child.id();
        let guard = Guard { forbidden: [0, 1, pid].into_iter().collect() };
        let out = close(&target(pid, start), true, &guard);
        assert!(matches!(out, Outcome::Refused { .. }), "{out:?}");
        reap(child);
    }

    #[test]
    fn init_and_group_pids_are_refused() {
        let guard = Guard { forbidden: HashSet::new() };
        let out = close(&target(1, 0), true, &guard);
        assert!(matches!(out, Outcome::Refused { .. }), "{out:?}");
        assert!(send_signal(0, libc::SIGTERM).is_err());
        assert!(send_signal(1, libc::SIGTERM).is_err());
        assert!(send_signal(u32::MAX, libc::SIGTERM).is_err(), "would be -1: every process");
    }

    #[test]
    fn guard_includes_self_and_ancestors() {
        let g = Guard::current(None);
        assert!(g.forbidden.contains(&std::process::id()));
        assert!(g.forbidden.contains(&0) && g.forbidden.contains(&1));
        assert!(g.forbidden.len() >= 4, "self, at least one parent, 0 and 1");
    }
}
