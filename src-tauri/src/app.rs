//! Shared state and the commands the panel can call. The panel only ever
//! refers to suggestions by the opaque id the last scan gave it; the
//! processes behind an id are kept here and re-checked before acting.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::actions::{self, Guard, Outcome, ProcTarget};
use crate::deepclean;
use crate::alerts::{self, AlertState, Level};
use crate::engine::assess::{self, Context, Verdict, fmt_bytes, fmt_duration};
use crate::engine::model::{Group, Kind};
use crate::platform;
use crate::sampler::{self, Sampler, Stats};
use crate::services;
use crate::docker_activity::Activity;
use crate::history::{self, History, Summary};
use crate::log::Log;
use crate::trends::{Culprit, Trends};
use crate::settings::Settings;

const MB: u64 = 1024 * 1024;
const GB_U: u64 = 1024 * MB;

/// Remembers which apps the user had in front, and when.
#[derive(Debug)]
pub struct FrontTracker {
    self_pid: u32,
    pub last_front: Option<u32>,
    recent: HashMap<u32, Instant>,
}

impl Default for FrontTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FrontTracker {
    pub fn new() -> Self {
        FrontTracker { self_pid: std::process::id(), last_front: None, recent: HashMap::new() }
    }

    pub fn poll(&mut self) {
        if let Some(pid) = platform::frontmost_pid()
            && pid != self.self_pid {
                self.last_front = Some(pid);
                self.recent.insert(pid, Instant::now());
            }
        self.recent.retain(|_, t| t.elapsed().as_secs() < 6 * 3600);
    }

    fn recent_secs(&self) -> HashMap<u32, u64> {
        self.recent.iter().map(|(pid, t)| (*pid, t.elapsed().as_secs())).collect()
    }
}

#[derive(Debug, Clone)]
enum Target {
    Procs { key: String, kind: Kind, target: ProcTarget },
    Container { key: String, id: String, name: String, memory: u64 },
    Model { key: String, name: String, memory: u64 },
    DockerDisk { key: String, which: DiskWhich },
    Deep { item: Box<deepclean::Item> },
}

#[derive(Debug, Clone)]
enum DiskWhich {
    BuildCache,
    DanglingImages,
    OldImages(Vec<String>),
}

impl Target {
    fn key(&self) -> &str {
        match self {
            Target::Procs { key, .. } | Target::Container { key, .. } | Target::Model { key, .. } | Target::DockerDisk { key, .. } => key,
            Target::Deep { item } => &item.key,
        }
    }

    /// What the log calls it (the same name the panel showed).
    fn label(&self) -> String {
        match self {
            Target::Procs { target, .. } => target.title.clone(),
            Target::Container { name, .. } => format!("Container: {name}"),
            Target::Model { name, .. } => format!("Ollama model: {name}"),
            Target::DockerDisk { key, .. } => key.clone(),
            Target::Deep { item } => item.title.clone(),
        }
    }
}

pub struct AppState {
    pub sampler: Mutex<Sampler>,
    pub front: Mutex<FrontTracker>,
    targets: Mutex<HashMap<String, Target>>,
    pub settings: Mutex<Settings>,
    pub settings_path: PathBuf,
    pub history: Mutex<History>,
    pub history_path: PathBuf,
    pub started: Instant,
    containers: Mutex<Option<usize>>,
    containers_all_idle: Mutex<Option<bool>>,
    pub trends: Mutex<Trends>,
    docker_activity: Mutex<Activity>,
    docker_activity_path: PathBuf,
    leak_notified: Mutex<HashSet<(u32, u64)>>,
    /// Ids from the latest scans that the end-of-day sweep may include.
    sweepable: Mutex<Vec<String>>,
    /// The last Docker/Ollama result, so the panel can show it instantly
    /// while a fresh one is fetched.
    services_cache: Mutex<Option<ServicesResult>>,
    pub alerts: Mutex<AlertState>,
    pub panel_open: AtomicBool,
    seq: AtomicU64,
    pub log: Log,
    /// What the log last said each list held (key -> title), so it only
    /// notes what appeared or went away, not every refresh.
    logged_procs: Mutex<HashMap<String, String>>,
    logged_services: Mutex<HashMap<String, String>>,
    /// The latest Deep clean look (it only runs when asked).
    deep: Mutex<DeepStatus>,
}

