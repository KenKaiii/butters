//! Turns a flat process list into groups a person recognises, decides which
//! ones are off-limits, and which are worth suggesting. Pure logic only.
//!
//! Safety stance: anything not owned by the user, part of the OS, part of
//! Butters, in use right now, or used recently is protected. Everything
//! else is at most a *suggestion*; nothing here ever acts.

use std::collections::{HashMap, HashSet};

use super::classify::{self, app_name, app_root, classify_app, classify_cli};
use super::model::{Group, Kind, ProcInfo};

const MIN: u64 = 60;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;
const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

/// What Butters knows about the user's situation at scan time.
#[derive(Debug, Default, Clone)]
pub struct Context {
    pub self_pid: u32,
    /// The app the user is in (the last frontmost app that isn't Butters).
    pub frontmost_pid: Option<u32>,
    /// pid -> seconds since that app was last frontmost.
    pub recent_pids: HashMap<u32, u64>,
    /// "Leave this alone" keys chosen by the user.
    pub ignored: HashSet<String>,
    /// How long Butters has been watching. App "unused for an hour" claims
    /// are only made once we've actually watched for that long.
    pub watched_secs: u64,
    /// Running Docker containers, when known.
    pub running_containers: Option<usize>,
    /// True when every running container has been idle for a day or more.
    pub containers_all_idle: Option<bool>,
}

/// An app counts as recently used for this long after it was last in front.
pub const RECENT_SECS: u64 = 15 * MIN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protection {
    ThisApp,
    LaunchedButters,
    InUse,
    RecentlyUsed { mins_ago: u64 },
    InsideAppInUse { app: String },
    System,
    Simulator,
    Ignored,
}

impl Protection {
    /// Whether the user may still choose to close it from the Protected list.
    /// Butters itself, whatever launched it, OS processes and the Simulator
    /// stay locked; "in use"-style protections are advice, not a lock.
    pub fn user_closable(&self) -> bool {
        matches!(
            self,
            Protection::InUse | Protection::RecentlyUsed { .. } | Protection::InsideAppInUse { .. } | Protection::Ignored
        )
    }

    pub fn text(&self) -> String {
        match self {
            Protection::ThisApp => "Butters".into(),
            Protection::LaunchedButters => "Launched Butters".into(),
            Protection::InUse => "In use".into(),
            Protection::RecentlyUsed { mins_ago } if *mins_ago <= 1 => "Used just now".into(),
            Protection::RecentlyUsed { mins_ago } => format!("Used {mins_ago} min ago"),
            Protection::InsideAppInUse { app } => format!("Part of {app}"),
            Protection::System => "System".into(),
            Protection::Simulator => "iOS Simulator".into(),
            Protection::Ignored => "Ignored".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Suggest { reason: String, recommended: bool, note: Option<String> },
    Protected(Protection),
    /// Not worth mentioning.
    Quiet,
}

#[derive(Debug, Clone)]
pub struct Assessed {
    pub group: Group,
    pub verdict: Verdict,
}

struct Tree<'a> {
    by_pid: HashMap<u32, &'a ProcInfo>,
    children: HashMap<u32, Vec<u32>>,
}

impl<'a> Tree<'a> {
    fn new(procs: &'a [ProcInfo]) -> Self {
        let by_pid: HashMap<u32, &ProcInfo> = procs.iter().map(|p| (p.pid, p)).collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for p in procs {
            if let Some(parent) = Self::valid_parent(&by_pid, p) {
                children.entry(parent).or_default().push(p.pid);
            }
        }
        Tree { by_pid, children }
    }

    /// The parent, unless it is gone or its pid was reused by a newer process.
    /// Whole seconds can't tell a parent from a newcomer that took its pid in
    /// the same second; the finer start time can, when both have one.
    fn valid_parent(by_pid: &HashMap<u32, &ProcInfo>, p: &ProcInfo) -> Option<u32> {
        let parent = p.parent?;
        let pp = by_pid.get(&parent)?;
        let older = match (pp.start_us, p.start_us) {
            (Some(a), Some(b)) => a <= b,
            _ => pp.start_time <= p.start_time,
        };
        (pp.pid != p.pid && older).then_some(parent)
    }

    fn parent(&self, pid: u32) -> Option<u32> {
        let p = self.by_pid.get(&pid)?;
        Self::valid_parent(&self.by_pid, p)
    }

    fn ancestors(&self, pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = pid;
        while let Some(parent) = self.parent(cur) {
            if out.contains(&parent) || out.len() > 64 {
                break;
            }
            out.push(parent);
            cur = parent;
        }
        out
    }

    fn subtree(&self, root: u32) -> Vec<u32> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            if let Some(kids) = self.children.get(&out[i]) {
                for k in kids {
                    if !out.contains(k) {
                        out.push(*k);
                    }
                }
            }
            i += 1;
        }
        out
    }
}

/// True when the parent is the init process: the terminal that started this
/// is gone, but the process kept running.
fn orphaned(tree: &Tree, p: &ProcInfo) -> bool {
    match p.parent {
        None | Some(0) | Some(1) => true,
        Some(_) => tree.parent(p.pid).is_none(),
    }
}

/// Daemons that are *meant* to outlive the terminal that started them.
fn is_daemon_by_design(label: &str) -> bool {
    matches!(
        label,
        "Watchman" | "Turborepo daemon" | "Gradle daemon" | "Kotlin daemon" | "eslint_d" | "prettierd" | "Ollama"
    )
}

