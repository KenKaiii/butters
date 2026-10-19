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
use crate::alerts::{self, AlertState, Level};
use crate::engine::assess::{self, Context, Verdict, fmt_bytes, fmt_duration};
use crate::engine::model::{Group, Kind};
use crate::platform;
use crate::sampler::{self, Sampler, Stats};
use crate::services;
use crate::docker_activity::Activity;
use crate::history::{self, History, Summary};
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
}

impl AppState {
    pub fn new(settings_path: PathBuf) -> Self {
        let dir = settings_path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        let history_path = History::path(&dir);
        let docker_activity_path = Activity::path(&dir);
        AppState {
            sampler: Mutex::new(Sampler::new()),
            front: Mutex::new(FrontTracker::new()),
            targets: Mutex::new(HashMap::new()),
            settings: Mutex::new(Settings::load(&settings_path)),
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
        }
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
    if e2e_only_pid().is_some() {
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
        t.retain(|_, v| matches!(v, Target::Procs { .. }));
        t.extend(targets);
    }
    {
        let mut sw = lock(&state.sweepable);
        sw.retain(|id| id.starts_with('p'));
        sw.extend(suggestions.iter().filter(|s| sweepable(s)).map(|s| s.id.clone()));
    }
    let result = ServicesResult {
        suggestions,
        docker: if containers.is_some() { "running" } else { "off" },
        ollama: if models.is_some() { "running" } else { "off" },
    };
    *lock(&state.services_cache) = Some(result.clone());
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
        let state = app.state::<AppState>();
        let Some(target) = lock(&state.targets).get(&id).cloned() else {
            return Outcome::Refused { message: "List changed. Refresh.".into() };
        };
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
        if let Outcome::Closed { freed, disk, .. } = &outcome {
            lock(&state.targets).remove(&id);
            if let Target::DockerDisk { .. } = &target {
                let mut h = lock(&state.history);
                h.record_disk(*disk);
                let _ = h.save(&state.history_path);
                return outcome;
            }
            let kind = match &target {
                Target::Procs { kind, .. } => *kind,
                Target::Container { .. } => Kind::DockerContainer,
                Target::Model { .. } | Target::DockerDisk { .. } => Kind::OllamaModel,
            };
            let kind = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let mut h = lock(&state.history);
            h.record_freed(history::today(), *freed, &kind);
            let _ = h.save(&state.history_path);
        }
        outcome
    })
    .await
    .map_err(blocking_err)
}

#[tauri::command]
pub fn ignore_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let key = lock(&state.targets).get(&id).map(|t| t.key().to_string()).ok_or("List changed. Refresh.")?;
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
    let new = lock(&app.state::<AppState>().settings).shortcut.clone();
    if !new.is_empty()
        && let Err(e) = gs.register(new.as_str()) {
            eprintln!("shortcut {new} unavailable: {e}");
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
        r.map_err(|e| format!("Couldn't change login item: {e}"))?;
    }
    let (saved, old_shortcut) = {
        let mut s = lock(&state.settings);
        let old = std::mem::replace(&mut *s, new);
        s.save(&state.settings_path).map_err(|e| format!("Couldn't save settings: {e}"))?;
        (s.clone(), old.shortcut)
    };
    if saved.shortcut != old_shortcut {
        apply_shortcut(&app, &old_shortcut);
    }
    Ok(saved)
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    crate::panel::close(&app);
}

#[tauri::command]
pub fn quit_butters(app: AppHandle) {
    let state = app.state::<AppState>();
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
                if let Some(l) = new_leaks.first()
                    && lock(&state.settings).alerts && !panel_open {
                        crate::permissions::notify(
                            &app,
                            &format!("{} grew {} in {}", l.label, fmt_bytes(l.growth.bytes), fmt_duration(l.growth.secs.max(60))),
                        );
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
                if let Some((_, msg)) = alert
                    && lock(&state.settings).alerts && !panel_open {
                        crate::permissions::notify(&app, msg);
                    }
            }
        })
        .expect("spawn background thread");
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