impl AppState {
    pub fn new(settings_path: PathBuf, log: Log) -> Self {
        let dir = settings_path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        let history_path = History::path(&dir);
        let docker_activity_path = Activity::path(&dir);
        let settings = Settings::load(&settings_path);
        log.info(format!("Settings: {}", settings_line(&settings)));
        AppState {
            sampler: Mutex::new(Sampler::new()),
            front: Mutex::new(FrontTracker::new()),
            targets: Mutex::new(HashMap::new()),
            settings: Mutex::new(settings),
            history: Mutex::new(History::load(&history_path)),
            history_path,
            settings_path,
            started: Instant::now(),
            containers: Mutex::new(None),
            containers_all_idle: Mutex::new(None),
            trends: Mutex::new(Trends::default()),
            docker_activity: Mutex::new(Activity::load(&docker_activity_path)),
            docker_activity_path,
            leak_notified: Mutex::new(HashSet::new()),
            sweepable: Mutex::new(Vec::new()),
            services_cache: Mutex::new(None),
            alerts: Mutex::new(AlertState::default()),
            panel_open: AtomicBool::new(false),
            seq: AtomicU64::new(1),
            log,
            logged_procs: Mutex::new(HashMap::new()),
            logged_services: Mutex::new(HashMap::new()),
            deep: Mutex::new(DeepStatus::default()),
        }
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn settings_line(s: &Settings) -> String {
    let shortcut = if s.shortcut.is_empty() { "off" } else { s.shortcut.as_str() };
    format!(
        "alerts {}, sounds {}, launch at login {}, shortcut {shortcut}, deep clean ticks after {} days, {} ignored",
        on_off(s.alerts),
        on_off(s.sounds),
        on_off(s.launch_at_login),
        s.deep_clean_days,
        s.ignored.len()
    )
}

fn kind_name(kind: Kind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Logs what's new in a list since the last time it was logged, and what
/// went away. Unchanged items stay silent.
fn log_list_changes(log: &Log, logged: &Mutex<HashMap<String, String>>, suggestions: &[Suggestion]) {
    let mut logged = lock(logged);
    let now: HashMap<String, String> = suggestions.iter().map(|s| (s.key.clone(), s.title.clone())).collect();
    let mut gone: Vec<&String> = logged.iter().filter(|(k, _)| !now.contains_key(*k)).map(|(_, t)| t).collect();
    gone.sort();
    for title in gone {
        log.info(format!("No longer suggesting \"{title}\""));
    }
    for s in suggestions.iter().filter(|s| !logged.contains_key(&s.key)) {
        let size = if s.disk > 0 { format!("{} disk", fmt_bytes(s.disk)) } else { fmt_bytes(s.memory) };
        log.info(format!(
            "Suggesting \"{}\" ({}): {}, {size}, CPU {:.0}%{}{}",
            s.title,
            kind_name(s.kind),
            s.reason,
            s.cpu,
            if s.recommended { ", pre-ticked" } else { "" },
            s.note.as_deref().map(|n| format!(", note: {n}")).unwrap_or_default(),
        ));
    }
    *logged = now;
}

/// How a close went, for the log.
fn outcome_entry(outcome: &Outcome) -> (crate::log::Level, &'static str, &str) {
    use crate::log::Level as L;
    match outcome {
        Outcome::Closed { message, .. } => (L::Info, "Closed", message),
        Outcome::StillRunning { message } => (L::Warn, "Still running after asking", message),
        Outcome::NeedsForce { message } => (L::Warn, "Needs force close", message),
        Outcome::Refused { message } => (L::Warn, "Refused to close", message),
        Outcome::Failed { message } => (L::Error, "Failed to close", message),
    }
}

/// Recovers from a poisoned lock instead of crashing the tray app.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    id: String,
    /// Stable identity across scans, for the panel's selection and animations.
    key: String,
    kind: Kind,
    title: String,
    detail: Option<String>,
    reason: String,
    note: Option<String>,
    memory: u64,
    cpu: f32,
    ports: Vec<u16>,
    recommended: bool,
    /// Whether a polite close is possible (otherwise only "Force close").
    polite: bool,
    /// Label for the primary button ("Quit", "Stop", "Unload").
    verb: &'static str,
    /// Disk space it would give back (Docker clean-ups), instead of memory.
    disk: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Protected {
    /// Present only when the user may close it anyway (hover "Close").
    id: Option<String>,

    key: String,
    verb: &'static str,
    title: String,
    kind: Kind,
    memory: u64,
    cpu: f32,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    stats: Stats,
    level: Level,
    suggestions: Vec<Suggestion>,
    protected: Vec<Protected>,
    watched_secs: u64,
    scan_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServicesResult {
    suggestions: Vec<Suggestion>,
    docker: &'static str,
    ollama: &'static str,
}

fn scan_context(state: &AppState) -> Context {
    let (frontmost, recent) = {
        let f = lock(&state.front);
        (f.last_front, f.recent_secs())
    };
    Context {
        self_pid: std::process::id(),
        frontmost_pid: frontmost,
        recent_pids: recent,
        ignored: lock(&state.settings).ignored.iter().cloned().collect(),
        watched_secs: state.started.elapsed().as_secs(),
        running_containers: *lock(&state.containers),
        containers_all_idle: *lock(&state.containers_all_idle),
    }
}

fn do_scan(state: &AppState) -> ScanResult {
    let t0 = Instant::now();
    let ports = sampler::listening_ports();
    let (mut procs, stats) = {
        let mut s = lock(&state.sampler);
        let procs = s.snapshot(&ports);
        (procs, s.stats())
    };
    {
        let t = lock(&state.trends);
        for p in procs.iter_mut() {
            p.growth = t.growth_of(p.pid, p.start_time).map(|g| (g.bytes, g.secs));
        }
    }
    #[cfg(feature = "playground")]
    playground_view(&mut procs);
    let ctx = scan_context(state);
    let assessed = assess::assess(&procs, &ctx);
    let start_of: HashMap<u32, u64> = procs.iter().map(|p| (p.pid, p.start_time)).collect();
    let seq = state.seq.fetch_add(1, Ordering::Relaxed);

    let only = e2e_only_pid();
    let mut protected_groups: Vec<(Group, assess::Protection)> = Vec::new();
    let mut picked: Vec<(Group, String, bool, Option<String>)> = Vec::new();
    let mut protected = Vec::new();
    for a in assessed {
        let g = a.group;
        match a.verdict {
            Verdict::Suggest { reason, recommended, note } => {
                if only.is_some_and(|pid| !g.members.contains(&pid)) {
                    continue;
                }
                // Identical leftovers (same tool, all orphaned) become one card.
                let same = picked.iter_mut().find(|(p, r, _, _)| g.orphaned && p.orphaned && p.kind == g.kind && base_title(&p.title) == g.title && *r == reason);
                match same {
                    Some((p, _, _, _)) => merge_into(p, g),
                    None => picked.push((g, reason, recommended, note)),
                }
            }
            Verdict::Protected(p) => {
                if g.memory >= 150 * MB || g.cpu >= 10.0 {
                    protected_groups.push((g, p));
                }
            }
            Verdict::Quiet => {}
        }
    }

    let mut suggestions = Vec::new();
    let mut targets: HashMap<String, Target> = HashMap::new();
    for (i, (g, reason, recommended, note)) in picked.into_iter().enumerate() {
        let id = format!("p{seq}-{i}");
        let members: Vec<(u32, u64)> = g.members.iter().filter_map(|m| start_of.get(m).map(|s| (*m, *s))).collect();
        let polite = platform::can_ask_politely(g.app_pid, &g.members);
        let detail = match (&g.detail, g.ports.is_empty()) {
            (Some(d), false) => Some(format!("{d} · {}", ports_text(&g.ports))),
            (None, false) => Some(ports_text(&g.ports)),
            (d, true) => d.clone(),
        };
        suggestions.push(Suggestion {
            id: id.clone(),
            key: format!("{}#{}", g.key, g.root),
            kind: g.kind,
            title: g.title.clone(),
            detail,
            reason,
            note,
            memory: g.memory,
            cpu: g.cpu,
            ports: g.ports.clone(),
            recommended,
            polite,
            verb: if g.app_pid.is_some() { "Quit" } else { "Stop" },
            disk: 0,
        });
        targets.insert(
            id,
            Target::Procs {
                key: g.key.clone(),
                kind: g.kind,
                target: ProcTarget { title: g.title.clone(), members, app_pid: g.app_pid, memory: g.memory },
            },
        );
    }
    protected_groups.sort_by_key(|(g, _)| std::cmp::Reverse(g.memory));
    protected_groups.truncate(12);
    for (i, (g, p)) in protected_groups.into_iter().enumerate() {
        let id = p.user_closable().then(|| format!("x{seq}-{i}"));
        if let Some(id) = &id {
            let members: Vec<(u32, u64)> = g.members.iter().filter_map(|m| start_of.get(m).map(|s| (*m, *s))).collect();
            targets.insert(
                id.clone(),
                Target::Procs {
                    key: g.key.clone(),
                    kind: g.kind,
                    target: ProcTarget { title: g.title.clone(), members, app_pid: g.app_pid, memory: g.memory },
                },
            );
        }
        protected.push(Protected {
            id,
            key: format!("{}#{}", g.key, g.root),
            verb: if g.app_pid.is_some() { "Quit" } else { "Stop" },
            title: g.title,
            kind: g.kind,
            memory: g.memory,
            cpu: g.cpu,
            reason: p.text(),
        });
    }
    suggestions.sort_by(|a, b| b.recommended.cmp(&a.recommended).then(weight(b).total_cmp(&weight(a))));

    {
        let mut sw = lock(&state.sweepable);
        sw.retain(|id| !id.starts_with('p'));
        sw.extend(suggestions.iter().filter(|s| sweepable(s)).map(|s| s.id.clone()));
    }
    log_list_changes(&state.log, &state.logged_procs, &suggestions);
    // Keep the latest service targets; replace process ones.
    {
        let mut t = lock(&state.targets);
        t.retain(|_, v| !matches!(v, Target::Procs { .. }));
        t.extend(targets);
    }
    ScanResult {
        level: alerts::level(&stats),
        stats,
        suggestions,
        protected,
        watched_secs: ctx.watched_secs,
        scan_ms: t0.elapsed().as_millis() as u64,
    }
}

fn base_title(t: &str) -> &str {
    match t.rsplit_once(" ×") {
        Some((b, n)) if n.parse::<usize>().is_ok() => b,
        _ => t,
    }
}

/// Folds another leftover copy into `into`, counting copies in the title.
fn merge_into(into: &mut Group, other: Group) {
    let copies = into.title.rsplit_once(" ×").and_then(|(_, n)| n.parse::<usize>().ok()).unwrap_or(1) + 1;
    let base = into.title.rsplit_once(" ×").map(|(b, _)| b.to_string()).unwrap_or_else(|| into.title.clone());
    into.title = format!("{base} ×{copies}");
    if into.detail != other.detail {
        into.detail = Some(format!("{copies} folders"));
    }
    into.members.extend(other.members);
    into.memory += other.memory;
    into.cpu += other.cpu;
    for p in other.ports {
        if !into.ports.contains(&p) {
            into.ports.push(p);
        }
    }
    into.ports.sort_unstable();
}

/// Whether the end-of-day sweep may include this suggestion.
fn sweepable(s: &Suggestion) -> bool {
    if s.note.is_some() || !s.polite {
        return false;
    }
    match s.kind {
        Kind::DevServer | Kind::AgentSession | Kind::McpServer | Kind::Watcher => {
            s.recommended || s.reason == "Terminal closed" || s.reason == "Session ended"
        }
        Kind::DockerContainer => s.recommended,
        Kind::OllamaModel => true,
        _ => false,
    }
}

fn weight(s: &Suggestion) -> f64 {
    s.memory as f64 / MB as f64 + s.cpu as f64 * 20.0
}

fn ports_text(ports: &[u16]) -> String {
    let shown: Vec<String> = ports.iter().take(3).map(|p| format!(":{p}")).collect();
    let more = ports.len().saturating_sub(3);
    if more > 0 { format!("{} +{more}", shown.join(" ")) } else { shown.join(" ") }
}

/// Playground builds: keep only the Playground's own test processes (and
/// their children), and age them so every rule shows its card right away
/// (idle hours, a leak, an orphaned dev server) instead of after real time.
#[cfg(feature = "playground")]
fn playground_view(procs: &mut Vec<crate::engine::model::ProcInfo>) {
    let ours = crate::playground_state().pids();
    // Children of our processes count too (a node child of npm, etc.).
    let mut keep: HashSet<u32> = ours.clone();
    loop {
        let before = keep.len();
        for p in procs.iter() {
            if p.parent.is_some_and(|pp| keep.contains(&pp)) {
                keep.insert(p.pid);
            }
        }
        if keep.len() == before {
            break;
        }
    }
    procs.retain(|p| keep.contains(&p.pid));
    for p in procs.iter_mut() {
        let cmd = p.cmd.join(" ");
        if cmd.contains("leaky-worker") {
            p.growth = Some((2_200 * 1024 * 1024, 40 * 60));
            p.memory = p.memory.max(3 * 1024 * 1024 * 1024);
        } else if cmd.contains("webpack") {
            p.idle_secs = 0;
        } else {
            p.idle_secs = 3 * 60 * 60 + 10 * 60;
            p.idle_is_lower_bound = false;
        }
    }
}

/// Debug builds only: `BUTTERS_E2E_ONLY_PID` limits suggestions to the one
/// throwaway process an end-to-end test started, so a test can never close
/// the developer's real work. Compiled out of release builds entirely.
fn e2e_only_pid() -> Option<u32> {
    #[cfg(debug_assertions)]
    {
        std::env::var("BUTTERS_E2E_ONLY_PID").ok().and_then(|v| v.parse().ok())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Records a look at Docker's containers: when each was last active, how
/// many run, and whether they're all idle. Returns per-container idle times.
fn observe_docker(state: &AppState, containers: Option<&[services::Container]>) {
    let now = history::now_secs();
    match containers {
        Some(cs) => {
            let mut act = lock(&state.docker_activity);
            act.observe(cs, now);
            let _ = act.save(&state.docker_activity_path);
            let all_idle = !cs.is_empty() && cs.iter().all(|c| act.idle(&c.id, now).is_some_and(|i| i.secs >= DAY));
            *lock(&state.containers) = Some(cs.len());
            *lock(&state.containers_all_idle) = Some(all_idle);
        }
        None => {
            *lock(&state.containers) = None;
            *lock(&state.containers_all_idle) = None;
        }
    }
}

const DAY: u64 = 24 * 60 * 60;

fn do_scan_services(state: &AppState) -> ServicesResult {
    let docker = std::thread::spawn(services::docker_containers);
    let disk = std::thread::spawn(services::docker_disk);
    let ollama = std::thread::spawn(services::ollama_models);
    let mut containers = docker.join().ok().flatten();
    let mut disk = disk.join().ok().flatten();
    let mut models = ollama.join().ok().flatten();
    if e2e_only_pid().is_some() || cfg!(feature = "playground") {
        containers = None;
        disk = None;
        models = None;
    }
    let ignored: Vec<String> = lock(&state.settings).ignored.clone();
    let seq = state.seq.fetch_add(1, Ordering::Relaxed);
    let mut suggestions = Vec::new();
    let mut targets = HashMap::new();

    observe_docker(state, containers.as_deref());
    let now = history::now_secs();
    for (i, c) in containers.iter().flatten().enumerate() {
        let key = format!("docker:{}", c.name);
        let idle = lock(&state.docker_activity).idle(&c.id, now);
        // Only containers measured quiet for an hour or more; a day pre-ticks it.
        let Some(idle) = idle.filter(|i| i.secs >= 60 * 60) else { continue };
        if c.cpu >= 1.0 || ignored.contains(&key) {
            continue;
        }
        let id = format!("d{seq}-{i}");
        let detail = match &c.project {
            Some(p) => format!("{} · {p}", c.image),
            None => c.image.clone(),
        };
        suggestions.push(Suggestion {
            id: id.clone(),
            key: key.clone(),
            kind: Kind::DockerContainer,
            title: format!("Container: {}", c.name),
            detail: Some(detail),
            reason: format!("No activity {}{}", fmt_duration(idle.secs), if idle.lower_bound { "+" } else { "" }),
            note: None,
            memory: c.memory,
            cpu: c.cpu,
            ports: Vec::new(),
            recommended: idle.secs >= DAY,
            polite: true,
            verb: "Stop",
            disk: 0,
        });
        targets.insert(id, Target::Container { key, id: c.id.clone(), name: c.name.clone(), memory: c.memory });
    }
    // Disk Docker can give back without touching anything you use.
    if let Some(d) = disk {
        let cache_reason = match d.build_cache_idle {
            Some(secs) if secs >= DAY => format!("Unused {}, rebuilds if needed", fmt_duration(secs)),
            _ => "Rebuilds when needed".to_string(),
        };
        let old_count = d.old_images.len();
        let old_bytes: u64 = d.old_images.iter().map(|i| i.size).sum();
        let old_refs: Vec<String> = d.old_images.iter().map(|i| i.reference.clone()).collect();
        let old_detail = d.old_images.iter().map(|i| i.reference.rsplit('/').next().unwrap_or(&i.reference).to_string()).collect::<Vec<_>>().join(", ");
        let items = [
            (DiskWhich::BuildCache, "docker:build-cache", "Docker build cache".to_string(), cache_reason, None, d.build_cache, GB_U),
            (DiskWhich::DanglingImages, "docker:dangling", "Untagged Docker images".to_string(), "Left over from rebuilds".to_string(), None, d.dangling_images, 500 * MB),
            (
                DiskWhich::OldImages(old_refs),
                "docker:old-images",
                format!("Old image versions ×{old_count}"),
                "Newer version kept, none in use".to_string(),
                Some(old_detail),
                old_bytes,
                GB_U,
            ),
        ];
        for (i, (which, key, title, reason, detail, bytes, min)) in items.into_iter().enumerate() {
            if bytes < min || ignored.iter().any(|k| k == key) {
                continue;
            }
            let id = format!("k{seq}-{i}");
            suggestions.push(Suggestion {
                id: id.clone(),
                key: key.to_string(),
                kind: Kind::DockerDisk,
                title,
                detail,
                reason,
                note: None,
                memory: 0,
                cpu: 0.0,
                ports: Vec::new(),
                recommended: false,
                polite: true,
                verb: "Clear",
                disk: bytes,
            });
            targets.insert(id, Target::DockerDisk { key: key.to_string(), which });
        }
    }
    for (i, m) in models.iter().flatten().enumerate() {
        let key = format!("ollama:{}", m.name);
        if ignored.contains(&key) {
            continue;
        }
        let (reason, recommended) = match m.expires_in {
            Some(s) if s <= 5 * 60 => (format!("Unloads in {}", fmt_duration(s.max(60) as u64)), false),
            Some(s) => (format!("Kept {} more", fmt_duration(s as u64)), s > 30 * 60),
            None => ("Loaded".to_string(), false),
        };
        let id = format!("o{seq}-{i}");
        suggestions.push(Suggestion {
            id: id.clone(),
            key: key.clone(),
            kind: Kind::OllamaModel,
            title: format!("Ollama model: {}", m.name),
            detail: None,
            reason,
            note: None,
            memory: m.size,
            cpu: 0.0,
            ports: Vec::new(),
            recommended,
            polite: true,
            verb: "Unload",
            disk: 0,
        });
        targets.insert(id, Target::Model { key, name: m.name.clone(), memory: m.size });
    }
    {
        let mut t = lock(&state.targets);
        t.retain(|_, v| matches!(v, Target::Procs { .. } | Target::Deep { .. }));
        t.extend(targets);
    }
    {
        let mut sw = lock(&state.sweepable);
        sw.retain(|id| id.starts_with('p'));
        sw.extend(suggestions.iter().filter(|s| sweepable(s)).map(|s| s.id.clone()));
    }
    log_list_changes(&state.log, &state.logged_services, &suggestions);
    let result = ServicesResult {
        suggestions,
        docker: if containers.is_some() { "running" } else { "off" },
        ollama: if models.is_some() { "running" } else { "off" },
    };
    let previous = lock(&state.services_cache).replace(result.clone());
    for (name, was, now) in [
        ("Docker", previous.as_ref().map(|p| p.docker), result.docker),
        ("Ollama", previous.as_ref().map(|p| p.ollama), result.ollama),
    ] {
        if was != Some(now) {
            state.log.info(format!("{name} is {now}"));
        }
    }
    result
}

fn blocking_err(e: impl std::fmt::Display) -> String {
    format!("Internal error: {e}")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanResult {
    culprits: Vec<Culprit>,
    /// Minutes of history behind the numbers (up to 30).
    minutes: usize,
}

/// Who worked the CPU hardest over the last half hour.
#[tauri::command]
pub fn fan_culprits(state: State<'_, AppState>) -> FanResult {
    let (culprits, minutes) = lock(&state.trends).culprits(6);
    FanResult { culprits, minutes }
}

/// End-of-day sweep: from the latest scans, the ids that are safe to close
/// in one go. Only leftovers whose terminal is gone, idle agent sessions,
/// idle dev servers, MCP servers and watchers, containers idle a day+, and
/// Ollama models. Never desktop apps, never protected or ignored items, never
/// anything with a caution note. The panel still asks before closing them.
#[tauri::command]
pub fn sweep_targets(state: State<'_, AppState>) -> Vec<String> {
    lock(&state.sweepable).clone()
}

/// One Deep clean row, shaped like a suggestion so the panel's rows fit.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeepRow {
    id: String,
    key: String,
    kind: Kind,
    title: String,
    detail: Option<String>,
    reason: String,
    note: Option<String>,
    memory: u64,
    cpu: f32,
    ports: Vec<u16>,
    recommended: bool,
    polite: bool,
    verb: &'static str,
    disk: u64,
    group: deepclean::Group,
    /// Which tab it shows in.
    category: deepclean::Category,
    /// The size is a ceiling (pnpm keeps what projects use).
    up_to: bool,
    /// "delete" or "trash": what cleaning does to it.
    removal: &'static str,
}

impl DeepRow {
    fn new(id: String, item: &deepclean::Item) -> DeepRow {
        DeepRow {
            id,
            key: item.key.clone(),
            kind: Kind::DeepClean,
            title: item.title.clone(),
            detail: item.detail.clone(),
            reason: item.reason.clone(),
            note: item.note.clone(),
            memory: 0,
            cpu: 0.0,
            ports: Vec::new(),
            recommended: item.recommended,
            polite: true,
            verb: "Clean",
            disk: item.bytes,
            group: item.group,
            category: item.category(),
            up_to: item.up_to,
            removal: item.removal.label(),
        }
    }
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeepStatus {
    supported: bool,
    scanning: bool,
    /// Unix seconds of the last finished look.
    scanned_at: Option<u64>,
    /// How far the current look has got (while `scanning`).
    progress: Option<deepclean::Progress>,
    rows: Vec<DeepRow>,
    /// A clean in progress. It runs here, not in the panel, so it keeps
    /// going when the panel closes.
    cleaning: Option<CleanRun>,
    /// The last clean that finished, until the panel has shown it.
    last_clean: Option<CleanDone>,
    /// Rows that couldn't be cleaned in the last run, and why (by key).
    outcomes: HashMap<String, Outcome>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanRun {
    /// Every key in this run, in order.
    keys: Vec<String>,
    /// Keys handled so far (cleaned or not).
    done: usize,
    /// The one being cleaned now.
    current: Option<String>,
    cleared: u64,
    trashed: u64,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanDone {
    /// Deleted outright.
    cleared: u64,
    /// Moved to the Trash: comes back once it's emptied.
    trashed: u64,
    /// How many were cleaned.
    count: usize,
    /// Keys that were cleaned (the panel melts them if it's watching).
    keys: Vec<String>,
    finished_at: u64,
}

/// A finished look is reused for this long; after that, opening Deep clean
/// looks again (folders change as you work).
const DEEP_FRESH_SECS: u64 = 10 * 60;

/// Whether opening Deep clean should start a new look: on first use, when
/// asked, or once the last look is 10 minutes old. Never while one is
/// already running, and never mid-clean (the rows must keep their ids).
fn should_look(d: &DeepStatus, rescan: bool, now: u64) -> bool {
    let stale = d.scanned_at.is_none_or(|t| now.saturating_sub(t) >= DEEP_FRESH_SECS);
    !d.scanning && d.cleaning.is_none() && (rescan || stale)
}

/// Deep clean only looks at the real disk in the real app.
fn deep_supported() -> bool {
    deepclean::supported() && !cfg!(feature = "playground") && e2e_only_pid().is_none()
}

/// Home, resolved, so every path Deep clean checks compares exactly.
fn home_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().home_dir().ok().and_then(|h| deepclean::canonical(&h))
}

/// Deep clean's state; starts a look on first use or when `rescan` is set.
/// The look runs on its own thread (it can take a minute), so the panel
/// polls this until `scanning` is false. It keeps going if the panel closes.
#[tauri::command]
pub fn deep_clean(app: AppHandle, rescan: bool) -> DeepStatus {
    let state = app.state::<AppState>();
    let supported = deep_supported();
    let start = {
        let mut d = lock(&state.deep);
        let start = supported && should_look(&d, rescan, history::now_secs());
        if start {
            d.scanning = true;
            // Why rows weren't cleaned stays until you look again yourself
            // or clean again; a look that starts on its own keeps it.
            if rescan {
                d.outcomes.clear();
            }
            d.progress = Some(deepclean::Progress { step: "Starting", done: 0, total: 0, found: 0, fraction: 0.0 });
        }
        start
    };
    if start {
        let handle = app.clone();
        let spawned = std::thread::Builder::new().name("butters-deep-clean".into()).spawn(move || run_deep_scan(&handle));
        if spawned.is_err() {
            let mut d = lock(&state.deep);
            d.scanning = false;
            d.progress = None;
        }
    }
    let mut status = lock(&state.deep).clone();
    status.supported = supported;
    status
}

fn run_deep_scan(app: &AppHandle) {
    let state = app.state::<AppState>();
    let t0 = Instant::now();
    let (days, ignored) = {
        let s = lock(&state.settings);
        (s.deep_clean_days as u64, s.ignored.clone())
    };
    let items: Vec<deepclean::Item> = match home_dir(app) {
        Some(home) => {
            let deep = &state.deep;
            let report = |p: deepclean::Progress| lock(deep).progress = Some(p);
            deepclean::scan(&home, history::now_secs(), days, deepclean::MIN_BYTES, &deepclean::Procs::current(), &report)
        }
        None => Vec::new(),
    }
    .into_iter()
    .filter(|i| !ignored.contains(&i.key))
    .collect();
    let total: u64 = items.iter().map(|i| i.bytes).sum();
    let ticked: u64 = items.iter().filter(|i| i.recommended).map(|i| i.bytes).sum();
    state.log.info(format!(
        "Deep clean looked in {:.0} s: {} items, {} in total, {} pre-ticked",
        t0.elapsed().as_secs_f32(),
        items.len(),
        fmt_bytes(total),
        fmt_bytes(ticked)
    ));
    let seq = state.seq.fetch_add(1, Ordering::Relaxed);
    let mut rows = Vec::new();
    let mut targets = HashMap::new();
    for (i, item) in items.into_iter().enumerate() {
        state.log.info(format!(
            "Deep clean found \"{}\" ({:?}): {}, {}{}{}",
            item.title,
            item.group,
            item.reason,
            fmt_bytes(item.bytes),
            if item.recommended { ", pre-ticked" } else { "" },
            item.note.as_deref().map(|n| format!(", note: {n}")).unwrap_or_default(),
        ));
        let id = format!("c{seq}-{i}");
        rows.push(DeepRow::new(id.clone(), &item));
        targets.insert(id, Target::Deep { item: Box::new(item) });
    }
    {
        let mut t = lock(&state.targets);
        t.retain(|_, v| !matches!(v, Target::Deep { .. }));
        t.extend(targets);
    }
    // Settings may have changed while it looked: drop what's now ignored,
    // and if the cutoff changed, look again next time.
    let (days_now, ignored_now) = {
        let s = lock(&state.settings);
        (s.deep_clean_days as u64, s.ignored.clone())
    };
    rows.retain(|r| !ignored_now.contains(&r.key));
    let mut d = lock(&state.deep);
    d.rows = rows;
    d.scanning = false;
    d.progress = None;
    d.scanned_at = (days_now == days).then(history::now_secs);
}

/// Cleans one Deep clean item, checking it again first.
fn clean_deep(app: &AppHandle, item: &deepclean::Item) -> Outcome {
    let Some(home) = home_dir(app) else {
        return Outcome::Failed { message: "Couldn't find your home folder".into() };
    };
    match deepclean::clean(item, &home, &deepclean::Procs::current()) {
        Ok(done) => {
            let size = fmt_bytes(done.bytes);
            let bin = if cfg!(windows) { "the Recycle Bin" } else { "the Trash" };
            let mut message = if done.trashed { format!("Moved {size} to {bin}") } else { format!("Cleared {size}") };
            if done.failed > 0 {
                message.push_str(&format!(", {} couldn't be removed", done.failed));
            }
            Outcome::Closed { message, freed: 0, disk: done.bytes }
        }
        Err(deepclean::CleanError::Blocked(message)) => Outcome::Refused { message },
        Err(deepclean::CleanError::Failed(message)) => Outcome::Failed { message },
    }
}

/// Starts cleaning these Deep clean rows, one at a time, on a background
/// thread. The panel watches through `deep_clean`; closing the panel or
/// leaving the view doesn't stop it.
#[tauri::command]
pub fn deep_clean_run(app: AppHandle, ids: Vec<String>) -> Result<DeepStatus, String> {
    let state = app.state::<AppState>();
    let picked: Vec<(String, String)> = {
        let targets = lock(&state.targets);
        ids.iter()
            .filter_map(|id| match targets.get(id) {
                Some(Target::Deep { item }) => Some((id.clone(), item.key.clone())),
                _ => None,
            })
            .collect()
    };
    if picked.is_empty() || picked.len() != ids.len() {
        return Err("List changed. Look again.".into());
    }
    {
        let mut d = lock(&state.deep);
        if d.scanning || d.cleaning.is_some() {
            return Err("Still busy. Try again in a moment.".into());
        }
        d.outcomes.clear();
        d.last_clean = None;
        d.cleaning = Some(CleanRun { keys: picked.iter().map(|(_, k)| k.clone()).collect(), ..CleanRun::default() });
    }
    state.log.info(format!("Deep clean: cleaning {} item{}", picked.len(), if picked.len() == 1 { "" } else { "s" }));
    let handle = app.clone();
    let spawned = std::thread::Builder::new().name("butters-deep-clean-run".into()).spawn(move || run_deep_clean(&handle, picked));
    if let Err(e) = spawned {
        lock(&state.deep).cleaning = None;
        return Err(format!("Couldn't start: {e}"));
    }
    Ok(deep_clean(app, false))
}

fn run_deep_clean(app: &AppHandle, picked: Vec<(String, String)>) {
    let state = app.state::<AppState>();
    let mut done = CleanDone::default();
    for (id, key) in picked {
        if let Some(run) = lock(&state.deep).cleaning.as_mut() {
            run.current = Some(key.clone());
        }
        let removal = match lock(&state.targets).get(&id) {
            Some(Target::Deep { item }) => item.removal.clone(),
            _ => deepclean::Removal::Delete,
        };
        let outcome = close_now(app, &id, false);
        let mut d = lock(&state.deep);
        match &outcome {
            Outcome::Closed { disk, .. } => {
                if removal == deepclean::Removal::Trash {
                    done.trashed += disk;
                } else {
                    done.cleared += disk;
                }
                done.count += 1;
                done.keys.push(key.clone());
            }
            other => {
                d.outcomes.insert(key.clone(), other.clone());
            }
        }
        if let Some(run) = d.cleaning.as_mut() {
            run.done += 1;
            run.current = None;
            run.cleared = done.cleared;
            run.trashed = done.trashed;
        }
    }
    done.finished_at = history::now_secs();
    let summary = format!(
        "{} cleaned up{}",
        fmt_bytes(done.cleared + done.trashed),
        if done.trashed > 0 { format!(", {} of it in the {}", fmt_bytes(done.trashed), if cfg!(windows) { "Recycle Bin" } else { "Trash" }) } else { String::new() }
    );
    state.log.info(format!("Deep clean finished: {} of them, {summary}", done.count));
    let count = done.count;
    {
        let mut d = lock(&state.deep);
        d.cleaning = None;
        d.last_clean = Some(done);
    }
    // Finished while the panel was closed: say so, like any other alert.
    if count > 0 && !state.panel_open.load(Ordering::Relaxed) && lock(&state.settings).alerts {
        crate::permissions::notify(app, &format!("Deep clean done: {summary}"));
    }
}

/// The panel has shown the last clean's result; don't show it again.
#[tauri::command]
pub fn deep_clean_seen(state: State<'_, AppState>) {
    lock(&state.deep).last_clean = None;
}

/// Today's smoothness score and the week's freed memory.
#[tauri::command]
pub fn smoothness(state: State<'_, AppState>) -> Summary {
    lock(&state.history).summary(history::today())
}

/// Live meters: called every couple of seconds while the panel is open.
#[tauri::command]
pub fn live_stats(state: State<'_, AppState>) -> LiveStats {
    let stats = lock(&state.sampler).quick_stats();
    LiveStats { level: alerts::level(&stats), stats }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStats {
    stats: Stats,
    level: Level,
}

#[tauri::command]
pub async fn scan(app: AppHandle) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || do_scan(&app.state::<AppState>())).await.map_err(blocking_err)
}

/// The last Docker/Ollama result, instantly (None before the first one).
/// Its ids stay valid until the next services scan replaces them.
#[tauri::command]
pub fn cached_services(state: State<'_, AppState>) -> Option<ServicesResult> {
    lock(&state.services_cache).clone()
}

#[tauri::command]
pub async fn scan_services(app: AppHandle) -> Result<ServicesResult, String> {
    tauri::async_runtime::spawn_blocking(move || do_scan_services(&app.state::<AppState>())).await.map_err(blocking_err)
}

#[tauri::command]
pub async fn close_item(app: AppHandle, id: String, force: bool) -> Result<Outcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Deep clean items only go through deep_clean_run: one clean at a
        // time, tracked, never alongside a look.
        if matches!(lock(&app.state::<AppState>().targets).get(&id), Some(Target::Deep { .. })) {
            return Outcome::Refused { message: "Use Clean in Deep clean".into() };
        }
        close_now(&app, &id, force)
    })
    .await
    .map_err(blocking_err)
}

/// Closes (or cleans) one item from the latest lists, logs it and adds it
/// to the history. Blocking: call it off the main thread.
fn close_now(app: &AppHandle, id: &str, force: bool) -> Outcome {
    {
        let state = app.state::<AppState>();
        let Some(target) = lock(&state.targets).get(id).cloned() else {
            state.log.warn("Close asked for an item from an old list; told the panel to refresh");
            return Outcome::Refused { message: "List changed. Refresh.".into() };
        };
        let label = target.label();
        state.log.info(format!("Closing \"{label}\"{}", if force { " (force)" } else { "" }));
        let outcome = match &target {
            Target::Procs { target, .. } => {
                let frontmost = lock(&state.front).last_front;
                actions::close(target, force, &Guard::current(frontmost))
            }
            Target::Container { id: cid, name, memory, .. } => match services::docker_stop(cid) {
                Ok(()) => Outcome::Closed { message: format!("Stopped {name}"), freed: *memory, disk: 0 },
                Err(e) => Outcome::Failed { message: format!("Couldn't stop: {e}") },
            },
            Target::Model { name, memory, .. } => match services::ollama_unload(name) {
                Ok(()) => Outcome::Closed { message: format!("Unloaded {name}"), freed: *memory, disk: 0 },
                Err(e) => Outcome::Failed { message: format!("Couldn't unload: {e}") },
            },
            Target::Deep { item } => clean_deep(app, item),
            Target::DockerDisk { which, .. } => {
                let r = match which {
                    DiskWhich::BuildCache => services::docker_prune_build_cache(),
                    DiskWhich::DanglingImages => services::docker_prune_dangling_images(),
                    DiskWhich::OldImages(refs) => services::docker_remove_old_images(refs).map(|(_, bytes)| bytes),
                };
                match r {
                    Ok(bytes) => Outcome::Closed { message: format!("Cleared {}", fmt_bytes(bytes)), freed: 0, disk: bytes },
                    Err(e) => Outcome::Failed { message: format!("Couldn't clear: {e}") },
                }
            }
        };
        let (level, what, message) = outcome_entry(&outcome);
        state.log.write(level, &format!("{what} \"{label}\": {message}"));
        if let Outcome::Closed { freed, disk, .. } = &outcome {
            lock(&state.targets).remove(id);
            let disk_kind = match &target {
                Target::DockerDisk { .. } => Some((Kind::DockerDisk, false)),
                Target::Deep { item } => {
                    lock(&state.deep).rows.retain(|r| r.key != item.key);
                    Some((Kind::DeepClean, item.removal == deepclean::Removal::Trash))
                }
                _ => None,
            };
            if let Some((k, trashed)) = disk_kind {
                let mut h = lock(&state.history);
                h.record_disk(*disk, &kind_name(k), trashed);
                let _ = h.save(&state.history_path);
                return outcome;
            }
            let kind = match &target {
                Target::Procs { kind, .. } => *kind,
                Target::Container { .. } => Kind::DockerContainer,
                Target::Model { .. } => Kind::OllamaModel,
                // Recorded as disk above and returned early.
                Target::DockerDisk { .. } | Target::Deep { .. } => return outcome,
            };
            let kind = kind_name(kind);
            let mut h = lock(&state.history);
            h.record_freed(history::today(), *freed, &kind);
            let _ = h.save(&state.history_path);
        }
        outcome
    }
}

#[tauri::command]
pub fn ignore_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (key, label) = lock(&state.targets).get(&id).map(|t| (t.key().to_string(), t.label())).ok_or("List changed. Refresh.")?;
    state.log.info(format!("Ignoring \"{label}\" from now on"));
    lock(&state.deep).rows.retain(|r| r.key != key);
    let mut s = lock(&state.settings);
    if !s.ignored.contains(&key) {
        s.ignored.push(key);
    }
    let cleaned = s.clone().sanitized();
    *s = cleaned;
    s.save(&state.settings_path).map_err(|e| format!("Couldn't save settings: {e}"))
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    lock(&state.settings).clone()
}

/// Swaps the registered global shortcut from `old` to the current setting.
/// Failing to register (another app owns the keys) is not fatal.
pub fn apply_shortcut(app: &AppHandle, old: &str) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let gs = app.global_shortcut();
    if !old.is_empty() {
        let _ = gs.unregister(old);
    }
    let state = app.state::<AppState>();
    let new = lock(&state.settings).shortcut.clone();
    if !new.is_empty()
        && let Err(e) = gs.register(new.as_str()) {
            state.log.warn(format!("Shortcut {new} unavailable: {e}"));
        }
}

#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    use tauri_plugin_autostart::ManagerExt;
    let new = settings.sanitized();
    let auto = app.autolaunch();
    let currently = auto.is_enabled().unwrap_or(false);
    if new.launch_at_login != currently {
        let r = if new.launch_at_login { auto.enable() } else { auto.disable() };
        r.map_err(|e| {
            state.log.error(format!("Couldn't change login item: {e}"));
            format!("Couldn't change login item: {e}")
        })?;
    }
    let (saved, old_shortcut) = {
        let mut s = lock(&state.settings);
        let old = std::mem::replace(&mut *s, new);
        if *s != old {
            state.log.info(format!("Settings changed: {}", settings_line(&s)));
        }
        if s.deep_clean_days != old.deep_clean_days {
            // What's ticked depends on it: look again next time.
            lock(&state.deep).scanned_at = None;
        }
        s.save(&state.settings_path).map_err(|e| {
            state.log.error(format!("Couldn't save settings: {e}"));
            format!("Couldn't save settings: {e}")
        })?;
        (s.clone(), old.shortcut)
    };
    if saved.shortcut != old_shortcut {
        apply_shortcut(&app, &old_shortcut);
    }
    Ok(saved)
}

#[cfg(feature = "playground")]
#[tauri::command]
pub async fn playground_reset() -> Result<crate::playground::PlaygroundStatus, String> {
    tauri::async_runtime::spawn_blocking(|| crate::playground_state().reset()).await.map_err(blocking_err)?
}

#[cfg(feature = "playground")]
#[tauri::command]
pub fn playground_status() -> crate::playground::PlaygroundStatus {
    crate::playground_state().status()
}

/// Is this the Playground build? (The panel shows its bar only then.)
#[tauri::command]
pub fn is_playground() -> bool {
    cfg!(feature = "playground")
}

/// The panel asks to stay open while it closes things (see panel::hold).
#[tauri::command]
pub fn hold_panel(app: AppHandle, on: bool) {
    crate::panel::hold(&app, on);
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    crate::panel::close(&app);
}

#[tauri::command]
pub fn quit_butters(app: AppHandle) {
    let state = app.state::<AppState>();
    state.log.info("Quit from the panel");
    let _ = lock(&state.history).save(&state.history_path);
    app.exit(0);
}

/// Background loop: note which app is in front every few seconds; once a
/// minute sample processes (leak watch, fan history, alerts); every 10
/// minutes look at Docker's containers.
pub fn spawn_background(app: AppHandle) {
    std::thread::Builder::new()
        .name("butters-watch".into())
        .spawn(move || {
            let mut tick: u64 = 0;
            let mut last_level: Option<Level> = None;
            lock(&app.state::<AppState>().sampler).sample();
            // Debug builds only: fire one real alert through the normal path,
            // so tests can see what a notification looks like.
            #[cfg(debug_assertions)]
            if let Ok(msg) = std::env::var("BUTTERS_E2E_ALERT") {
                std::thread::sleep(std::time::Duration::from_secs(2));
                crate::permissions::notify(&app, &msg);
            }
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let state = app.state::<AppState>();
                lock(&state.front).poll();
                tick += 1;
                if !tick.is_multiple_of(20) {
                    continue;
                }
                let panel_open = state.panel_open.load(Ordering::Relaxed);
                let (stats, readings) = {
                    let mut s = lock(&state.sampler);
                    s.sample();
                    (s.stats(), s.readings())
                };
                // Leak watch + fan history, from the same minute sample.
                let new_leaks = {
                    let mut t = lock(&state.trends);
                    t.record(history::now_secs(), &readings);
                    let mut seen = lock(&state.leak_notified);
                    let leaks = t.leaks();
                    seen.retain(|k| leaks.iter().any(|l| (l.pid, l.start) == *k));
                    leaks.into_iter().filter(|l| seen.insert((l.pid, l.start))).collect::<Vec<_>>()
                };
                let alerts_on = lock(&state.settings).alerts;
                for (i, l) in new_leaks.iter().enumerate() {
                    let msg = format!("{} grew {} in {}", l.label, fmt_bytes(l.growth.bytes), fmt_duration(l.growth.secs.max(60)));
                    // One notification per minute at most; the rest are only logged.
                    let show = i == 0 && alerts_on && !panel_open;
                    state.log.warn(format!("Leak: {msg}{}", alert_suffix(show, alerts_on, panel_open)));
                    if show {
                        crate::permissions::notify(&app, &msg);
                    }
                }
                // Docker activity every 10 minutes, even with the panel closed,
                // so "no activity for 6 days" is measured, not guessed.
                // Docker + Ollama every 10 minutes (and once right after start),
                // even with the panel closed: activity is measured over time,
                // and the panel can show the result the moment it opens.
                if tick == 20 || tick.is_multiple_of(200) {
                    let _ = do_scan_services(&state);
                }
                let level = alerts::level(&stats);
                if last_level != Some(level) {
                    let name = level_name(level);
                    state.log.info(format!("Machine is {name}: {}, pressure {:?}, thermal {:?}", summary_line(&stats), stats.pressure, stats.thermal));
                    last_level = Some(level);
                }
                crate::panel::set_tray_level(&app, level, &stats);
                {
                    let mut h = lock(&state.history);
                    h.record_level(history::today(), level);
                    // Write at most every ~10 minutes; closes save right away.
                    if tick.is_multiple_of(200) {
                        let _ = h.save(&state.history_path);
                    }
                }
                let alert = lock(&state.alerts).check(&stats, Instant::now());
                if let Some((_, msg)) = alert {
                    let show = alerts_on && !panel_open;
                    state.log.warn(format!("Alert: {msg}{}", alert_suffix(show, alerts_on, panel_open)));
                    if show {
                        crate::permissions::notify(&app, msg);
                    }
                }
            }
        })
        .expect("spawn background thread");
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Smooth => "smooth",
        Level::Busy => "busy",
        Level::Strained => "strained",
    }
}