fn shorten_home(path: &str) -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

pub fn build_groups(procs: &[ProcInfo], self_pid: u32) -> Vec<Group> {
    let tree = Tree::new(procs);
    let mut groups: Vec<Group> = Vec::new();
    let mut claimed: HashSet<u32> = HashSet::new();

    // 1. Desktop apps, by bundle (macOS) or program path (Windows).
    let mut app_members: HashMap<String, Vec<u32>> = HashMap::new();
    let mut app_of: HashMap<u32, String> = HashMap::new();
    for p in procs.iter().filter(|p| p.owned_by_user) {
        // On Windows every .exe would count as an "app"; a recognised tool
        // (claude.exe, codex.exe, an Android emulator, a go-run server) is
        // command-line work instead. macOS apps live in .app bundles, so this
        // only affects plain executables.
        if p.exe.as_deref().is_some_and(|e| !e.contains(".app/")) && classify::classify_cli(p).is_some() {
            continue;
        }
        if let Some(root) = p.exe.as_deref().and_then(app_root) {
            app_members.entry(root.clone()).or_default().push(p.pid);
            app_of.insert(p.pid, root);
        }
    }
    let mut app_group_index: HashMap<String, usize> = HashMap::new();
    let mut roots: Vec<_> = app_members.into_iter().collect();
    roots.sort_by(|a, b| a.0.cmp(&b.0));
    for (root, members) in roots {
        // The app's own process: the earliest member whose parent is not in the app.
        let mut main = members
            .iter()
            .copied()
            .filter(|pid| tree.parent(*pid).is_none_or(|pp| app_of.get(&pp) != Some(&root)))
            .collect::<Vec<_>>();
        main.sort_by_key(|pid| tree.by_pid[pid].start_time);
        let app_pid = main.first().copied().or(members.first().copied());
        let name = app_name(&root);
        let mut ordered = members.clone();
        if let Some(a) = app_pid {
            ordered.retain(|m| *m != a);
            ordered.insert(0, a);
        }
        claimed.extend(ordered.iter().copied());
        app_group_index.insert(root.clone(), groups.len());
        groups.push(Group {
            key: format!("app:{root}"),
            kind: classify_app(&name),
            title: name,
            detail: None,
            root: app_pid.unwrap_or(0),
            members: ordered,
            app_pid,
            memory: 0,
            cpu: 0.0,
            idle_secs: 0,
            idle_is_lower_bound: false,
            orphaned: false,
            ports: Vec::new(),
            host_app: None,
            host_root: None,
            growth: None,
        });
    }

    // 1b. Virtual machines belong to the app responsible for them, so Docker
    // Desktop's VM memory counts as Docker's (and quitting Docker frees it).
    for p in procs {
        if !p.owned_by_user || claimed.contains(&p.pid) {
            continue;
        }
        let Some(owner) = p.responsible else { continue };
        if let Some(g) = groups.iter_mut().find(|g| g.app_pid.is_some() && g.members.contains(&owner)) {
            g.members.push(p.pid);
            claimed.insert(p.pid);
        }
    }

    // 2. Command-line work: agent sessions, dev servers, MCP servers...
    let mut by_start: Vec<&ProcInfo> = procs.iter().filter(|p| p.owned_by_user && !claimed.contains(&p.pid)).collect();
    // Parents before children: start time, then depth (Windows pids aren't
    // in start order and start times have one-second resolution).
    by_start.sort_by_key(|p| (p.start_time, tree.ancestors(p.pid).len(), p.pid));
    for p in by_start {
        if claimed.contains(&p.pid) || p.pid == self_pid {
            continue;
        }
        let Some((kind, label)) = classify_cli(p) else { continue };
        // Lift to the launcher (npm run dev -> vite): closing that is what the user means.
        let mut root = p.pid;
        while let Some(parent) = tree.parent(root) {
            let pp = tree.by_pid[&parent];
            if !pp.owned_by_user || claimed.contains(&parent) || app_of.contains_key(&parent) || !classify::is_wrapper(pp) {
                break;
            }
            root = parent;
        }
        let members: Vec<u32> = tree
            .subtree(root)
            .into_iter()
            .filter(|m| !claimed.contains(m) && !app_of.contains_key(m) && tree.by_pid[m].owned_by_user)
            .collect();
        if members.is_empty() {
            continue;
        }
        // Find the app this runs inside, and whether a shell sits in between.
        let mut via_shell = false;
        let mut host: Option<(String, u32)> = None;
        for anc in tree.ancestors(root) {
            if let Some(app) = app_of.get(&anc) {
                host = Some((app.clone(), anc));
                break;
            }
            if classify::is_interactive_shell(tree.by_pid[&anc]) {
                via_shell = true;
            }
        }
        if let Some((app, _)) = &host {
            let idx = app_group_index[app];
            let app_kind = groups[idx].kind;
            // Helpers an editor/app started itself (no shell in between) are part of that app.
            if !via_shell && app_kind != Kind::Terminal {
                claimed.extend(members.iter().copied());
                groups[idx].members.extend(members);
                continue;
            }
        }
        let rp = tree.by_pid[&root];
        let cwd = p.cwd.as_deref().or(rp.cwd.as_deref()).map(shorten_home);
        claimed.extend(members.iter().copied());
        let orphan = matches!(kind, Kind::AgentSession | Kind::McpServer | Kind::DevServer | Kind::Watcher | Kind::LanguageServer)
            && !is_daemon_by_design(&label)
            && orphaned(&tree, rp);
        groups.push(Group {
            key: format!("{}:{}:{}", kind_slug(kind), label, cwd.clone().unwrap_or_default()),
            kind,
            title: cli_title(kind, &label),
            detail: cwd,
            root,
            members,
            app_pid: None,
            memory: 0,
            cpu: 0.0,
            idle_secs: 0,
            idle_is_lower_bound: false,
            orphaned: orphan,
            ports: Vec::new(),
            host_app: host.as_ref().map(|(a, _)| app_name(a)),
            host_root: host.map(|(_, pid)| pid),
            growth: None,
        });
    }

    // 3. Anything else that is heavy on its own.
    for p in procs {
        if !p.owned_by_user || claimed.contains(&p.pid) || p.pid == self_pid {
            continue;
        }
        // Big, busy, or steadily growing (leak watch measured it climbing).
        if classify::is_shell(p) || !(p.memory >= 400 * MB || p.cpu >= 40.0 || p.growth.is_some()) {
            continue;
        }
        let members: Vec<u32> = tree
            .subtree(p.pid)
            .into_iter()
            .filter(|m| !claimed.contains(m) && !app_of.contains_key(m) && tree.by_pid[m].owned_by_user)
            .collect();
        claimed.extend(members.iter().copied());
        let name = p.name.split_whitespace().next().unwrap_or(&p.name);
        let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name).to_string();
        groups.push(Group {
            key: format!("process:{}", classify::base(&name)),
            kind: Kind::Process,
            title: name,
            detail: p.cwd.as_deref().map(shorten_home),
            root: p.pid,
            members,
            app_pid: None,
            memory: 0,
            cpu: 0.0,
            idle_secs: 0,
            idle_is_lower_bound: false,
            orphaned: false,
            ports: Vec::new(),
            host_app: None,
            host_root: None,
            growth: None,
        });
    }

    // Totals.
    for g in &mut groups {
        let mut idle = u64::MAX;
        for m in &g.members {
            let p = tree.by_pid[m];
            g.memory += p.memory;
            g.cpu += p.cpu;
            for port in &p.ports {
                if !g.ports.contains(port) {
                    g.ports.push(*port);
                }
            }
            if p.idle_secs < idle {
                idle = p.idle_secs;
                g.idle_is_lower_bound = p.idle_is_lower_bound;
            }
            if let Some(gr) = p.growth
                && g.growth.is_none_or(|cur| gr.0 > cur.0) {
                    g.growth = Some(gr);
                }
        }
        g.idle_secs = if idle == u64::MAX { 0 } else { idle };
        g.ports.sort_unstable();
    }
    groups
}

