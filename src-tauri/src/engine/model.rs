//! Plain data types shared by the engine. Nothing in here touches the OS, so
//! every rule built on top of these can be unit-tested with fake processes.

use serde::Serialize;

/// One process as the engine sees it, already reduced to what the rules need.
#[derive(Debug, Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub parent: Option<u32>,
    /// Seconds since the Unix epoch. Together with `pid` this identifies a
    /// process even if the OS later hands the same pid to something else.
    pub start_time: u64,
    /// Start time in microseconds, when the OS knows it finer than a second.
    /// Tells a real parent from a newer process that got a dead parent's pid
    /// in the same second (Windows reuses pids quickly).
    pub start_us: Option<u64>,
    pub name: String,
    pub exe: Option<String>,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    /// Resident memory in bytes.
    pub memory: u64,
    /// Recent CPU use in percent of one core (can exceed 100 on multi-core).
    pub cpu: f32,
    /// Seconds since this process last did noticeable work (CPU or terminal I/O).
    pub idle_secs: u64,
    /// True when `idle_secs` only counts from when Butters started watching.
    pub idle_is_lower_bound: bool,
    /// Owned by the signed-in user (we never touch anyone else's processes).
    pub owned_by_user: bool,
    pub ports: Vec<u16>,
    /// Steady memory growth (bytes, over seconds) seen by leak watch.
    pub growth: Option<(u64, u64)>,
    /// For virtual machines: the app macOS holds responsible (Docker, UTM...).
    pub responsible: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    AgentSession,
    McpServer,
    DevServer,
    Watcher,
    LanguageServer,
    LocalModel,
    Browser,
    Terminal,
    Editor,
    ContainerRuntime,
    App,
    Process,
    DockerContainer,
    DockerDisk,
    OllamaModel,
    /// Leftovers on disk from the Deep clean view.
    DeepClean,
}

/// A set of processes the user would think of as one thing: "Chrome",
/// "the Vite server for my-app", "that Claude Code session".
#[derive(Debug, Clone)]
pub struct Group {
    /// Stable identity used for "never suggest this again".
    pub key: String,
    pub kind: Kind,
    pub title: String,
    pub detail: Option<String>,
    pub root: u32,
    /// Every pid in the group, root first.
    pub members: Vec<u32>,
    /// For desktop apps: the pid that owns the app (gets the polite quit).
    pub app_pid: Option<u32>,
    pub memory: u64,
    pub cpu: f32,
    pub idle_secs: u64,
    pub idle_is_lower_bound: bool,
    pub orphaned: bool,
    pub ports: Vec<u16>,
    /// Name of the app this runs inside (e.g. the terminal), if any.
    pub host_app: Option<String>,
    /// pid of the app group this runs inside, if any.
    pub host_root: Option<u32>,
    /// Biggest steady memory growth among its processes.
    pub growth: Option<(u64, u64)>,
}