/// Why an alert was or wasn't shown, for the log.
fn alert_suffix(shown: bool, alerts_on: bool, panel_open: bool) -> &'static str {
    if shown {
        " (notified)"
    } else if !alerts_on {
        " (not notified: alerts are off)"
    } else if panel_open {
        " (not notified: panel was open)"
    } else {
        " (not notified)"
    }
}

pub fn summary_line(stats: &Stats) -> String {
    format!("Memory {} of {} · CPU {:.0}%", fmt_bytes(stats.mem_used), fmt_bytes(stats.mem_total), stats.cpu)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftover(root: u32, port: u16, dir: &str) -> Group {
        Group {
            key: format!("dev:server.js:{dir}"),
            kind: Kind::DevServer,
            title: "server.js dev server".into(),
            detail: Some(dir.into()),
            root,
            members: vec![root],
            app_pid: None,
            memory: 30 * MB,
            cpu: 0.0,
            idle_secs: 0,
            idle_is_lower_bound: true,
            orphaned: true,
            ports: vec![port],
            host_app: None,
            host_root: None,
            growth: None,
        }
    }

    #[test]
    fn deep_clean_reuses_a_look_for_ten_minutes() {
        let now = 1_000_000;
        let mut d = DeepStatus::default();
        assert!(should_look(&d, false, now), "first use looks");
        d.scanned_at = Some(now - 9 * 60);
        assert!(!should_look(&d, false, now), "9 minutes old: reused");
        assert!(should_look(&d, true, now), "Look again always looks");
        d.scanned_at = Some(now - 10 * 60);
        assert!(should_look(&d, false, now), "10 minutes old: looks again");
        d.scanning = true;
        assert!(!should_look(&d, true, now), "never two looks at once");
        d.scanning = false;
        d.cleaning = Some(CleanRun::default());
        assert!(!should_look(&d, true, now), "never mid-clean");
    }

    #[test]
    fn many_leftovers_merge_into_one_card() {
        let mut into = leftover(1, 3000, "/a");
        for i in 0..15u32 {
            let next = leftover(10 + i, 4000 + i as u16, "/b");
            assert_eq!(base_title(&into.title), next.title);
            merge_into(&mut into, next);
        }
        assert_eq!(into.title, "server.js dev server ×16");
        assert_eq!(into.members.len(), 16);
        assert_eq!(into.memory, 16 * 30 * MB);
        assert_eq!(into.detail.as_deref(), Some("16 folders"));
        assert_eq!(base_title("Vite ×dev"), "Vite ×dev");
    }
}