fn kind_slug(k: Kind) -> &'static str {
    match k {
        Kind::AgentSession => "agent",
        Kind::McpServer => "mcp",
        Kind::DevServer => "dev",
        Kind::Watcher => "watch",
        Kind::LanguageServer => "lsp",
        Kind::LocalModel => "model",
        _ => "other",
    }
}

fn cli_title(kind: Kind, label: &str) -> String {
    match kind {
        Kind::AgentSession => format!("{label} session"),
        Kind::McpServer => format!("MCP server: {label}"),
        Kind::DevServer => format!("{label} dev server"),
        _ => label.to_string(),
    }
}

pub fn fmt_duration(secs: u64) -> String {
    if secs >= 2 * DAY {
        format!("{} d", secs / DAY)
    } else if secs >= HOUR {
        let h = secs / HOUR;
        let m = (secs % HOUR) / MIN;
        if m == 0 || h >= 10 { format!("{h} h") } else { format!("{h} h {m} min") }
    } else {
        format!("{} min", (secs / MIN).max(1))
    }
}

pub fn fmt_bytes(b: u64) -> String {
    if b >= GB {
        format!("{:.1} GB", b as f64 / GB as f64)
    } else {
        format!("{} MB", b / MB)
    }
}

fn quiet_for(g: &Group) -> String {
    if g.idle_is_lower_bound {
        format!("Idle {}+", fmt_duration(g.idle_secs))
    } else {
        format!("Idle {}", fmt_duration(g.idle_secs))
    }
}

fn protection(g: &Group, ctx: &Context, self_ancestors: &HashSet<u32>, procs: &HashMap<u32, &ProcInfo>, by_root: &HashMap<u32, &Group>) -> Option<Protection> {
    if g.members.contains(&ctx.self_pid) {
        return Some(Protection::ThisApp);
    }
    if g.members.iter().any(|m| self_ancestors.contains(m)) {
        return Some(Protection::LaunchedButters);
    }
    let root = procs.get(&g.root)?;
    let is_app = g.app_pid.is_some();
    let system = if is_app {
        g.key.contains("/System/Library/") || g.key.contains("/Library/Apple/") || g.key.to_ascii_lowercase().contains("c:\\windows\\")
    } else {
        classify::is_os_critical(root)
    };
    if system {
        return Some(Protection::System);
    }
    if g.key == format!("app:{}", classify::SIMULATOR_ROOT) {
        return Some(Protection::Simulator);
    }
    if let Some(front) = ctx.frontmost_pid
        && g.members.contains(&front) {
            return Some(Protection::InUse);
        }
    if let Some(mins) = g.members.iter().filter_map(|m| ctx.recent_pids.get(m)).min().filter(|s| **s < RECENT_SECS) {
        return Some(Protection::RecentlyUsed { mins_ago: mins / MIN });
    }
    // Work running inside the app in front: protect it, except in terminals,
    // which host many unrelated sessions (only active ones are protected there).
    if let Some(host) = g.host_root.and_then(|h| by_root.get(&h)) {
        let host_in_front = ctx.frontmost_pid.is_some_and(|f| host.members.contains(&f));
        if host_in_front && (host.kind != Kind::Terminal || g.idle_secs < RECENT_SECS) {
            return Some(Protection::InsideAppInUse { app: host.title.clone() });
        }
    }
    if ctx.ignored.contains(&g.key) {
        return Some(Protection::Ignored);
    }
    None
}

