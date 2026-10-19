//! Keeps one lightweight process table and remembers when each process last
//! did any work, so "quiet for 2 h" is measured rather than guessed.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, Uid, UpdateKind};

use crate::engine::classify::{app_name, app_root};
use crate::engine::model::ProcInfo;
use crate::trends::Reading;
use crate::platform::{self, Pressure, Thermal};

/// A process counts as active when it used at least this much of one core
/// on average since the previous sample.
const ACTIVE_CPU_PCT: f32 = 2.0;

struct Activity {
    cpu_ms: u64,
    at: Instant,
    cpu_pct: f32,
    last_active: Option<Instant>,
    first_seen: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub cpu: f32,
    pub cores: usize,
    pub pressure: Pressure,
    pub thermal: Thermal,
}

pub struct Sampler {
    sys: System,
    activity: HashMap<(u32, u64), Activity>,
    last_refresh: Option<Instant>,
    my_uid: Option<Uid>,
    self_pid: u32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            sys: System::new(),
            activity: HashMap::new(),
            last_refresh: None,
            my_uid: None,
            self_pid: std::process::id(),
        }
    }

    /// Cheap refresh: CPU time and memory for every process, plus totals.
    /// Runs about once a minute in the background.
    pub fn sample(&mut self) {
        // exe and owner are fetched once per process, then reused.
        self.refresh(
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        if self.my_uid.is_none() {
            self.my_uid = self.sys.process(Pid::from_u32(self.self_pid)).and_then(|p| p.user_id().cloned());
        }
    }

    /// The user's own processes as of the last refresh, for leak watch and
    /// the fan view. App helpers are labelled with their app's name.
    pub fn readings(&self) -> Vec<Reading> {
        let mut out = Vec::new();
        for (pid, p) in self.sys.processes() {
            let owned = matches!((&self.my_uid, p.user_id()), (Some(me), Some(them)) if me == them);
            if !owned || pid.as_u32() == self.self_pid {
                continue;
            }
            let key = (pid.as_u32(), p.start_time());
            let cpu = self.activity.get(&key).map(|a| a.cpu_pct).unwrap_or(0.0);
            let label = p
                .exe()
                .map(|e| e.to_string_lossy().into_owned())
                .and_then(|e| app_root(&e))
                .map(|root| app_name(&root))
                .unwrap_or_else(|| p.name().to_string_lossy().into_owned());
            out.push(Reading { pid: key.0, start: key.1, label, memory: p.memory(), cpu });
        }
        out
    }

    fn refresh(&mut self, kind: ProcessRefreshKind) {
        self.sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        self.sys.refresh_memory();
        self.sys.refresh_cpu_usage();
        let now = Instant::now();
        self.last_refresh = Some(now);
        let mut seen = std::collections::HashSet::with_capacity(self.sys.processes().len());
        for (pid, p) in self.sys.processes() {
            let key = (pid.as_u32(), p.start_time());
            seen.insert(key);
            let cpu_ms = p.accumulated_cpu_time();
            match self.activity.get_mut(&key) {
                Some(a) => {
                    let wall = now.duration_since(a.at).as_millis().max(1) as f32;
                    let pct = cpu_ms.saturating_sub(a.cpu_ms) as f32 / wall * 100.0;
                    a.cpu_pct = pct;
                    if pct >= ACTIVE_CPU_PCT {
                        a.last_active = Some(now);
                    }
                    a.cpu_ms = cpu_ms;
                    a.at = now;
                }
                None => {
                    self.activity.insert(
                        key,
                        Activity { cpu_ms, at: now, cpu_pct: 0.0, last_active: None, first_seen: now },
                    );
                }
            }
        }
        self.activity.retain(|k, _| seen.contains(k));
    }

    /// Full picture for a scan: adds command lines, folders and owners, and
    /// combines CPU history with terminal activity into an idle time.
    pub fn snapshot(&mut self, ports: &HashMap<u32, Vec<u16>>) -> Vec<ProcInfo> {
        let first = self.last_refresh.is_none();
        let kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet)
            .with_user(UpdateKind::OnlyIfNotSet)
            .without_tasks();
        if first || self.last_refresh.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            // CPU use needs two readings; take a short second one on first use.
            self.refresh(kind);
            std::thread::sleep(Duration::from_millis(500));
        }
        self.refresh(kind);

        if self.my_uid.is_none() {
            self.my_uid = self.sys.process(Pid::from_u32(self.self_pid)).and_then(|p| p.user_id().cloned());
        }
        let pids: Vec<u32> = self.sys.processes().keys().map(|p| p.as_u32()).collect();
        let tty_idle = platform::terminal_idle(&pids);
        let now = Instant::now();

        let mut out: Vec<ProcInfo> = Vec::with_capacity(pids.len());
        for (pid, p) in self.sys.processes() {
            let pid = pid.as_u32();
            let key = (pid, p.start_time());
            let (cpu, idle, lower_bound) = match self.activity.get(&key) {
                Some(a) => {
                    let since = a.last_active.unwrap_or(a.first_seen);
                    (a.cpu_pct, now.duration_since(since).as_secs(), a.last_active.is_none())
                }
                None => (0.0, 0, true),
            };
            // Terminal I/O is a precise "last used" signal. Use it unless the CPU
            // history shows more recent work.
            let (idle, lower_bound) = match tty_idle.get(&pid) {
                Some(&t) if lower_bound => (t, false),
                Some(&t) => (idle.min(t), false),
                None => (idle, lower_bound),
            };
            let owned = match (&self.my_uid, p.user_id()) {
                (Some(me), Some(them)) => me == them,
                _ => false, // unknown owner: treat as not ours (fail closed)
            };
            out.push(ProcInfo {
                pid,
                parent: p.parent().map(|pp| pp.as_u32()),
                start_time: p.start_time(),
                name: p.name().to_string_lossy().into_owned(),
                exe: p.exe().map(|e| e.to_string_lossy().into_owned()),
                cmd: p.cmd().iter().take(8).map(|a| a.to_string_lossy().into_owned()).collect(),
                cwd: p.cwd().map(|c| c.to_string_lossy().into_owned()),
                memory: p.memory(),
                cpu,
                idle_secs: idle,
                idle_is_lower_bound: lower_bound,
                owned_by_user: owned,
                ports: ports.get(&pid).cloned().unwrap_or_default(),
                growth: None,
                responsible: None,
            });
        }
        // Virtual machines run as system services; count them under the app
        // that started them (Docker Desktop's VM holds most of its memory).
        for p in out.iter_mut() {
            if p.name.starts_with("com.apple.Virtualization.VirtualMachine") {
                p.responsible = platform::responsible_pid(p.pid);
            }
        }
        out
    }

    /// Just memory and CPU totals: a cheap reading for the live meters
    /// (no per-process work).
    pub fn quick_stats(&mut self) -> Stats {
        self.sys.refresh_memory();
        self.sys.refresh_cpu_usage();
        self.stats()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            mem_used: self.sys.used_memory(),
            mem_total: self.sys.total_memory(),
            swap_used: self.sys.used_swap(),
            cpu: self.sys.global_cpu_usage(),
            cores: self.sys.cpus().len().max(1),
            pressure: platform::memory_pressure(),
            thermal: platform::thermal_state(),
        }
    }

    /// Memory of processes that are not the user's (OS and other accounts).
    pub fn my_uid(&self) -> Option<&Uid> {
        self.my_uid.as_ref()
    }
}

/// TCP ports each process is listening on. Only called during a scan.
pub fn listening_ports() -> HashMap<u32, Vec<u16>> {
    let mut out: HashMap<u32, Vec<u16>> = HashMap::new();
    if let Ok(all) = listeners::get_all() {
        for l in all {
            if l.protocol == listeners::Protocol::TCP && l.state == listeners::SocketState::Listen {
                let ports = out.entry(l.process.pid).or_default();
                let port = l.socket.port();
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
    }
    out
}