/// Decide, for every group, whether it is protected, suggested, or not worth mentioning.
pub fn assess(procs: &[ProcInfo], ctx: &Context) -> Vec<Assessed> {
    let groups = build_groups(procs, ctx.self_pid);
    let tree = Tree::new(procs);
    let self_ancestors: HashSet<u32> = tree.ancestors(ctx.self_pid).into_iter().collect();
    let by_root: HashMap<u32, &Group> = groups.iter().filter(|g| g.app_pid.is_some()).map(|g| (g.root, g)).collect();

    let mut out = Vec::with_capacity(groups.len());
    for g in &groups {
        let verdict = match protection(g, ctx, &self_ancestors, &tree.by_pid, &by_root) {
            Some(p) => Verdict::Protected(p),
            None => suggest(g, ctx, &by_root),
        };
        out.push(Assessed { group: g.clone(), verdict });
    }
    out
}

fn suggest(g: &Group, ctx: &Context, by_root: &HashMap<u32, &Group>) -> Verdict {
    let mem = g.memory;
    let busy = g.cpu >= 50.0;
    let heavy_enough = mem >= 150 * MB || g.cpu >= 15.0;
    let in_front_terminal = g
        .host_root
        .and_then(|h| by_root.get(&h))
        .is_some_and(|host| host.kind == Kind::Terminal && ctx.frontmost_pid.is_some_and(|f| host.members.contains(&f)));
    let note = in_front_terminal.then(|| {
        format!("In open {}, check first", g.host_app.clone().unwrap_or_default())
    });
    let s = |reason: String, recommended: bool| Verdict::Suggest {
        reason,
        recommended: recommended && note.is_none(),
        note: note.clone(),
    };

    // Leftovers are leaks whatever their size: a dozen small ones add up.
    if g.orphaned && (mem >= 5 * MB || g.cpu >= 1.0) {
        let reason = match g.kind {
            Kind::McpServer => "Session ended".to_string(),
            _ => "Terminal closed".to_string(),
        };
        return s(reason, true);
    }

    // Leak watch: memory that keeps climbing is worth a look, whatever it is.
    if let Some((bytes, secs)) = g.growth
        && g.kind != Kind::Terminal {
            return s(format!("Grew {} in {}", fmt_bytes(bytes), fmt_duration(secs.max(MIN))), false);
        }

    match g.kind {
        Kind::AgentSession => {
            if (g.idle_secs >= 30 * MIN && heavy_enough) || (g.idle_secs >= 2 * HOUR && mem >= 50 * MB) {
                return s(quiet_for(g), g.idle_secs >= 2 * HOUR);
            }
        }
        Kind::DevServer => {
            if g.idle_secs >= 30 * MIN && (heavy_enough || !g.ports.is_empty()) {
                return s(quiet_for(g), g.idle_secs >= 2 * HOUR);
            }
        }
        Kind::McpServer | Kind::Watcher | Kind::LanguageServer | Kind::LocalModel => {
            if g.idle_secs >= HOUR && heavy_enough {
                return s(quiet_for(g), false);
            }
        }
        Kind::Terminal => {
            // Quitting a terminal ends every session in it. Never suggested.
            return Verdict::Quiet;
        }
        Kind::ContainerRuntime => {
            if ctx.running_containers == Some(0) && mem >= 500 * MB {
                return s("No containers running".to_string(), true);
            }
            if ctx.containers_all_idle == Some(true) && mem >= 500 * MB {
                return s("All containers idle 1 d+".to_string(), false);
            }
        }
        Kind::Browser | Kind::Editor | Kind::App => {
            let last_used = g.members.iter().filter_map(|m| ctx.recent_pids.get(m)).min().copied();
            let unused_for = last_used.unwrap_or(ctx.watched_secs);
            if ctx.watched_secs >= HOUR && unused_for >= HOUR && mem >= 800 * MB {
                return s(format!("Unused {}", fmt_duration(unused_for)), false);
            }
        }
        Kind::Process => {
            if mem >= 1536 * MB && g.idle_secs >= HOUR {
                return s(quiet_for(g), false);
            }
        }
        Kind::DockerContainer | Kind::DockerDisk | Kind::OllamaModel | Kind::DeepClean => {}
    }

    if busy {
        let cli = g.app_pid.is_none();
        return s(format!("Busy, {:.0}% CPU", g.cpu), cli && g.kind != Kind::Process);
    }
    Verdict::Quiet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, name: &str, cmd: &[&str]) -> ProcInfo {
        ProcInfo {
            pid,
            parent: Some(parent),
            start_time: 1000 + pid as u64,
            name: name.into(),
            exe: Some(format!("/usr/local/bin/{name}")),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            cwd: Some("/Users/me/proj".into()),
            memory: 200 * MB,
            owned_by_user: true,
            ..Default::default()
        }
    }

    fn app(pid: u32, parent: u32, bundle: &str, sub: &str) -> ProcInfo {
        ProcInfo {
            pid,
            parent: Some(parent),
            start_time: 1000 + pid as u64,
            name: sub.into(),
            exe: Some(format!("/Applications/{bundle}.app/Contents/MacOS/{sub}")),
            memory: 300 * MB,
            owned_by_user: true,
            ..Default::default()
        }
    }

    fn launchd() -> ProcInfo {
        ProcInfo { pid: 1, parent: None, start_time: 0, name: "launchd".into(), exe: Some("/sbin/launchd".into()), owned_by_user: false, ..Default::default() }
    }

    fn verdict_of<'a>(out: &'a [Assessed], title: &str) -> &'a Verdict {
        &out.iter().find(|a| a.group.title == title).unwrap_or_else(|| panic!("no group {title}: {:?}", out.iter().map(|a| &a.group.title).collect::<Vec<_>>())).verdict
    }

    /// A terminal with a shell running `npm run dev` (vite) and a Claude Code session.
    fn workstation() -> Vec<ProcInfo> {
        let mut v = vec![
            launchd(),
            app(10, 1, "Ghostty", "ghostty"),
            p(11, 10, "zsh", &["-zsh"]),
            p(12, 11, "npm", &["npm", "run", "dev"]),
            p(13, 12, "node", &["node", "/Users/me/proj/node_modules/vite/bin/vite.js"]),
            p(14, 10, "zsh", &["-zsh"]),
            p(15, 14, "claude", &["claude"]),
            p(16, 15, "node", &["node", "/x/node_modules/.bin/chrome-devtools-mcp"]),
            app(20, 1, "Google Chrome", "Google Chrome"),
            app(21, 20, "Google Chrome", "Google Chrome Helper (Renderer)"),
        ];
        v[4].ports = vec![5173];
        v
    }

    #[test]
    fn groups_launcher_with_its_dev_server_and_session_with_its_mcp() {
        let groups = build_groups(&workstation(), 999);
        let vite = groups.iter().find(|g| g.title == "Vite dev server").unwrap();
        assert_eq!(vite.root, 12, "npm launcher is the root");
        assert_eq!(vite.members, vec![12, 13]);
        assert_eq!(vite.ports, vec![5173]);
        assert_eq!(vite.host_app.as_deref(), Some("Ghostty"));
        let claude = groups.iter().find(|g| g.title == "Claude Code session").unwrap();
        assert_eq!(claude.members, vec![15, 16], "MCP child closes with its session");
        let chrome = groups.iter().find(|g| g.title == "Google Chrome").unwrap();
        assert_eq!(chrome.app_pid, Some(20));
        assert_eq!(chrome.members.len(), 2);
        assert_eq!(chrome.memory, 600 * MB);
    }

    #[test]
    fn frontmost_app_is_protected_even_when_heavy() {
        let mut procs = workstation();
        procs[8].cpu = 300.0;
        procs[8].memory = 6 * GB;
        let ctx = Context { self_pid: 999, frontmost_pid: Some(21), watched_secs: 5 * HOUR, ..Default::default() };
        let out = assess(&procs, &ctx);
        assert_eq!(verdict_of(&out, "Google Chrome"), &Verdict::Protected(Protection::InUse));
    }

    #[test]
    fn recently_used_app_is_protected() {
        let mut procs = workstation();
        procs[8].memory = 6 * GB;
        let mut ctx = Context { self_pid: 999, watched_secs: 5 * HOUR, ..Default::default() };
        ctx.recent_pids.insert(20, 5 * MIN);
        let out = assess(&procs, &ctx);
        assert_eq!(verdict_of(&out, "Google Chrome"), &Verdict::Protected(Protection::RecentlyUsed { mins_ago: 5 }));
    }

    #[test]
    fn unused_heavy_app_is_suggested_only_after_watching_long_enough() {
        let mut procs = workstation();
        procs[8].memory = 2 * GB;
        let ctx = Context { self_pid: 999, watched_secs: 10 * MIN, ..Default::default() };
        assert_eq!(verdict_of(&assess(&procs, &ctx), "Google Chrome"), &Verdict::Quiet);
        let ctx = Context { self_pid: 999, watched_secs: 3 * HOUR, ..Default::default() };
        match verdict_of(&assess(&procs, &ctx), "Google Chrome") {
            Verdict::Suggest { recommended, .. } => assert!(!recommended, "apps are never pre-selected"),
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn butters_and_whatever_launched_it_are_protected() {
        let mut procs = workstation();
        procs.push(p(30, 14, "cargo", &["cargo", "run"]));
        procs.push(p(31, 30, "butters", &["butters"]));
        let ctx = Context { self_pid: 31, ..Default::default() };
        let out = assess(&procs, &ctx);
        // The terminal Butters was launched from is off-limits.
        assert_eq!(verdict_of(&out, "Ghostty"), &Verdict::Protected(Protection::LaunchedButters));
    }

    #[test]
    fn idle_dev_server_is_suggested_and_active_one_is_not() {
        let mut procs = workstation();
        for pr in procs.iter_mut() {
            pr.idle_secs = 3 * HOUR;
        }
        let ctx = Context { self_pid: 999, ..Default::default() };
        match verdict_of(&assess(&procs, &ctx), "Vite dev server") {
            Verdict::Suggest { reason, recommended, note } => {
                assert_eq!(reason, "Idle 3 h");
                assert!(*recommended);
                assert!(note.is_none());
            }
            v => panic!("{v:?}"),
        }
        procs[4].idle_secs = 2 * MIN; // vite served a request recently
        assert_eq!(verdict_of(&assess(&procs, &ctx), "Vite dev server"), &Verdict::Quiet);
    }

    #[test]
    fn sessions_in_the_terminal_in_front() {
        let mut procs = workstation();
        for pr in procs.iter_mut() {
            pr.idle_secs = 3 * HOUR;
        }
        // Terminal is in front: idle sessions are suggested with a caution and never pre-selected.
        let ctx = Context { self_pid: 999, frontmost_pid: Some(10), ..Default::default() };
        match verdict_of(&assess(&procs, &ctx), "Claude Code session") {
            Verdict::Suggest { recommended, note, .. } => {
                assert!(!recommended);
                assert!(note.as_deref().unwrap().contains("Ghostty"));
            }
            v => panic!("{v:?}"),
        }
        // An active session in the terminal in front is protected.
        procs[6].idle_secs = 30;
        assert_eq!(
            verdict_of(&assess(&procs, &ctx), "Claude Code session"),
            &Verdict::Protected(Protection::InsideAppInUse { app: "Ghostty".into() })
        );
    }

    #[test]
    fn editor_helpers_belong_to_the_editor() {
        let procs = vec![
            launchd(),
            app(40, 1, "Cursor", "Cursor"),
            app(41, 40, "Cursor", "Cursor Helper (Plugin)"),
            p(42, 41, "rust-analyzer", &["/Users/me/.cursor/extensions/rust-analyzer"]),
        ];
        let groups = build_groups(&procs, 999);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec![40, 41, 42]);
        let ctx = Context { self_pid: 999, frontmost_pid: Some(40), ..Default::default() };
        assert_eq!(assess(&procs, &ctx)[0].verdict, Verdict::Protected(Protection::InUse));
    }

    #[test]
    fn orphaned_dev_server_and_mcp_server_are_recommended() {
        let mut procs = vec![launchd(), p(50, 1, "node", &["node", "/p/node_modules/next/dist/bin/next", "dev"])];
        procs.push(p(51, 1, "node", &["node", "/x/node_modules/@playwright/mcp/cli.js"]));
        let ctx = Context { self_pid: 999, ..Default::default() };
        let out = assess(&procs, &ctx);
        for title in ["Next.js dev server", "MCP server: playwright"] {
            match verdict_of(&out, title) {
                Verdict::Suggest { recommended, .. } => assert!(*recommended, "{title}"),
                v => panic!("{title}: {v:?}"),
            }
        }
    }

    #[test]
    fn dev_server_whose_npm_launcher_was_orphaned() {
        let mut npm = p(90, 1, "npm run dev", &["npm run dev"]);
        npm.memory = 40 * MB;
        let mut node = p(91, 90, "node", &["node", "server.js"]);
        node.ports = vec![56970];
        node.memory = 15 * MB;
        let out = assess(&[launchd(), npm, node], &Context { self_pid: 999, ..Default::default() });
        let a = out.iter().find(|a| a.group.root == 90).expect("npm is the root");
        assert!(a.group.orphaned);
        assert!(matches!(a.verdict, Verdict::Suggest { recommended: true, .. }));
    }

    #[test]
    fn simulator_apps_are_left_to_xcode() {
        let mut sim = p(95, 1, "pew2", &["pew2"]);
        sim.exe = Some("/Users/me/Library/Developer/CoreSimulator/Devices/X/data/Containers/Bundle/Application/Y/pew2.app/pew2".into());
        sim.cpu = 106.0;
        let out = assess(&[launchd(), sim], &Context { self_pid: 999, ..Default::default() });
        assert_eq!(out[0].verdict, Verdict::Protected(Protection::Simulator));
    }

    #[test]
    fn daemons_by_design_are_not_orphans() {
        let procs = vec![launchd(), p(60, 1, "watchman", &["watchman", "--foreground"])];
        let g = build_groups(&procs, 999);
        assert!(!g[0].orphaned);
    }

    #[test]
    fn other_users_and_os_processes_are_never_grouped_or_suggested() {
        let mut root_proc = p(70, 1, "node", &["node", "/p/node_modules/vite/bin/vite.js"]);
        root_proc.owned_by_user = false;
        root_proc.cpu = 400.0;
        let mut ws = p(71, 1, "WindowServer", &["WindowServer"]);
        ws.cpu = 90.0;
        ws.memory = 2 * GB;
        let out = assess(&[launchd(), root_proc, ws], &Context { self_pid: 999, ..Default::default() });
        assert!(out.iter().all(|a| a.group.root != 70));
        assert!(out.iter().all(|a| a.group.root != 71 || a.verdict == Verdict::Protected(Protection::System)));
    }

    #[test]
    fn reused_parent_pid_is_not_treated_as_parent() {
        // Parent pid 80 now belongs to a *newer* process: the real parent is gone.
        let mut newer = p(80, 1, "zsh", &["-zsh"]);
        newer.start_time = 5000;
        let child = p(81, 80, "node", &["node", "/p/node_modules/vite/bin/vite.js"]);
        let groups = build_groups(&[launchd(), newer, child], 999);
        let vite = groups.iter().find(|g| g.title == "Vite dev server").unwrap();
        assert!(vite.orphaned);
    }

    #[test]
    fn a_pid_reused_in_the_same_second_is_not_the_parent() {
        // The terminal (pid 80) closed and Windows gave its pid to redis,
        // all within the same second the dev server started.
        let mut newer = p(80, 1, "redis-server", &["redis-server"]);
        newer.start_time = 100;
        newer.start_us = Some(100_400_000);
        let mut child = p(81, 80, "node", &["node", "/p/node_modules/vite/bin/vite.js"]);
        child.start_time = 100;
        child.start_us = Some(100_200_000);
        let groups = build_groups(&[launchd(), newer.clone(), child.clone()], 999);
        assert!(groups.iter().find(|g| g.title == "Vite dev server").unwrap().orphaned, "redis isn't its parent");
        // A real parent started first, even in the same second.
        newer.start_us = Some(100_100_000);
        let groups = build_groups(&[launchd(), newer, child], 999);
        assert!(!groups.iter().any(|g| g.title == "Vite dev server" && g.orphaned));
    }

    #[test]
    fn ignored_groups_are_left_alone() {
        let mut procs = workstation();
        for pr in procs.iter_mut() {
            pr.idle_secs = 3 * HOUR;
        }
        let mut ctx = Context { self_pid: 999, ..Default::default() };
        let key = build_groups(&procs, 999).into_iter().find(|g| g.title == "Vite dev server").unwrap().key;
        ctx.ignored.insert(key);
        assert_eq!(verdict_of(&assess(&procs, &ctx), "Vite dev server"), &Verdict::Protected(Protection::Ignored));
    }

    #[test]
    fn terminals_are_never_suggested() {
        let mut procs = workstation();
        procs[1].memory = 4 * GB;
        procs[1].cpu = 90.0;
        let ctx = Context { self_pid: 999, watched_secs: 5 * HOUR, ..Default::default() };
        assert_eq!(verdict_of(&assess(&procs, &ctx), "Ghostty"), &Verdict::Quiet);
    }

    #[test]
    fn docker_desktop_with_no_containers() {
        let procs = vec![launchd(), {
            let mut d = app(90, 1, "Docker", "com.docker.backend");
            d.memory = 2 * GB;
            d
        }];
        let ctx = Context { self_pid: 999, running_containers: Some(0), ..Default::default() };
        assert!(matches!(verdict_of(&assess(&procs, &ctx), "Docker"), Verdict::Suggest { recommended: true, .. }));
        let ctx = Context { self_pid: 999, running_containers: Some(2), ..Default::default() };
        assert_eq!(verdict_of(&assess(&procs, &ctx), "Docker"), &Verdict::Quiet);
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_duration(30), "1 min");
        assert_eq!(fmt_duration(90 * MIN), "1 h 30 min");
        assert_eq!(fmt_duration(11 * HOUR), "11 h");
        assert_eq!(fmt_bytes(1536 * MB), "1.5 GB");
    }

    #[test]
    fn only_advisory_protections_can_be_closed_anyway() {
        assert!(Protection::InUse.user_closable());
        assert!(Protection::RecentlyUsed { mins_ago: 3 }.user_closable());
        assert!(Protection::InsideAppInUse { app: "Ghostty".into() }.user_closable());
        assert!(Protection::Ignored.user_closable());
        for locked in [Protection::ThisApp, Protection::LaunchedButters, Protection::System, Protection::Simulator] {
            assert!(!locked.user_closable(), "{locked:?} must stay locked");
        }
    }

    #[test]
    fn steady_growth_is_suggested_but_never_preticked() {
        let mut node = p(70, 1, "node", &["node", "build.js"]);
        node.memory = 3 * GB;
        node.growth = Some((2 * GB, 40 * MIN));
        let mut fine = p(71, 1, "node", &["node", "other.js"]);
        fine.memory = 3 * GB;
        let out = assess(&[launchd(), node, fine], &Context { self_pid: 999, ..Default::default() });
        let leak = out.iter().find(|a| a.group.root == 70).unwrap();
        match &leak.verdict {
            Verdict::Suggest { reason, recommended, .. } => {
                assert_eq!(reason, "Grew 2.0 GB in 40 min");
                assert!(!recommended);
            }
            v => panic!("{v:?}"),
        }
        assert!(fmt_duration(6 * DAY + 3 * HOUR).starts_with("6 d"));
        assert_eq!(fmt_duration(30 * HOUR), "30 h");
    }

    #[test]
    fn docker_vm_counts_as_docker() {
        let docker = app(60, 1, "Docker", "Docker Desktop");
        let mut vm = p(61, 1, "com.apple.Virtualization.VirtualMachine", &["com.apple.Virtualization.VirtualMachine"]);
        vm.exe = Some("/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine".into());
        vm.memory = 6 * GB;
        vm.responsible = Some(60);
        let ctx = Context { self_pid: 999, running_containers: Some(0), ..Default::default() };
        let out = assess(&[launchd(), docker, vm], &ctx);
        let d = out.iter().find(|a| a.group.title == "Docker").expect("docker group");
        assert!(d.group.members.contains(&61));
        assert!(d.group.memory >= 6 * GB);
        assert!(matches!(d.verdict, Verdict::Suggest { recommended: true, .. }));
        assert!(!out.iter().any(|a| a.group.root == 61), "VM is not its own (system) item");
    }

    // ---- Windows: the same rules with Windows paths and .exe tools ----

    fn win(pid: u32, parent: u32, name: &str, exe: &str, cmd: &[&str]) -> ProcInfo {
        ProcInfo { exe: Some(exe.into()), ..p(pid, parent, name, cmd) }
    }

    #[test]
    fn windows_cli_tools_are_not_desktop_apps() {
        // Parent 4242 is gone (terminal closed): orphans.
        let claude = {
            let mut c = win(80, 4242, "claude.exe", r"C:\Users\me\.local\bin\claude.exe", &["claude"]);
            c.memory = 300 * MB;
            c
        };
        let emu = {
            let mut e = win(81, 4242, "qemu-system-x86_64.exe", r"C:\Users\me\AppData\Local\Android\Sdk\emulator\qemu\windows-x86_64\qemu-system-x86_64.exe", &["qemu-system-x86_64.exe", "-avd", "Pixel_8"]);
            e.memory = 2 * GB;
            e.idle_secs = 3 * HOUR;
            e
        };
        let go = {
            let mut g = win(82, 4242, "api.exe", r"C:\Users\me\AppData\Local\Temp\go-build3412345\b001\exe\api.exe", &["api.exe"]);
            g.ports = vec![8080];
            g.memory = 40 * MB;
            g
        };
        let slack = win(83, 4242, "slack.exe", r"C:\Users\me\AppData\Local\slack\app-4.41.0\slack.exe", &["slack.exe"]);
        let out = assess(&[claude, emu, go, slack], &Context { self_pid: 999, watched_secs: 4 * HOUR, ..Default::default() });
        let find = |root: u32| out.iter().find(|a| a.group.root == root).unwrap();
        assert_eq!(find(80).group.kind, Kind::AgentSession);
        assert!(matches!(find(80).verdict, Verdict::Suggest { recommended: true, .. }), "orphaned claude.exe");
        assert_eq!(find(81).group.title, "Android emulator");
        assert!(matches!(find(81).verdict, Verdict::Suggest { .. }));
        assert_eq!(find(82).group.kind, Kind::DevServer);
        assert!(matches!(find(82).verdict, Verdict::Suggest { recommended: true, .. }));
        // A real Windows app is still an app.
        assert_eq!(find(83).group.kind, Kind::App);
    }

    #[test]
    fn windows_python_and_npm_chains() {
        // py.exe -m uvicorn -> python.exe child, terminal closed.
        let launcher = win(90, 4242, "py.exe", r"C:\Windows\py.exe", &["py", "-m", "uvicorn", "main:app"]);
        let mut child = win(91, 90, "python.exe", r"C:\Users\me\AppData\Local\Programs\Python\Python313\python.exe", &["python.exe", "-m", "uvicorn", "main:app"]);
        child.ports = vec![8000];
        child.memory = 70 * MB;
        // npm run dev on Windows: node npm-cli.js -> cmd /d /s /c vite -> node vite.js
        let npm = win(92, 4242, "node.exe", r"C:\Program Files\nodejs\node.exe", &["node", r"C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js", "run", "dev"]);
        let glue = win(93, 92, "cmd.exe", r"C:\Windows\System32\cmd.exe", &["cmd.exe", "/d", "/s", "/c", "vite"]);
        let mut vite = win(94, 93, "node.exe", r"C:\Program Files\nodejs\node.exe", &["node", r"C:\work\shop\node_modules\vite\bin\vite.js"]);
        vite.ports = vec![5173];
        vite.memory = 90 * MB;
        let out = assess(&[launcher, child, npm, glue, vite], &Context { self_pid: 999, ..Default::default() });
        let uv = out.iter().find(|a| a.group.title == "Uvicorn dev server").expect("uvicorn");
        assert!(uv.group.members.contains(&91));
        assert!(matches!(uv.verdict, Verdict::Suggest { recommended: true, .. }));
        let v = out.iter().find(|a| a.group.title == "Vite dev server").expect("vite");
        assert_eq!(v.group.root, 92, "closing npm closes the whole chain");
        assert!(v.group.members.contains(&94));
        assert!(matches!(v.verdict, Verdict::Suggest { recommended: true, .. }));
    }

    #[test]
    fn a_leak_is_flagged_while_still_small() {
        // Leak watch saw it grow 1.1 GB, but right now it holds only 300 MB
        // (it just got trimmed/compressed): still worth a look.
        let mut w = p(95, 1, "node", &["node", "/x/worker.js"]);
        w.parent = Some(9999); // in a terminal, not an orphan
        w.memory = 300 * MB;
        w.growth = Some((1_100 * MB, 25 * MIN));
        let out = assess(&[launchd(), w], &Context { self_pid: 999, ..Default::default() });
        let a = out.iter().find(|a| a.group.root == 95).expect("leaking process is a candidate");
        assert!(matches!(&a.verdict, Verdict::Suggest { reason, recommended: false, .. } if reason.starts_with("Grew 1.1 GB")));
    }

    #[test]
    fn child_with_a_lower_pid_still_joins_its_parent_group() {
        // Windows: next-server (child) got a lower pid than `next dev` (parent),
        // and both started in the same second.
        let mut parent = p(500, 4242, "node.exe", &["node", "/x/node_modules/next/dist/bin/next", "dev"]);
        parent.start_time = 100;
        let mut child = p(120, 500, "node.exe", &["next-server (v15.5.0)", "-e", "x"]);
        child.start_time = 100;
        child.ports = vec![3000];
        child.memory = 300 * MB;
        let out = assess(&[parent, child], &Context { self_pid: 999, ..Default::default() });
        let nexts: Vec<_> = out.iter().filter(|a| a.group.title == "Next.js dev server").collect();
        assert_eq!(nexts.len(), 1, "one row, not two");
        assert_eq!(nexts[0].group.root, 500);
        assert!(nexts[0].group.members.contains(&120));
    }
}
