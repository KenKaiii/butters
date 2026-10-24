//! Deep clean: developer leftovers on disk that pile up over months. Package
//! caches, the build folders of projects you stopped working on and, on a
//! Mac, Xcode build files, simulators nobody opens and old iPhone support
//! files. Works on macOS, Windows and Linux, each with its own cache folders
//! and its own Trash (the Recycle Bin on Windows).
//!
//! Three groups, each with its own way out:
//! - Rebuilds itself: caches the tools fetch or rebuild on their own. Deleted.
//! - Unused for months: judged by real use (a simulator's last boot, a
//!   project's newest change or commit), never by a build folder's own date.
//!   Moved to the Trash, except what Xcode recreates by itself (old device
//!   files) and simulators (only `simctl` can remove those, and it deletes).
//! - Yours to decide: slow or impossible to get back (Xcode archives,
//!   downloaded models, Android emulators). Never pre-ticked. To the Trash.
//!
//! Safety: only fixed locations, or folders proven by a marker file (Cargo's
//! CACHEDIR.TAG, a package.json next to node_modules, pyvenv.cfg...). Never a
//! symlink, never outside home, never while the tool that owns it is running
//! or anything is working inside the project. All of it is checked again
//! right before anything is removed. Scans only run when the user asks.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(target_os = "macos")]
use crate::services::run_command_raw;
use crate::services::{parse_rfc3339_secs, run_command};

const MB: u64 = 1024 * 1024;
const DAY: u64 = 86_400;
/// Smaller than this isn't worth a row.
pub const MIN_BYTES: u64 = 100 * MB;
/// "Unused" things show once they've been quiet this long (or the user's
/// cutoff, if shorter), unticked until the cutoff.
const SHOW_AFTER: u64 = 30 * DAY;

/// Where Deep clean knows the tool folders and how to reach the Trash.
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", windows, target_os = "linux"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Rebuilds,
    Unused,
    Yours,
}

/// Which tab a row lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Package and build caches.
    Caches,
    /// Build folders of projects.
    Projects,
    /// Simulators, phone support files, archives, Android emulators.
    Mobile,
    /// Downloaded AI models.
    Models,
}

/// How far a look has got, for the panel's progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub step: &'static str,
    /// Within the step, when it can be counted ("12 of 80").
    pub done: usize,
    pub total: usize,
    /// Disk measured so far.
    pub found: u64,
    /// 0..1 for the whole look.
    pub fraction: f32,
}

impl Progress {
    fn at(step: &'static str, fraction: f32) -> Progress {
        Progress { step, done: 0, total: 0, found: 0, fraction }
    }
}

/// The share of the bar each stage gets (measuring is the slow part).
const FIND_END: f32 = 0.1;
const PROJECTS_END: f32 = 0.3;

#[derive(Debug, Clone, PartialEq)]
pub enum Removal {
    /// Deleted outright: the tool fetches or rebuilds it when needed.
    Delete,
    /// Moved to the Trash, so it can be put back.
    Trash,
    /// `pnpm store prune`: removes only what no project on disk uses.
    PnpmPrune(PathBuf),
    /// `xcrun simctl delete <udid>`: the only safe way to remove a simulator.
    Simulator(String),
}

impl Removal {
    /// What the panel tells the user happens to it.
    pub fn label(&self) -> &'static str {
        match self {
            Removal::Trash => "trash",
            _ => "delete",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Stable identity, also what "Ignore" remembers.
    pub key: String,
    pub group: Group,
    pub title: String,
    pub detail: Option<String>,
    pub reason: String,
    pub note: Option<String>,
    pub bytes: u64,
    /// The size is a ceiling, not a promise (pnpm keeps what projects use).
    pub up_to: bool,
    pub recommended: bool,
    pub paths: Vec<PathBuf>,
    /// Per path: a file that must still exist when cleaning, proving the
    /// folder is what it was when scanned.
    pub markers: Vec<Option<PathBuf>>,
    pub sizes: Vec<u64>,
    pub removal: Removal,
    /// Programs that must not be running while this is cleaned.
    pub blockers: Vec<&'static str>,
    /// Projects: nothing may be working inside this folder.
    pub busy_root: Option<PathBuf>,
    /// Projects: when it was last worked on, as of the scan. Any newer work
    /// found when cleaning stops the clean.
    pub last_used: Option<u64>,
    /// Per path: (device, inode) as scanned, so cleaning can tell the very
    /// same folder from one put in its place (Unix; None elsewhere).
    pub ids: Vec<Option<(u64, u64)>>,
}

impl Item {
    pub fn category(&self) -> Category {
        let k = self.key.as_str();
        if k.starts_with("project:") {
            Category::Projects
        } else if k.starts_with("sim:") || k.starts_with("devsupport:") || k.starts_with("avd:") || k == "xcode:archives" {
            Category::Mobile
        } else if k.starts_with("hf:") {
            Category::Models
        } else {
            Category::Caches
        }
    }

    fn new(key: String, group: Group, title: String, removal: Removal) -> Item {
        Item {
            key,
            group,
            title,
            detail: None,
            reason: String::new(),
            note: None,
            bytes: 0,
            up_to: false,
            recommended: false,
            paths: Vec::new(),
            markers: Vec::new(),
            sizes: Vec::new(),
            removal,
            blockers: Vec::new(),
            busy_root: None,
            last_used: None,
            ids: Vec::new(),
        }
    }

    fn path(mut self, p: PathBuf, marker: Option<PathBuf>) -> Item {
        self.ids.push(identity(&p));
        self.paths.push(p);
        self.markers.push(marker);
        self
    }
}

/// What's running right now, reduced to what Deep clean checks: program
/// names and working folders.
pub struct Procs {
    list: Vec<Running>,
    /// False when open files couldn't be read: then "in use" can't be ruled
    /// out, so nothing is ticked and nothing is cleaned.
    open_known: bool,
}

struct Running {
    label: String,
    /// Lower-case names it could go by: process name, executable, and the
    /// first two command-line words (npm runs as `node .../npm`).
    names: Vec<String>,
    /// Everything it is using: working folder, program, absolute paths on
    /// its command line, and (macOS, Linux) every file it has open or
    /// mapped. A dev server or editor holding node_modules shows up here
    /// even when its working folder is somewhere else.
    paths: Vec<PathBuf>,
    /// One of macOS's own services: its open files don't count as use.
    system: bool,
}

impl Procs {
    pub fn current() -> Procs {
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::Always)
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::Always)
                .without_tasks(),
        );
        let me = std::process::id();
        let open = open_files();
        let open_known = open.is_some();
        let mut open = open.unwrap_or_default();
        let list = sys
            .processes()
            .iter()
            .filter(|(pid, _)| pid.as_u32() != me)
            .map(|(pid, p)| {
                let mut names = vec![program_name(&p.name().to_string_lossy())];
                if let Some(e) = p.exe().and_then(Path::file_name) {
                    names.push(program_name(&e.to_string_lossy()));
                }
                for arg in p.cmd().iter().take(2) {
                    if let Some(n) = Path::new(arg).file_name() {
                        names.push(program_name(&n.to_string_lossy()));
                    }
                }
                let mut paths: Vec<PathBuf> = p.cwd().into_iter().chain(p.exe()).map(Path::to_path_buf).collect();
                paths.extend(p.cmd().iter().map(Path::new).filter(|a| a.is_absolute()).map(Path::to_path_buf));
                paths.extend(open.remove(&pid.as_u32()).unwrap_or_default());
                Running { label: p.name().to_string_lossy().into_owned(), names, paths, system: is_system_program(p.exe()) }
            })
            .collect();
        Procs { list, open_known }
    }

    /// Test processes: a name and one path each is using (or none).
    #[cfg(all(test, unix))]
    fn fake(list: &[(&str, Option<&Path>)]) -> Procs {
        Procs {
            list: list
                .iter()
                .map(|(n, path)| Running {
                    label: n.to_string(),
                    names: vec![n.to_lowercase()],
                    paths: path.map(Path::to_path_buf).into_iter().collect(),
                    system: false,
                })
                .collect(),
            open_known: true,
        }
    }

    /// The first of `blockers` that is running.
    fn running(&self, blockers: &[&str]) -> Option<String> {
        blockers
            .iter()
            .find(|b| {
                let b = b.to_lowercase();
                self.list.iter().any(|p| p.names.contains(&b))
            })
            .map(|b| b.to_string())
    }

    /// Something using a file or folder inside `root` (open, mapped, run,
    /// named on its command line, or its working folder).
    fn using(&self, root: &Path) -> Option<String> {
        self.list.iter().find(|p| !p.system && p.paths.iter().any(|f| inside(f, root))).map(|p| p.label.clone())
    }
}

/// macOS's own background services (Spotlight, storage and backup
/// scanners) read files everywhere; that's not you using them, so they never
/// block a clean. Only their system folders count: tools you run yourself
/// from /usr/bin (git, ruby, make) still do.
fn is_system_program(exe: Option<&Path>) -> bool {
    cfg!(target_os = "macos")
        && exe.is_some_and(|e| ["/System/", "/usr/libexec/", "/usr/sbin/", "/Library/Apple/"].iter().any(|d| e.starts_with(d)))
}

/// Every file your processes have open or mapped, by pid. None if that
/// couldn't be found out.
#[cfg(target_os = "macos")]
fn open_files() -> Option<HashMap<u32, Vec<PathBuf>>> {
    let mut cmd = Command::new("/usr/sbin/lsof");
    // Only your own processes, names only, no network lookups, no warnings.
    // SAFETY: getuid has no preconditions and can't fail.
    let uid = unsafe { libc::getuid() };
    cmd.args(["-nP", "-w", "-Fn", "-u", &uid.to_string()]);
    // lsof can exit non-zero over a process it couldn't read; its output is
    // still good. No output at all means it didn't work.
    let (_, out) = run_command_raw(cmd, Duration::from_secs(20), 256 * 1024 * 1024).ok()?;
    let files = parse_lsof(&out);
    (!files.is_empty()).then_some(files)
}

/// `lsof -Fn` output: `p<pid>` starts a process, `n<path>` is a file it
/// uses. lsof writes bytes outside printable ASCII as `\xNN` ("café" is
/// "caf\xc3\xa9"); those are turned back into the real bytes.
#[cfg(target_os = "macos")]
fn parse_lsof(out: &[u8]) -> HashMap<u32, Vec<PathBuf>> {
    use std::os::unix::ffi::OsStrExt;
    let mut files: HashMap<u32, Vec<PathBuf>> = HashMap::new();
    let mut pid = None;
    for line in out.split(|b| *b == b'\n') {
        match line.split_first() {
            Some((b'p', n)) => pid = std::str::from_utf8(n).ok().and_then(|n| n.parse().ok()),
            Some((b'n', path)) if path.first() == Some(&b'/') => {
                let Some(pid) = pid else { continue };
                // Deleted-but-open files and sockets carry a note after the path.
                let end = path.windows(2).position(|w| w == b" (").unwrap_or(path.len());
                let raw = unescape_lsof(&path[..end]);
                files.entry(pid).or_default().push(PathBuf::from(std::ffi::OsStr::from_bytes(&raw)));
            }
            _ => {}
        }
    }
    files
}

#[cfg(target_os = "macos")]
fn unescape_lsof(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' && s.get(i + 1) == Some(&b'x')
            && let (Some(hi), Some(lo)) = (s.get(i + 2).and_then(|b| hex(*b)), s.get(i + 3).and_then(|b| hex(*b)))
        {
            out.push(hi << 4 | lo);
            i += 4;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Every file your processes have open or mapped, by pid, from /proc.
#[cfg(target_os = "linux")]
fn open_files() -> Option<HashMap<u32, Vec<PathBuf>>> {
    let mut files: HashMap<u32, Vec<PathBuf>> = HashMap::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let dir = entry.path();
        let mut list: Vec<PathBuf> = std::fs::read_dir(dir.join("fd"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|fd| std::fs::read_link(fd.path()).ok())
            .filter(|p| p.is_absolute())
            .collect();
        // Mapped files (libraries, native modules), read as bytes so any
        // file name works.
        if let Ok(maps) = std::fs::read(dir.join("maps")) {
            use std::os::unix::ffi::OsStrExt;
            for line in maps.split(|b| *b == b'\n') {
                let Some(i) = line.iter().position(|b| *b == b'/') else { continue };
                let path = line[i..].strip_suffix(b" (deleted)").unwrap_or(&line[i..]);
                list.push(PathBuf::from(std::ffi::OsStr::from_bytes(path)));
            }
        }
        if !list.is_empty() {
            files.insert(pid, list);
        }
    }
    // At least our own process always has files open.
    (!files.is_empty()).then_some(files)
}

/// Windows has no cheap way to list open files; there, cleaning first sets
/// the whole folder aside in one rename, which Windows refuses while
/// anything inside is open (see `set_aside`).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn open_files() -> Option<HashMap<u32, Vec<PathBuf>>> {
    Some(HashMap::new())
}

/// "Node.exe" -> "node": how a program is matched on every OS.
fn program_name(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(stem) => stem.to_string(),
        None => lower,
    }
}

/// `p` is `root` or inside it. Windows paths ignore letter case.
fn inside(p: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let (p, root) = (p.to_string_lossy().to_lowercase(), root.to_string_lossy().to_lowercase());
        let root = root.trim_end_matches('\\');
        p == root || p.strip_prefix(root).is_some_and(|rest| rest.starts_with('\\'))
    } else {
        p.starts_with(root)
    }
}

/// The real, absolute form of a path, written the way the rest of the
/// system writes it (no `\\?\` prefix on Windows).
pub fn canonical(p: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(p).ok()?;
    #[cfg(windows)]
    {
        let s = real.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\")
            && rest.as_bytes().get(1) == Some(&b':')
        {
            return Some(PathBuf::from(rest));
        }
    }
    Some(real)
}

/// `home` plus a relative path written with `/`, built from real components.
fn under(home: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(home.to_path_buf(), |p, part| p.join(part))
}

struct Ctx<'a> {
    home: &'a Path,
    now: u64,
    cutoff: u64,
    show_after: u64,
    procs: &'a Procs,
}

impl Ctx<'_> {
    fn age(&self, t: u64) -> u64 {
        self.now.saturating_sub(t)
    }

    fn tilde(&self, p: &Path) -> String {
        match p.strip_prefix(self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        }
    }
}

/// Everything worth offering, measured, largest first within each group.
/// `cutoff_days`: unused this long gets pre-ticked. `report` hears how far
/// the look has got (it may be called from several threads).
pub fn scan(home: &Path, now: u64, cutoff_days: u64, min_bytes: u64, procs: &Procs, report: &(dyn Fn(Progress) + Sync)) -> Vec<Item> {
    if !supported() {
        return Vec::new();
    }
    let cutoff = cutoff_days.max(1) * DAY;
    let c = Ctx { home, now, cutoff, show_after: cutoff.min(SHOW_AFTER), procs };
    type Step = fn(&Ctx) -> Vec<Item>;
    let steps: [(&'static str, Step); 7] = [
        ("Looking for caches", |c| caches(c).into_iter().chain(pnpm(c)).collect()),
        ("Checking simulators", simulators),
        ("Checking old device files", device_support),
        ("Checking test browsers", test_browsers),
        ("Checking Xcode archives", |c| archives(c).into_iter().collect()),
        ("Checking downloaded models", models),
        ("Checking emulators", emulators),
    ];
    let mut items = Vec::new();
    for (i, (step, find)) in steps.iter().enumerate() {
        report(Progress::at(step, FIND_END * i as f32 / steps.len() as f32));
        items.extend(find(&c));
    }
    items.extend(projects(&c, report));
    for item in items.iter_mut() {
        mark_in_use(item, procs);
    }
    measure(&mut items, report);
    items.retain(|i| i.bytes >= min_bytes);
    items.sort_by(|a, b| a.group.cmp(&b.group).then(b.recommended.cmp(&a.recommended)).then(b.bytes.cmp(&a.bytes)).then(a.key.cmp(&b.key)));
    items
}

// ---- What gets offered ----

struct Cache {
    key: &'static str,
    title: &'static str,
    paths: &'static [&'static str],
    blockers: &'static [&'static str],
    reason: &'static str,
}

const DOWNLOADS: &str = "Downloads again when needed";

/// Caches the tools refill by themselves, on a Mac. Fixed locations only.
const CACHES_MAC: &[Cache] = &[
    Cache { key: "xcode:derived-data", title: "Xcode build files", paths: &["Library/Developer/Xcode/DerivedData"], blockers: &["Xcode", "xcodebuild"], reason: "Rebuilds on the next build" },
    Cache { key: "cache:npm", title: "npm cache", paths: &[".npm/_cacache"], blockers: NPM, reason: DOWNLOADS },
    Cache { key: "cache:bun", title: "Bun cache", paths: &[".bun/install/cache"], blockers: &["bun"], reason: DOWNLOADS },
    Cache { key: "cache:uv", title: "uv cache", paths: &[".cache/uv"], blockers: &["uv", "uvx"], reason: DOWNLOADS },
    Cache { key: "cache:pip", title: "pip cache", paths: &["Library/Caches/pip"], blockers: &["pip", "pip3"], reason: DOWNLOADS },
    Cache { key: "cache:yarn", title: "Yarn cache", paths: &["Library/Caches/Yarn"], blockers: &["yarn"], reason: DOWNLOADS },
    CARGO,
    Cache { key: "cache:go", title: "Go build cache", paths: &["Library/Caches/go-build"], blockers: &["go"], reason: "Rebuilds on the next build" },
    GRADLE,
    Cache { key: "cache:cocoapods", title: "CocoaPods cache", paths: &["Library/Caches/CocoaPods"], blockers: &["pod"], reason: DOWNLOADS },
    Cache { key: "cache:homebrew", title: "Homebrew downloads", paths: &["Library/Caches/Homebrew"], blockers: &["brew"], reason: DOWNLOADS },
    Cache { key: "cache:electron", title: "Electron downloads", paths: &["Library/Caches/electron"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "cache:node-gyp", title: "node-gyp headers", paths: &["Library/Caches/node-gyp"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "cache:typescript", title: "TypeScript type cache", paths: &["Library/Caches/typescript"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "xcode:sim-caches", title: "Simulator caches", paths: &["Library/Developer/CoreSimulator/Caches"], blockers: &["Simulator"], reason: "Rebuilt when needed" },
    NUGET,
];

/// The same caches where Windows keeps them (mostly %LOCALAPPDATA%).
const CACHES_WINDOWS: &[Cache] = &[
    Cache { key: "cache:npm", title: "npm cache", paths: &["AppData/Local/npm-cache/_cacache"], blockers: NPM, reason: DOWNLOADS },
    Cache { key: "cache:bun", title: "Bun cache", paths: &[".bun/install/cache"], blockers: &["bun"], reason: DOWNLOADS },
    Cache { key: "cache:uv", title: "uv cache", paths: &["AppData/Local/uv/cache"], blockers: &["uv", "uvx"], reason: DOWNLOADS },
    Cache { key: "cache:pip", title: "pip cache", paths: &["AppData/Local/pip/cache"], blockers: &["pip", "pip3"], reason: DOWNLOADS },
    Cache { key: "cache:yarn", title: "Yarn cache", paths: &["AppData/Local/Yarn/Cache"], blockers: &["yarn"], reason: DOWNLOADS },
    CARGO,
    Cache { key: "cache:go", title: "Go build cache", paths: &["AppData/Local/go-build"], blockers: &["go"], reason: "Rebuilds on the next build" },
    GRADLE,
    NUGET,
    Cache { key: "cache:electron", title: "Electron downloads", paths: &["AppData/Local/electron/Cache"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "cache:node-gyp", title: "node-gyp headers", paths: &["AppData/Local/node-gyp/Cache"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "cache:typescript", title: "TypeScript type cache", paths: &["AppData/Local/Microsoft/TypeScript"], blockers: &[], reason: DOWNLOADS },
];

/// The same caches where Linux keeps them (mostly ~/.cache).
const CACHES_LINUX: &[Cache] = &[
    Cache { key: "cache:npm", title: "npm cache", paths: &[".npm/_cacache"], blockers: NPM, reason: DOWNLOADS },
    Cache { key: "cache:bun", title: "Bun cache", paths: &[".bun/install/cache"], blockers: &["bun"], reason: DOWNLOADS },
    Cache { key: "cache:uv", title: "uv cache", paths: &[".cache/uv"], blockers: &["uv", "uvx"], reason: DOWNLOADS },
    Cache { key: "cache:pip", title: "pip cache", paths: &[".cache/pip"], blockers: &["pip", "pip3"], reason: DOWNLOADS },
    Cache { key: "cache:yarn", title: "Yarn cache", paths: &[".cache/yarn"], blockers: &["yarn"], reason: DOWNLOADS },
    CARGO,
    Cache { key: "cache:go", title: "Go build cache", paths: &[".cache/go-build"], blockers: &["go"], reason: "Rebuilds on the next build" },
    GRADLE,
    NUGET,
    Cache { key: "cache:homebrew", title: "Homebrew downloads", paths: &[".cache/Homebrew"], blockers: &["brew"], reason: DOWNLOADS },
    Cache { key: "cache:electron", title: "Electron downloads", paths: &[".cache/electron"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "cache:node-gyp", title: "node-gyp headers", paths: &[".cache/node-gyp"], blockers: &[], reason: DOWNLOADS },
    Cache { key: "cache:typescript", title: "TypeScript type cache", paths: &[".cache/typescript"], blockers: &[], reason: DOWNLOADS },
];

/// npm runs as node; its script's name is what gives it away.
const NPM: &[&str] = &["npm", "npx", "npm-cli.js", "npx-cli.js"];
const CARGO: Cache = Cache { key: "cache:cargo", title: "Cargo downloads", paths: &[".cargo/registry/cache", ".cargo/registry/src"], blockers: &["cargo", "rustc"], reason: DOWNLOADS };
const GRADLE: Cache = Cache { key: "cache:gradle", title: "Gradle cache", paths: &[".gradle/caches"], blockers: &["java"], reason: DOWNLOADS };
const NUGET: Cache = Cache { key: "cache:nuget", title: "NuGet packages", paths: &[".nuget/packages"], blockers: &["dotnet", "msbuild", "devenv"], reason: DOWNLOADS };

fn cache_table() -> &'static [Cache] {
    if cfg!(windows) {
        CACHES_WINDOWS
    } else if cfg!(target_os = "linux") {
        CACHES_LINUX
    } else {
        CACHES_MAC
    }
}

fn caches(c: &Ctx) -> Vec<Item> {
    let mut out = Vec::new();
    for cache in cache_table() {
        let paths: Vec<PathBuf> = cache.paths.iter().map(|p| under(c.home, p)).filter(|p| real_dir(p)).collect();
        if paths.is_empty() {
            continue;
        }
        let mut item = Item::new(cache.key.into(), Group::Rebuilds, cache.title.into(), Removal::Delete);
        item.detail = Some(paths.iter().map(|p| c.tilde(p)).collect::<Vec<_>>().join(", "));
        item.reason = cache.reason.into();
        item.blockers = cache.blockers.to_vec();
        hold_while_running(c, &mut item);
        for p in paths {
            item = item.path(p, None);
        }
        out.push(item);
    }
    out
}

const UNKNOWN_USE: &str = "Couldn't check what's in use";

/// Something is using it right now: say who, and don't tick it.
fn mark_in_use(item: &mut Item, procs: &Procs) {
    if !procs.open_known {
        item.note = Some(UNKNOWN_USE.into());
        item.recommended = false;
        return;
    }
    let who = item.busy_root.iter().chain(&item.paths).find_map(|p| procs.using(p));
    if let Some(who) = who {
        item.note = Some(format!("In use by {who}"));
        item.recommended = false;
    }
}

/// Pre-ticked unless its tool is running right now.
fn hold_while_running(c: &Ctx, item: &mut Item) {
    match c.procs.running(&item.blockers) {
        Some(b) => {
            item.note = Some(format!("{b} is running"));
            item.recommended = false;
        }
        None => item.recommended = true,
    }
}

/// pnpm's shared store, through pnpm itself: only packages no project uses.
fn pnpm(c: &Ctx) -> Option<Item> {
    let stores: &[&str] = if cfg!(windows) {
        &["AppData/Local/pnpm/store", ".pnpm-store"]
    } else {
        &["Library/pnpm/store", ".local/share/pnpm/store", ".pnpm-store"]
    };
    let store = stores.iter().map(|r| under(c.home, r)).find(|p| real_dir(p))?;
    let bins: Vec<PathBuf> = if cfg!(windows) {
        vec![under(c.home, "AppData/Local/pnpm/pnpm.exe"), under(c.home, "AppData/Roaming/npm/pnpm.cmd")]
    } else {
        vec![
            under(c.home, "Library/pnpm/pnpm"),
            under(c.home, ".local/share/pnpm/pnpm"),
            PathBuf::from("/opt/homebrew/bin/pnpm"),
            PathBuf::from("/usr/local/bin/pnpm"),
            PathBuf::from("/usr/bin/pnpm"),
        ]
    };
    let bin = bins.into_iter().find(|p| p.is_file())?;
    let mut item = Item::new("cache:pnpm".into(), Group::Rebuilds, "pnpm store".into(), Removal::PnpmPrune(bin));
    item.detail = Some(c.tilde(&store));
    item.reason = "Keeps what your projects use".into();
    item.up_to = true;
    item.blockers = vec!["pnpm"];
    hold_while_running(c, &mut item);
    Some(item.path(store, None))
}

/// xcrun without a full Xcode pops up an "install tools" prompt, so only
/// call it when Xcode is the selected developer folder.
fn have_xcode() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let mut cmd = Command::new("/usr/bin/xcode-select");
    cmd.arg("-p");
    run_command(cmd, Duration::from_secs(5)).is_ok_and(|out| {
        let dir = out.trim();
        dir.contains(".app/Contents/Developer") && Path::new(dir).is_dir()
    })
}

/// "com.apple.CoreSimulator.SimRuntime.iOS-26-1" -> "iOS 26.1".
fn runtime_name(id: &str) -> String {
    let short = id.rsplit('.').next().unwrap_or(id);
    match short.split_once('-') {
        Some((os, ver)) => format!("{os} {}", ver.replace('-', ".")),
        None => short.to_string(),
    }
}

/// 8-4-4-4-12 hex digits, nothing else (never an option like "-x").
fn is_udid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12]) && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Simulators not opened in a long time, or whose iOS version is gone.
fn simulators(c: &Ctx) -> Vec<Item> {
    let devices_dir = c.home.join("Library/Developer/CoreSimulator/Devices");
    if !real_dir(&devices_dir) || !have_xcode() {
        return Vec::new();
    }
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.args(["simctl", "list", "devices", "--json"]);
    let Ok(out) = run_command(cmd, Duration::from_secs(30)) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&out) else { return Vec::new() };
    let Some(runtimes) = json.get("devices").and_then(|d| d.as_object()) else { return Vec::new() };
    let runtimes: BTreeMap<&String, &serde_json::Value> = runtimes.iter().collect();
    let mut items = Vec::new();
    for (runtime, list) in runtimes {
        for d in list.as_array().into_iter().flatten() {
            let text = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or_default();
            let udid = text("udid");
            let dir = devices_dir.join(udid);
            if !is_udid(udid) || text("state") == "Booted" || !real_dir(&dir) {
                continue;
            }
            let os = runtime_name(runtime);
            let available = d.get("isAvailable").and_then(|v| v.as_bool()).unwrap_or(true);
            let booted = parse_rfc3339_secs(text("lastBootedAt")).filter(|t| *t > 0).map(|t| t as u64);
            let (reason, recommended) = if !available {
                (format!("{os} is no longer installed"), true)
            } else {
                let (age, how) = match booted {
                    Some(t) => (c.age(t), "Last opened"),
                    None => (mtime(&dir).map(|t| c.age(t)).unwrap_or(0), "Never opened, made"),
                };
                if age < c.show_after {
                    continue;
                }
                (format!("{how} {} ago", span(age)), age >= c.cutoff)
            };
            let mut item = Item::new(format!("sim:{udid}"), Group::Unused, format!("Simulator: {}", text("name")), Removal::Simulator(udid.to_string()));
            item.detail = Some(os);
            item.reason = reason;
            item.note = Some("Deleted for good".into());
            item.blockers = vec!["Simulator"];
            item.recommended = recommended && c.procs.running(&item.blockers).is_none();
            items.push(item.path(dir.clone(), Some(dir.join("device.plist"))));
        }
    }
    items
}

/// "iPhone17,2 26.1 (23B85)" -> ("iPhone17,2", [26, 1]); "16.4 (20E247)" -> ("", [16, 4]).
fn parse_support(name: &str) -> Option<(String, Vec<u32>, String)> {
    let mut words = name.split_whitespace();
    let first = words.next()?;
    let (model, ver) = if first.contains(',') { (first.to_string(), words.next()?) } else { (String::new(), first) };
    let parts: Option<Vec<u32>> = ver.split('.').map(|p| p.parse().ok()).collect();
    Some((model, parts?, ver.to_string()))
}

/// Xcode keeps a copy of each phone's system files for every iOS version it
/// ever saw. Only the newest per device model is still needed; Xcode copies
/// an older one again if that phone ever connects.
fn device_support(c: &Ctx) -> Vec<Item> {
    let mut items = Vec::new();
    if !cfg!(target_os = "macos") {
        return items;
    }
    for os in ["iOS", "watchOS", "tvOS", "visionOS"] {
        let root = c.home.join(format!("Library/Developer/Xcode/{os} DeviceSupport"));
        let mut by_model: BTreeMap<String, Vec<(Vec<u32>, String, PathBuf)>> = BTreeMap::new();
        for dir in child_dirs(&root) {
            let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some((model, ver, label)) = parse_support(name) {
                by_model.entry(model).or_default().push((ver, label, dir));
            }
        }
        let mut old: Vec<(String, PathBuf)> = Vec::new();
        for (_, mut list) in by_model {
            list.sort();
            list.pop(); // the newest stays
            old.extend(list.into_iter().map(|(_, label, dir)| (label, dir)));
        }
        if old.is_empty() {
            continue;
        }
        old.sort();
        let title = if old.len() == 1 { format!("Old {os} device files") } else { format!("Old {os} device files ×{}", old.len()) };
        let mut item = Item::new(format!("devsupport:{os}"), Group::Unused, title, Removal::Delete);
        item.detail = Some(old.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", "));
        item.reason = "Newer version kept".into();
        item.blockers = vec!["Xcode"];
        let all_old = old.iter().all(|(_, d)| mtime(d).is_some_and(|t| c.age(t) >= c.cutoff));
        item.recommended = all_old && c.procs.running(&item.blockers).is_none();
        for (_, dir) in old {
            item = item.path(dir, None);
        }
        items.push(item);
    }
    items
}

/// Browsers downloaded for Playwright/Puppeteer tests, last installed long ago.
fn test_browsers(c: &Ctx) -> Vec<Item> {
    let mut items = Vec::new();
    let playwright = if cfg!(windows) {
        "AppData/Local/ms-playwright"
    } else if cfg!(target_os = "linux") {
        ".cache/ms-playwright"
    } else {
        "Library/Caches/ms-playwright"
    };
    for (key, title, rel) in [("browsers:playwright", "Playwright browsers", playwright), ("browsers:puppeteer", "Puppeteer browsers", ".cache/puppeteer")] {
        let dir = under(c.home, rel);
        if !real_dir(&dir) {
            continue;
        }
        let newest = child_dirs(&dir).iter().filter_map(|d| mtime(d)).max().or_else(|| mtime(&dir));
        let Some(age) = newest.map(|t| c.age(t)) else { continue };
        if age < c.show_after {
            continue;
        }
        let mut item = Item::new(key.into(), Group::Unused, title.into(), Removal::Trash);
        item.detail = Some(c.tilde(&dir));
        item.reason = format!("Last installed {} ago", span(age));
        item.recommended = age >= c.cutoff;
        items.push(item.path(dir, None));
    }
    items
}

// ---- Projects ----

/// Top-level folders in home that never hold code (macOS, Windows, Linux).
const SKIP_HOME: &[&str] = &[
    "Library", "Applications", "Music", "Movies", "Pictures", "Public", "AppData", "Videos", "Saved Games", "Contacts",
    "Favorites", "Links", "Searches", "3D Objects", "snap",
];
/// Any of these in a folder makes it a project.
const PROJECT_FILES: &[&str] =
    &[".git", "package.json", "Cargo.toml", "pyproject.toml", "requirements.txt", "go.mod", "Podfile", "build.gradle", "build.gradle.kts", "Gemfile"];
/// Folders a project's "last used" walk never looks inside.
const GENERATED: &[&str] = &["node_modules", "target", ".next", ".nuxt", ".svelte-kit", ".turbo", ".parcel-cache", ".angular", ".venv", "venv", "Pods", "build", "dist", ".gradle", "__pycache__"];

fn is_project(dir: &Path) -> bool {
    PROJECT_FILES.iter().any(|f| exists(&dir.join(f)))
}

/// Project folders up to three levels below home (~/code/app,
/// ~/Documents/GitHub/app), never inside another project.
fn project_roots(home: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, depth: usize, top: bool, budget: &mut usize, out: &mut Vec<PathBuf>) {
        for child in child_dirs(dir) {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Some(name) = child.file_name().and_then(|n| n.to_str()) else { continue };
            if name.starts_with('.') || GENERATED.contains(&name) || (top && SKIP_HOME.contains(&name)) {
                continue;
            }
            if is_project(&child) {
                out.push(child);
            } else if depth < 3 {
                visit(&child, depth + 1, false, budget, out);
            }
        }
    }
    let mut out = Vec::new();
    let mut budget = 20_000;
    visit(home, 1, true, &mut budget, &mut out);
    out
}

/// A generated folder, with the file that proves what it is.
fn build_marker(dir: &Path, name: &str) -> Option<PathBuf> {
    let parent = dir.parent()?;
    let first = |names: &[&str], base: &Path| names.iter().map(|n| base.join(n)).find(|p| real_file(p));
    match name {
        "node_modules" | ".next" | ".nuxt" | ".svelte-kit" | ".turbo" | ".parcel-cache" | ".angular" => first(&["package.json"], parent),
        // Cargo tags its target folder as a cache (bford.info/cachedir).
        "target" => first(&["CACHEDIR.TAG"], dir),
        ".venv" | "venv" => first(&["pyvenv.cfg"], dir),
        "Pods" => first(&["Podfile"], parent),
        "build" => first(&["build.gradle", "build.gradle.kts"], parent),
        ".gradle" => first(&["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"], parent),
        _ => None,
    }
}

/// Generated folders inside a project (up to four levels down), each with
/// its marker. Never looks inside one it found.
fn build_dirs(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    fn visit(dir: &Path, depth: usize, budget: &mut usize, out: &mut Vec<(PathBuf, PathBuf)>) {
        for child in child_dirs(dir) {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Some(name) = child.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some(marker) = build_marker(&child, name) {
                out.push((child, marker));
            } else if depth < 4 && !name.starts_with('.') && !GENERATED.contains(&name) {
                visit(&child, depth + 1, budget, out);
            }
        }
    }
    let mut out = Vec::new();
    let mut budget = 5_000;
    visit(root, 1, &mut budget, &mut out);
    out
}

/// When the project was last really worked on: its newest commit, checkout
/// or staged change, or the newest file anywhere outside generated folders
/// (editing a file doesn't change its folder's date, so every file is
/// looked at). None when that can't be told: too big to check fully, or
/// nothing readable. Such a project is never offered.
fn project_last_used(root: &Path) -> Option<u64> {
    let git = [".git/index", ".git/HEAD", ".git/logs/HEAD", ".git/FETCH_HEAD"].iter().filter_map(|g| mtime(&root.join(g))).max();
    fn walk(dir: &Path, depth: usize, budget: &mut usize, newest: &mut Option<u64>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let name = e.file_name();
            let name = name.to_string_lossy();
            let Ok(meta) = e.metadata() else { continue };
            if let Some(t) = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()) {
                *newest = Some(newest.map_or(t, |n| n.max(t)));
            }
            if meta.is_dir() && !name.starts_with('.') && !GENERATED.contains(&&*name) {
                if depth >= 12 {
                    *budget = 0; // deeper than we look: can't be sure
                    return;
                }
                walk(&e.path(), depth + 1, budget, newest);
            }
        }
    }
    let mut newest = None;
    let mut budget = 60_000;
    walk(root, 1, &mut budget, &mut newest);
    if budget == 0 {
        return None;
    }
    git.max(newest)
}

fn projects(c: &Ctx, report: &(dyn Fn(Progress) + Sync)) -> Vec<Item> {
    let mut items = Vec::new();
    report(Progress::at("Finding your projects", FIND_END));
    let roots = project_roots(c.home);
    let total = roots.len();
    for (i, root) in roots.into_iter().enumerate() {
        let fraction = FIND_END + (PROJECTS_END - FIND_END) * i as f32 / total.max(1) as f32;
        report(Progress { done: i + 1, total, ..Progress::at("Checking projects", fraction) });
        let Some(last_used) = project_last_used(&root) else { continue };
        let age = c.age(last_used);
        if age < c.show_after {
            continue;
        }
        let found = build_dirs(&root);
        if found.is_empty() {
            continue;
        }
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut item = Item::new(format!("project:{}", root.display()), Group::Unused, format!("{name} build files"), Removal::Trash);
        let rel: Vec<String> = found.iter().map(|(d, _)| d.strip_prefix(&root).unwrap_or(d).display().to_string()).collect();
        item.detail = Some(rel.join(", "));
        item.reason = format!("Project untouched {}", span(age));
        item.recommended = age >= c.cutoff;
        item.busy_root = Some(root.clone());
        item.last_used = Some(last_used);
        for (dir, marker) in found {
            item = item.path(dir, Some(marker));
        }
        items.push(item);
    }
    items
}

// ---- Yours to decide ----

/// Old Xcode archives: needed to read crash reports, so never pre-ticked.
fn archives(c: &Ctx) -> Option<Item> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let root = c.home.join("Library/Developer/Xcode/Archives");
    let mut old: Vec<(PathBuf, u64)> = Vec::new();
    for day in child_dirs(&root) {
        for a in child_dirs(&day) {
            if a.extension().is_some_and(|e| e == "xcarchive")
                && let Some(age) = mtime(&a).map(|t| c.age(t))
                && age >= c.show_after
            {
                old.push((a, age));
            }
        }
    }
    let youngest = old.iter().map(|(_, a)| *a).min()?;
    let mut names: Vec<String> = old
        .iter()
        .filter_map(|(a, _)| a.file_stem().map(|s| s.to_string_lossy().split(" 20").next().unwrap_or_default().to_string()))
        .collect();
    names.sort();
    names.dedup();
    let title = if old.len() == 1 { "Old Xcode archive".to_string() } else { format!("Old Xcode archives ×{}", old.len()) };
    let mut item = Item::new("xcode:archives".into(), Group::Yours, title, Removal::Trash);
    item.detail = Some(names.join(", "));
    item.reason = format!("Made {} ago or earlier", span(youngest));
    item.note = Some("Needed to read their crash reports".into());
    for (a, _) in old {
        let info = a.join("Info.plist");
        item = item.path(a, Some(info));
    }
    Some(item)
}

/// Downloaded Hugging Face models and datasets.
fn models(c: &Ctx) -> Vec<Item> {
    let hub = c.home.join(".cache/huggingface/hub");
    let mut items = Vec::new();
    for dir in child_dirs(&hub) {
        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { continue };
        let (what, rest) = if let Some(r) = name.strip_prefix("models--") {
            ("Model", r)
        } else if let Some(r) = name.strip_prefix("datasets--") {
            ("Dataset", r)
        } else {
            continue;
        };
        let age = mtime(&dir.join("snapshots")).or_else(|| mtime(&dir)).map(|t| c.age(t)).unwrap_or(0);
        let mut item = Item::new(format!("hf:{name}"), Group::Yours, format!("{what}: {}", rest.replace("--", "/")), Removal::Trash);
        item.detail = Some("Hugging Face".into());
        item.reason = format!("Downloaded {} ago", span(age));
        item.note = Some("Slow to download again".into());
        items.push(item.path(dir, None));
    }
    items
}

/// Android emulators (their disks are big; their setup is yours).
fn emulators(c: &Ctx) -> Vec<Item> {
    let root = c.home.join(".android/avd");
    let mut items = Vec::new();
    for dir in child_dirs(&root) {
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".avd")) else { continue };
        let age = mtime(&dir).map(|t| c.age(t)).unwrap_or(0);
        let mut item = Item::new(format!("avd:{name}"), Group::Yours, format!("Android emulator: {}", name.replace('_', " ")), Removal::Trash);
        item.reason = format!("Last used {} ago", span(age));
        item.note = Some("Its setup goes with it".into());
        item.blockers = vec!["qemu-system-aarch64", "qemu-system-x86_64", "emulator"];
        let ini = root.join(format!("{name}.ini"));
        let config = dir.join("config.ini");
        item = item.path(dir, Some(config));
        if real_file(&ini) {
            item = item.path(ini, None);
        }
        items.push(item);
    }
    items
}

// ---- Cleaning ----

#[derive(Debug, PartialEq)]
pub enum CleanError {
    /// Something that owns it is busy; nothing was touched.
    Blocked(String),
    Failed(String),
}

#[derive(Debug, PartialEq)]
pub struct Cleaned {
    pub bytes: u64,
    pub trashed: bool,
    /// Paths that couldn't be removed (the rest were).
    pub failed: usize,
}

/// Removes one item, after checking again that it is what the scan saw and
/// that nothing is using it.
pub fn clean(item: &Item, home: &Path, procs: &Procs) -> Result<Cleaned, CleanError> {
    if !procs.open_known {
        return Err(CleanError::Blocked(format!("{UNKNOWN_USE}. Try again")));
    }
    if let Some(b) = procs.running(&item.blockers) {
        return Err(CleanError::Blocked(format!("{b} is running. Quit it first")));
    }
    if let Some(root) = &item.busy_root
        && let Some(who) = procs.using(root)
    {
        return Err(CleanError::Blocked(format!("{who} is using this project. Quit it first")));
    }
    if let (Some(root), Some(then)) = (&item.busy_root, item.last_used)
        && project_last_used(root).is_none_or(|now| now > then)
    {
        return Err(CleanError::Blocked("Worked on since the scan. Look again".into()));
    }
    let mut live = Vec::new();
    for (i, p) in item.paths.iter().enumerate() {
        if std::fs::symlink_metadata(p).is_err() {
            continue; // already gone
        }
        safe_target(home, p).map_err(CleanError::Blocked)?;
        if let Some(Some(m)) = item.markers.get(i)
            && !real_file(m)
        {
            return Err(CleanError::Blocked("Changed since the scan. Rescan".into()));
        }
        if let Some(who) = procs.using(p) {
            return Err(CleanError::Blocked(format!("{who} is using it. Quit it first")));
        }
        live.push((p, item.sizes.get(i).copied().unwrap_or(0), item.ids.get(i).copied().flatten()));
    }
    if live.is_empty() {
        return Ok(Cleaned { bytes: 0, trashed: false, failed: 0 });
    }
    let mut done = Cleaned { bytes: 0, trashed: item.removal == Removal::Trash, failed: 0 };
    let mut first_error = None;
    let mut removed = 0usize;
    for (p, size, expect) in live {
        let r = match &item.removal {
            Removal::Delete => remove(home, p, expect),
            Removal::Trash => trash(home, p, expect),
            Removal::Simulator(udid) => delete_simulator(udid),
            Removal::PnpmPrune(bin) => pnpm_prune(bin, p).map(|freed| {
                done.bytes += freed;
            }),
        };
        match r {
            Ok(()) => {
                removed += 1;
                if !matches!(item.removal, Removal::PnpmPrune(_)) {
                    done.bytes += size;
                }
            }
            Err(e) => {
                done.failed += 1;
                first_error.get_or_insert(e);
            }
        }
    }
    let tried = done.failed + removed;
    match first_error {
        // Nothing went: say why. Some went: report what did.
        Some(e) if removed == 0 && tried > 0 => Err(CleanError::Failed(e)),
        _ => Ok(done),
    }
}

/// Only a real folder or file inside home (not home's own top level), with
/// no symlink anywhere on the way.
fn safe_target(home: &Path, p: &Path) -> Result<(), String> {
    let refuse = || Err("Not a plain folder in your home. Left alone".to_string());
    if !p.is_absolute() || !inside(p, home) || p.components().count() < home.components().count() + 2 {
        return refuse();
    }
    match std::fs::symlink_metadata(p) {
        Ok(m) if !m.file_type().is_symlink() => {}
        _ => return refuse(),
    }
    match canonical(p) {
        Some(real) if same_path(&real, p) => Ok(()),
        _ => refuse(),
    }
}

/// Two spellings of one path (Windows ignores letter case).
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase() } else { a == b }
}

const NOT_PLAIN: &str = "Not a plain folder in your home. Left alone";
/// Unix tells a stand-in folder apart by device and inode.
#[cfg(unix)]
const CHANGED: &str = "Changed since the scan. Look again";

/// Deletes it through open folder handles (see `pinned`): a folder swapped
/// for a symlink mid-way can't redirect it anywhere else.
#[cfg(unix)]
fn remove(home: &Path, p: &Path, expect: Option<(u64, u64)>) -> Result<(), String> {
    let (parent, name, id) = pinned::locate(home, p, expect)?;
    pinned::remove_at(&parent, &name, id.dev, 0).map_err(|e| format!("Couldn't remove: {e}"))
}

/// Windows has no handle-relative delete in the standard library, so: no
/// link or junction anywhere on the way, then the whole folder is set aside
/// in one rename (which fails if anything in it is open or running), then
/// deleted. If deleting stops part-way, what's left goes back under its
/// own name, so it shows up again.
#[cfg(windows)]
fn remove(home: &Path, p: &Path, _expect: Option<(u64, u64)>) -> Result<(), String> {
    no_links_on_the_way(home, p)?;
    let aside = set_aside(p)?;
    let meta = std::fs::symlink_metadata(&aside).map_err(|e| e.to_string())?;
    let r = if meta.is_dir() { std::fs::remove_dir_all(&aside) } else { std::fs::remove_file(&aside) };
    r.map_err(|e| {
        let _ = std::fs::rename(&aside, p);
        format!("Only partly removed: {e}")
    })
}

#[cfg(not(any(unix, windows)))]
fn remove(_home: &Path, _p: &Path, _expect: Option<(u64, u64)>) -> Result<(), String> {
    Err("Not supported here yet".into())
}

#[cfg(windows)]
const IN_USE: &str = "Something has files open in it. Quit it first";

/// Every step from home to `p` is a real folder, never a link or junction.
#[cfg(windows)]
fn no_links_on_the_way(home: &Path, p: &Path) -> Result<(), String> {
    let rel = p.strip_prefix(home).map_err(|_| NOT_PLAIN)?;
    let mut at = home.to_path_buf();
    for part in rel.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            // is_symlink covers junctions and other name-surrogate reparse points.
            Ok(m) if !m.file_type().is_symlink() => {}
            _ => return Err(NOT_PLAIN.into()),
        }
    }
    Ok(())
}

/// Renames `p` beside itself. Windows refuses to rename a folder while any
/// file inside is open without delete sharing or is a running program, so
/// this moves all of it or none of it.
#[cfg(windows)]
fn set_aside(p: &Path) -> Result<PathBuf, String> {
    let name = p.file_name().ok_or(NOT_PLAIN)?.to_string_lossy().into_owned();
    for n in 0..100 {
        let aside = p.with_file_name(format!("{name}.butters-{}-{n}", std::process::id()));
        if exists(&aside) {
            continue;
        }
        return match std::fs::rename(p, &aside) {
            Ok(()) => Ok(aside),
            // Access denied, sharing or lock violation: something holds it.
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32 | 33)) => Err(IN_USE.into()),
            Err(e) => Err(format!("Couldn't remove: {e}")),
        };
    }
    Err("Couldn't remove: no free name to set it aside".into())
}

/// Moves it to the Trash, then checks the Trash got exactly the folder that
/// was checked a moment before. The Trash API only takes a path; if
/// something else was put in its place in between, that's reported (it's in
/// the Trash, so it can be put back).
#[cfg(target_os = "macos")]
fn trash(home: &Path, p: &Path, expect: Option<(u64, u64)>) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let (_parent, _name, id) = pinned::locate(home, p, expect)?;
    let path = p.to_str().ok_or("Unusual file name. Left alone")?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let mut landed: Option<objc2::rc::Retained<NSURL>> = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut landed))
        .map_err(|e| format!("Couldn't move to the Trash: {}", e.localizedDescription()))?;
    let landed = landed.and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string()));
    match landed.as_deref().and_then(identity) {
        Some(got) if got != (id.dev, id.ino) => Err("Something else went to the Trash in its place. Check the Trash".into()),
        _ => Ok(()),
    }
}

/// The Recycle Bin. Anything too big for it gets Windows' own "delete
/// permanently?" question rather than being deleted silently. First the
/// same all-or-nothing in-use check as deleting: set aside and put back.
#[cfg(windows)]
fn trash(home: &Path, p: &Path, _expect: Option<(u64, u64)>) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    no_links_on_the_way(home, p)?;
    let aside = set_aside(p)?;
    std::fs::rename(&aside, p).map_err(|e| format!("Couldn't put it back after checking: {e}"))?;
    use windows::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW, SHFileOperationW,
    };
    use windows::core::PCWSTR;
    // A list of paths, each ending in a NUL, the whole list in another.
    let mut from: Vec<u16> = p.as_os_str().encode_wide().collect();
    if from.contains(&0) {
        return Err("Unusual file name. Left alone".into());
    }
    from.extend([0, 0]);
    let flags = FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_SILENT.0 | FOF_WANTNUKEWARNING.0;
    let mut op = SHFILEOPSTRUCTW { wFunc: FO_DELETE, pFrom: PCWSTR(from.as_ptr()), fFlags: flags as u16, ..Default::default() };
    // SAFETY: `op` points at `from`, which lives until the call returns.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        return Err(format!("Couldn't move to the Recycle Bin (error {code})"));
    }
    if op.fAnyOperationsAborted.as_bool() || exists(p) {
        return Err("Left in place: not moved to the Recycle Bin".into());
    }
    Ok(())
}

/// The desktop Trash (freedesktop.org spec), in ~/.local/share/Trash.
#[cfg(target_os = "linux")]
fn trash(home: &Path, p: &Path, expect: Option<(u64, u64)>) -> Result<(), String> {
    let (parent, name, _id) = pinned::locate(home, p, expect)?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"));
    trash_freedesktop(&parent, &name, p, &data.join("Trash"), &local_timestamp())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn trash(_home: &Path, _p: &Path, _expect: Option<(u64, u64)>) -> Result<(), String> {
    Err("The Trash isn't supported here yet".into())
}

/// Moves `name` (inside the open folder `parent`; `original` is its path)
/// into a freedesktop.org Trash folder (`files/` + `info/`), so the file
/// manager can put it back. Only within one disk: a move across disks would
/// be a copy, so that is refused instead.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn trash_freedesktop(parent: &std::os::fd::OwnedFd, leaf: &std::ffi::CStr, original: &Path, trash_dir: &Path, deleted_at: &str) -> Result<(), String> {
    let p = original;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    let (files, info) = (trash_dir.join("files"), trash_dir.join("info"));
    for d in [&files, &info] {
        std::fs::create_dir_all(d).map_err(|e| format!("Couldn't open the Trash: {e}"))?;
    }
    let name = p.file_name().ok_or("Not a folder name")?.to_string_lossy().into_owned();
    // Keep the original path as bytes, percent-encoded as the spec asks.
    let mut encoded = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    for n in 1..1000 {
        let entry = if n == 1 { name.clone() } else { format!("{name}.{n}") };
        let info_path = info.join(format!("{entry}.trashinfo"));
        let target = files.join(&entry);
        if exists(&target) {
            continue;
        }
        // Reserving the info file first makes the name ours.
        let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&info_path) else { continue };
        let written = write!(f, "[Trash Info]\nPath={encoded}\nDeletionDate={deleted_at}\n");
        let moved = written.map_err(|e| e.to_string()).and_then(|_| pinned::rename_out(parent, leaf, &target).map_err(|e| e.to_string()));
        return moved.map_err(|e| {
            let _ = std::fs::remove_file(&info_path);
            format!("Couldn't move to the Trash: {e}")
        });
    }
    Err("The Trash is full of the same name".into())
}

/// Local time as the Trash spec wants it: 2026-10-24T12:30:05.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn local_timestamp() -> String {
    let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r only writes into the zeroed struct we pass it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return String::new();
    }
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// File operations that a folder swapped for a symlink can't redirect:
/// every step goes through an open handle to the folder above (openat,
/// unlinkat, renameat with O_NOFOLLOW), never a path looked up again.
#[cfg(unix)]
mod pinned {
    use std::ffi::{CStr, CString, OsStr};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    const DIR: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    /// Deeper than any real node_modules; stops a crafted tree from using up
    /// file handles.
    const MAX_DEPTH: usize = 256;

    pub struct Ident {
        pub dev: u64,
        pub ino: u64,
        pub dir: bool,
    }

    fn c(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    }

    fn check(rc: libc::c_int) -> io::Result<libc::c_int> {
        if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(rc) }
    }

    fn open_dir_at(dir: Option<&OwnedFd>, name: &CStr) -> io::Result<OwnedFd> {
        // SAFETY: valid NUL-terminated name and an open fd (or AT_FDCWD).
        let fd = check(unsafe { libc::openat(dir.map_or(libc::AT_FDCWD, |d| d.as_raw_fd()), name.as_ptr(), DIR) })?;
        // SAFETY: openat just returned this fd and nothing else owns it.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// lstat relative to an open folder.
    fn stat_at(dir: &OwnedFd, name: &CStr) -> io::Result<(Ident, bool)> {
        // SAFETY: zeroed is a valid bit pattern for this plain C struct.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: valid fd, name and out-pointer.
        check(unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) })?;
        let kind = st.st_mode & libc::S_IFMT;
        Ok((Ident { dev: st.st_dev as u64, ino: st.st_ino as u64, dir: kind == libc::S_IFDIR }, kind == libc::S_IFLNK))
    }

    /// Opens the folder holding `target` one step at a time from `home`,
    /// never following a symlink, and checks `target` is the very thing the
    /// scan saw (same device and inode). Returns that folder, the name, and
    /// what it is.
    pub fn locate(home: &Path, target: &Path, expect: Option<(u64, u64)>) -> Result<(OwnedFd, CString, Ident), String> {
        let not_plain = || super::NOT_PLAIN.to_string();
        let rel = target.strip_prefix(home).map_err(|_| not_plain())?;
        let mut parts = Vec::new();
        for part in rel.components() {
            match part {
                Component::Normal(n) => parts.push(c(n).map_err(|_| not_plain())?),
                _ => return Err(not_plain()),
            }
        }
        let name = parts.pop().ok_or_else(not_plain)?;
        // Home itself is the resolved real path; its parents belong to the system.
        let mut dir = open_dir_at(None, &c(home.as_os_str()).map_err(|_| not_plain())?).map_err(|_| not_plain())?;
        for part in &parts {
            dir = open_dir_at(Some(&dir), part).map_err(|_| not_plain())?;
        }
        let (id, link) = stat_at(&dir, &name).map_err(|_| not_plain())?;
        if link {
            return Err(not_plain());
        }
        if expect.is_some_and(|e| e != (id.dev, id.ino)) {
            return Err(super::CHANGED.to_string());
        }
        Ok((dir, name, id))
    }

    /// Deletes `name` inside `dir`, folders and all. Each level is opened
    /// from the one above without following symlinks; symlinks inside are
    /// removed as links, never followed; other disks are never entered.
    pub fn remove_at(dir: &OwnedFd, name: &CStr, dev: u64, depth: usize) -> io::Result<()> {
        let (id, link) = stat_at(dir, name)?;
        if link || !id.dir {
            // SAFETY: valid fd and name.
            check(unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) })?;
            return Ok(());
        }
        if id.dev != dev || depth > MAX_DEPTH {
            return Err(io::Error::other("another disk, or nested too deep"));
        }
        let sub = open_dir_at(Some(dir), name)?;
        for child in list(&sub)? {
            remove_at(&sub, &child, dev, depth + 1)?;
        }
        drop(sub);
        // SAFETY: valid fd and name.
        check(unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) })?;
        Ok(())
    }

    /// Every name in an open folder (all read before anything is removed).
    fn list(dir: &OwnedFd) -> io::Result<Vec<CString>> {
        // fdopendir takes ownership of the fd it's given, so give it a copy.
        // SAFETY: valid fd.
        let copy = check(unsafe { libc::dup(dir.as_raw_fd()) })?;
        // SAFETY: `copy` is a fresh fd we own; on success DIR owns it.
        let d = unsafe { libc::fdopendir(copy) };
        if d.is_null() {
            let e = io::Error::last_os_error();
            // SAFETY: fdopendir failed, so `copy` is still ours to close.
            unsafe { libc::close(copy) };
            return Err(e);
        }
        let mut names = Vec::new();
        loop {
            // SAFETY: `d` is a valid open DIR; the entry stays valid until the next call.
            let entry = unsafe { libc::readdir(d) };
            if entry.is_null() {
                break; // end, or an error: then the final rmdir fails and reports it
            }
            // SAFETY: d_name is a NUL-terminated name inside the entry.
            let n = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if n.to_bytes() != b"." && n.to_bytes() != b".." {
                names.push(n.to_owned());
            }
        }
        // SAFETY: closes the DIR and the fd it owns, once.
        unsafe { libc::closedir(d) };
        Ok(names)
    }

    /// Renames `name` inside `dir` to an absolute path elsewhere.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn rename_out(dir: &OwnedFd, name: &CStr, to: &Path) -> io::Result<()> {
        let to = c(to.as_os_str())?;
        // SAFETY: valid fd and NUL-terminated names.
        check(unsafe { libc::renameat(dir.as_raw_fd(), name.as_ptr(), libc::AT_FDCWD, to.as_ptr()) })?;
        Ok(())
    }
}

fn delete_simulator(udid: &str) -> Result<(), String> {
    if !is_udid(udid) {
        return Err("Not a simulator id".into());
    }
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.args(["simctl", "delete", udid]);
    run_command(cmd, Duration::from_secs(120)).map(|_| ()).map_err(|e| format!("Couldn't delete: {e}"))
}

/// Runs `pnpm store prune`; returns how much the store shrank.
fn pnpm_prune(bin: &Path, store: &Path) -> Result<u64, String> {
    let before = size_of(store);
    let mut cmd = Command::new(bin);
    cmd.args(["store", "prune"]);
    // macOS gives GUI apps a bare PATH; Homebrew's pnpm needs to find node.
    if cfg!(unix) {
        let dir = bin.parent().map(|d| d.display().to_string()).unwrap_or_default();
        cmd.env("PATH", format!("{dir}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"));
    }
    run_command(cmd, Duration::from_secs(600)).map_err(|e| format!("pnpm couldn't prune: {e}"))?;
    Ok(before.saturating_sub(size_of(store)))
}

// ---- Measuring and small helpers ----

/// Sizes every path, a few at a time, reporting as each one finishes.
fn measure(items: &mut [Item], report: &(dyn Fn(Progress) + Sync)) {
    let jobs: Vec<(usize, usize, PathBuf)> =
        items.iter().enumerate().flat_map(|(i, it)| it.paths.iter().enumerate().map(move |(j, p)| (i, j, p.clone()))).collect();
    let total = jobs.len();
    let next = AtomicUsize::new(0);
    // (finished, bytes so far), together so each report is consistent.
    let tally = Mutex::new((0usize, 0u64));
    let sizes = Mutex::new(Vec::with_capacity(total));
    let progress = |done: usize, found: u64| {
        let fraction = PROJECTS_END + (1.0 - PROJECTS_END) * done as f32 / total.max(1) as f32;
        report(Progress { done, total, found, ..Progress::at("Measuring", fraction) });
    };
    progress(0, 0);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                while let Some((i, j, p)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let n = size_of(p);
                    sizes.lock().unwrap_or_else(|e| e.into_inner()).push((*i, *j, n));
                    let mut t = tally.lock().unwrap_or_else(|e| e.into_inner());
                    t.0 += 1;
                    t.1 += n;
                    progress(t.0, t.1);
                }
            });
        }
    });
    for it in items.iter_mut() {
        it.sizes = vec![0; it.paths.len()];
    }
    for (i, j, n) in sizes.into_inner().unwrap_or_else(|e| e.into_inner()) {
        items[i].sizes[j] = n;
    }
    for it in items.iter_mut() {
        it.bytes = it.sizes.iter().sum();
    }
}

/// Disk space deleting `root` would really give back.
///
/// - macOS: each file's "private size", the bytes not shared with a clone
///   or snapshot. pnpm, bun and uv clone files between their caches and
///   your projects, so a folder can look far bigger than what deleting it
///   frees. Read a whole folder per system call, so it stays quick.
/// - Linux: allocated blocks.
/// - A hard-linked file only counts when every link to it is inside
///   `root`; otherwise deleting `root` frees nothing of it.
///
/// Never follows symlinks or crosses into another disk. Data shared only
/// between clones that are both inside `root` isn't counted, so the total
/// errs low, never high.
#[cfg(unix)]
pub fn size_of(root: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let Ok(top) = std::fs::symlink_metadata(root) else { return 0 };
    let mut tally = Tally::default();
    if top.is_file() {
        tally.file(top.dev(), top.ino(), top.nlink(), top.blocks() * 512);
        return tally.total();
    }
    if !top.is_dir() {
        return 0;
    }
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        #[cfg(target_os = "macos")]
        if bulk_dir(&dir, top.dev(), &mut tally, &mut dirs) {
            continue;
        }
        // Swapped for a symlink since it was listed: not followed.
        if real_dir(&dir) {
            plain_dir(&dir, top.dev(), &mut tally, &mut dirs);
        }
    }
    tally.total()
}

/// Bytes found so far, with hard links held back until every link is seen.
#[derive(Default)]
struct Tally {
    bytes: u64,
    /// (device, inode) -> (links seen, links in total, bytes)
    links: HashMap<(u64, u64), (u64, u64, u64)>,
}

impl Tally {
    fn file(&mut self, dev: u64, ino: u64, nlink: u64, bytes: u64) {
        if nlink <= 1 {
            self.bytes += bytes;
        } else {
            self.links.entry((dev, ino)).or_insert((0, nlink, bytes)).0 += 1;
        }
    }

    fn total(&self) -> u64 {
        self.bytes + self.links.values().filter(|(seen, all, _)| seen >= all).map(|(_, _, b)| b).sum::<u64>()
    }
}

/// One folder through the standard library (lstat per entry).
#[cfg(unix)]
fn plain_dir(dir: &Path, dev: u64, tally: &mut Tally, dirs: &mut Vec<PathBuf>) {
    use std::os::unix::fs::MetadataExt;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        // DirEntry::metadata doesn't follow symlinks on Unix.
        let Ok(m) = e.metadata() else { continue };
        if m.dev() != dev {
            continue;
        }
        if m.is_dir() {
            dirs.push(e.path());
        } else if m.is_file() {
            tally.file(m.dev(), m.ino(), m.nlink(), m.blocks() * 512);
        }
    }
}

/// One folder through getattrlistbulk: name, type, device, file id, link
/// count, allocated and private size for every entry, in a few calls.
/// False if the folder can't be read this way (then `plain_dir` is used).
#[cfg(target_os = "macos")]
fn bulk_dir(dir: &Path, dev: u64, tally: &mut Tally, dirs: &mut Vec<PathBuf>) -> bool {
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;

    struct Fd(libc::c_int);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: we own this descriptor and close it exactly once.
            unsafe { libc::close(self.0) };
        }
    }

    let Ok(c_dir) = CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid NUL-terminated path; the result is checked.
    let fd = unsafe { libc::open(c_dir.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 {
        return false;
    }
    let fd = Fd(fd);
    let mut list = libc::attrlist {
        bitmapcount: 5, // ATTR_BIT_MAP_COUNT
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_NAME | libc::ATTR_CMN_DEVID | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE,
        forkattr: libc::ATTR_CMNEXT_PRIVATESIZE,
    };
    let options = (libc::FSOPT_NOFOLLOW | libc::FSOPT_ATTR_CMN_EXTENDED) as u64;
    let mut buf = vec![0u8; 256 * 1024];
    let mut first = true;
    loop {
        // SAFETY: `list` and `buf` are valid for the call; the kernel writes
        // at most `buf.len()` bytes and returns how many entries it packed.
        let n = unsafe { libc::getattrlistbulk(fd.0, (&raw mut list).cast(), buf.as_mut_ptr().cast(), buf.len(), options) };
        if n <= 0 {
            return n == 0 || !first;
        }
        first = false;
        let mut at = 0usize;
        for _ in 0..n {
            let Some(len) = read_u32(&buf, at).map(|l| l as usize) else { break };
            let Some(entry) = buf.get(at..at + len).filter(|_| len >= 4) else { break };
            if let Some(e) = parse_bulk_entry(entry) {
                const VREG: u32 = 1;
                const VDIR: u32 = 2;
                let same_disk = e.dev.is_none_or(|d| d == dev as i32);
                match e.kind {
                    VDIR if same_disk && !e.name.is_empty() => dirs.push(dir.join(OsStr::from_bytes(e.name))),
                    VREG if same_disk => {
                        // Never more than it takes up, whatever the private size says.
                        let bytes = e.private.map_or(e.allocated, |p| p.min(e.allocated));
                        tally.file(dev, e.file_id, e.links, bytes);
                    }
                    _ => {}
                }
            }
            at += len;
        }
    }
}

#[cfg(target_os = "macos")]
struct BulkEntry<'a> {
    name: &'a [u8],
    dev: Option<i32>,
    kind: u32,
    file_id: u64,
    links: u64,
    allocated: u64,
    private: Option<u64>,
}

/// One getattrlistbulk record. Attributes are packed in a fixed order, each
/// present only if its bit is set in the returned set, 4-byte aligned.
#[cfg(target_os = "macos")]
fn parse_bulk_entry(e: &[u8]) -> Option<BulkEntry<'_>> {
    let common = read_u32(e, 4)?;
    let file = read_u32(e, 16)?;
    let ext = read_u32(e, 20)?;
    let mut at = 24;
    let mut out = BulkEntry { name: &[], dev: None, kind: 0, file_id: 0, links: 1, allocated: 0, private: None };
    if common & libc::ATTR_CMN_NAME != 0 {
        // attrreference_t: offset from itself, length including the NUL.
        let off = i32::from_ne_bytes(e.get(at..at + 4)?.try_into().ok()?);
        let len = read_u32(e, at + 4)? as usize;
        let start = usize::try_from(at as i64 + off as i64).ok()?;
        out.name = e.get(start..start + len.saturating_sub(1))?;
        at += 8;
    }
    if common & libc::ATTR_CMN_DEVID != 0 {
        out.dev = Some(i32::from_ne_bytes(e.get(at..at + 4)?.try_into().ok()?));
        at += 4;
    }
    if common & libc::ATTR_CMN_OBJTYPE != 0 {
        out.kind = read_u32(e, at)?;
        at += 4;
    }
    if common & libc::ATTR_CMN_FILEID != 0 {
        out.file_id = read_u64(e, at)?;
        at += 8;
    }
    if file & libc::ATTR_FILE_LINKCOUNT != 0 {
        out.links = read_u32(e, at)? as u64;
        at += 4;
    }
    if file & libc::ATTR_FILE_ALLOCSIZE != 0 {
        out.allocated = read_u64(e, at)?;
        at += 8;
    }
    if ext & libc::ATTR_CMNEXT_PRIVATESIZE != 0 {
        out.private = Some(read_u64(e, at)?);
    }
    Some(out)
}

#[cfg(target_os = "macos")]
fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(target_os = "macos")]
fn read_u64(b: &[u8], at: usize) -> Option<u64> {
    // off_t values are signed; anything negative counts as nothing.
    Some(i64::from_ne_bytes(b.get(at..at + 8)?.try_into().ok()?).max(0) as u64)
}

/// Windows: space each file really takes on disk, hard links counted only
/// when every link is inside (pnpm hard-links its store into projects on
/// Windows), never following links or junctions, never leaving the volume.
#[cfg(windows)]
pub fn size_of(root: &Path) -> u64 {
    let Some(top) = win_file_info(root) else { return 0 };
    let mut tally = Tally::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() {
            continue; // also junctions
        }
        if meta.is_dir() {
            stack.extend(std::fs::read_dir(&path).into_iter().flatten().flatten().map(|e| e.path()));
        } else if let Some(f) = win_file_info(&path)
            && f.volume == top.volume
        {
            tally.file(f.volume, f.index, f.links, f.allocated);
        }
    }
    tally.total()
}

#[cfg(windows)]
struct WinFile {
    volume: u64,
    index: u64,
    links: u64,
    allocated: u64,
}

/// Volume, file id, link count and allocated size, read through a handle
/// opened for attributes only (it never blocks anyone else).
#[cfg(windows)]
fn win_file_info(p: &Path) -> Option<WinFile> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_STANDARD_INFO, FileStandardInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    };
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    // Open the link itself, not its target; folders need backup semantics.
    const FLAGS: u32 = 0x0020_0000 | 0x0200_0000;
    let f = std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES).share_mode(SHARE_ALL).custom_flags(FLAGS).open(p).ok()?;
    let h = HANDLE(f.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `h` is an open handle kept alive by `f`; `info` is a valid out-pointer.
    unsafe { GetFileInformationByHandle(h, &mut info) }.ok()?;
    let mut size = FILE_STANDARD_INFO::default();
    // SAFETY: as above; the buffer is exactly one FILE_STANDARD_INFO.
    unsafe { GetFileInformationByHandleEx(h, FileStandardInfo, (&raw mut size).cast(), std::mem::size_of::<FILE_STANDARD_INFO>() as u32) }.ok()?;
    Some(WinFile {
        volume: info.dwVolumeSerialNumber as u64,
        index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        links: info.nNumberOfLinks as u64,
        allocated: size.AllocationSize.max(0) as u64,
    })
}

#[cfg(not(any(unix, windows)))]
pub fn size_of(_root: &Path) -> u64 {
    0
}

/// (device, inode) without following a symlink.
fn identity(p: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(p).ok().map(|m| (m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        None
    }
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

fn real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

fn real_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

fn mtime(p: &Path) -> Option<u64> {
    std::fs::symlink_metadata(p).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Real (non-symlink) subfolders, sorted so results are stable.
fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// "3 days", "2 weeks", "4 months", "1 year".
pub fn span(secs: u64) -> String {
    let d = (secs / DAY).max(1);
    let (n, unit) = if d < 14 {
        (d, "day")
    } else if d < 60 {
        (d / 7, "week")
    } else if d < 730 {
        (d / 30, "month")
    } else {
        (d / 365, "year")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

// Real files with real timestamps and Unix symlinks: macOS and Linux.
// Tests of macOS-only parts (Xcode, APFS clones, lsof) say so.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    /// File times are real, so "now" is too.
    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    /// A fresh fake home folder (canonical, like the app's).
    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("butters-deep-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(dir).unwrap()
    }

    fn file(p: &Path, days_old: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
        age(p, days_old);
    }

    fn age(p: &Path, days_old: u64) {
        let t = UNIX_EPOCH + Duration::from_secs(now() - days_old * DAY);
        fs::File::open(p).unwrap().set_modified(t).unwrap();
    }

    /// Ages every folder under `root` too (making files touches them).
    fn age_dirs(root: &Path, days_old: u64) {
        for d in child_dirs(root) {
            age_dirs(&d, days_old);
        }
        age(root, days_old);
    }

    fn scan_all(h: &Path, procs: &Procs) -> Vec<Item> {
        scan(h, now(), 90, 0, procs, &|_| {})
    }

    #[test]
    fn dormant_project_build_folders_are_offered_with_proof() {
        let h = home("dormant");
        let p = h.join("code/shop");
        file(&p.join("package.json"), 200);
        file(&p.join("src/index.ts"), 200);
        file(&p.join("node_modules/left-pad/index.js"), 200);
        file(&p.join("src-tauri/target/CACHEDIR.TAG"), 200);
        file(&p.join("src-tauri/target/debug/app"), 200);
        // Looks like build output but nothing proves it: never offered.
        file(&p.join("tools/target/notes.txt"), 200);
        file(&p.join("api/.venv/lib/x.py"), 200);
        age_dirs(&h, 200);
        let seen = Mutex::new(Vec::new());
        let items = scan(&h, now(), 90, 0, &Procs::fake(&[]), &|p| seen.lock().unwrap().push(p));
        // The bar only moves forward and ends full, after measuring every folder.
        let seen = seen.into_inner().unwrap();
        assert!(seen.windows(2).all(|w| w[0].fraction <= w[1].fraction), "{seen:?}");
        let last = seen.last().unwrap();
        assert_eq!((last.step, last.done, last.total, last.fraction), ("Measuring", last.total, last.total, 1.0));
        assert!(last.found > 0 && seen.iter().any(|p| p.step == "Checking projects" && p.total == 1));
        let shop = items.iter().find(|i| i.key.ends_with("code/shop")).expect("project listed");
        assert_eq!((shop.group, shop.category()), (Group::Unused, Category::Projects));
        assert_eq!(shop.removal, Removal::Trash);
        assert_eq!(shop.detail.as_deref(), Some("node_modules, src-tauri/target"));
        assert_eq!(shop.reason, "Project untouched 6 months");
        assert!(shop.recommended);
        assert!(shop.paths.iter().all(|p| !p.ends_with("tools/target") && !p.ends_with(".venv")));
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn recent_work_keeps_a_project_off_the_list_or_unticked() {
        let h = home("recent");
        let active = h.join("active");
        file(&active.join("package.json"), 300);
        file(&active.join("node_modules/a/index.js"), 300);
        file(&active.join("src/app.ts"), 2); // edited two days ago
        let paused = h.join("paused");
        file(&paused.join("Cargo.toml"), 45);
        file(&paused.join("target/CACHEDIR.TAG"), 45);
        // A commit counts as use even when files are old.
        let committed = h.join("committed");
        file(&committed.join("package.json"), 400);
        file(&committed.join("node_modules/a/index.js"), 400);
        age_dirs(&h, 400);
        file(&committed.join(".git/logs/HEAD"), 5);
        file(&active.join("src/app.ts"), 2);
        age(&paused, 45);
        age(&paused.join("target"), 45);
        let items = scan_all(&h, &Procs::fake(&[]));
        assert!(!items.iter().any(|i| i.key.ends_with("/active")), "edited this week");
        assert!(!items.iter().any(|i| i.key.ends_with("/committed")), "committed this week");
        let paused = items.iter().find(|i| i.key.ends_with("/paused")).expect("quiet 45 days shows");
        assert!(!paused.recommended, "not ticked before the 90-day cutoff");
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn something_working_in_a_project_blocks_it() {
        let h = home("busy");
        let p = h.join("site");
        file(&p.join("package.json"), 200);
        file(&p.join("node_modules/a/index.js"), 200);
        age_dirs(&h, 200);
        let procs = Procs::fake(&[("node", Some(&p.join("src")))]);
        let items = scan_all(&h, &procs);
        let site = items.iter().find(|i| i.key.ends_with("/site")).unwrap();
        assert!(!site.recommended);
        assert_eq!(site.note.as_deref(), Some("In use by node"));
        assert_eq!(clean(site, &h, &procs), Err(CleanError::Blocked("node is using this project. Quit it first".into())));
        assert!(p.join("node_modules").exists(), "nothing touched");
        // An editor's TypeScript server, running from elsewhere, with a file
        // open inside node_modules: the same.
        let tsserver = Procs::fake(&[("Code Helper (Plugin)", Some(&p.join("node_modules/a/index.js")))]);
        let items = scan_all(&h, &tsserver);
        let site = items.iter().find(|i| i.key.ends_with("/site")).unwrap();
        assert_eq!((site.recommended, site.note.as_deref()), (false, Some("In use by Code Helper (Plugin)")));
        assert!(matches!(clean(site, &h, &tsserver), Err(CleanError::Blocked(_))));
        assert!(p.join("node_modules").exists(), "nothing touched");
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn caches_are_deleted_but_not_while_their_tool_runs() {
        let h = home("cache");
        file(&h.join(".npm/_cacache/index-v5/aa"), 1);
        let items = scan_all(&h, &Procs::fake(&[]));
        let npm = items.iter().find(|i| i.key == "cache:npm").unwrap();
        assert_eq!((npm.group, npm.recommended, npm.category()), (Group::Rebuilds, true, Category::Caches));
        assert_eq!(npm.detail.as_deref(), Some("~/.npm/_cacache"));
        let running = Procs::fake(&[("npm", None)]);
        assert!(matches!(clean(npm, &h, &running), Err(CleanError::Blocked(_))));
        assert!(!scan_all(&h, &running).iter().find(|i| i.key == "cache:npm").unwrap().recommended);
        let done = clean(npm, &h, &Procs::fake(&[])).unwrap();
        assert!(!done.trashed && done.failed == 0 && done.bytes > 0);
        assert!(!h.join(".npm/_cacache").exists());
        assert!(h.join(".npm").exists(), "only the cache folder goes");
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn a_folder_that_changed_since_the_scan_is_left_alone() {
        let h = home("changed");
        let p = h.join("old-app");
        file(&p.join("package.json"), 200);
        file(&p.join("node_modules/a/index.js"), 200);
        age_dirs(&h, 200);
        let items = scan_all(&h, &Procs::fake(&[]));
        let item = items.iter().find(|i| i.key.ends_with("/old-app")).unwrap();
        fs::remove_file(p.join("package.json")).unwrap();
        assert_eq!(clean(item, &h, &Procs::fake(&[])), Err(CleanError::Blocked("Changed since the scan. Rescan".into())));
        assert!(p.join("node_modules").exists());
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn never_touches_symlinks_home_itself_or_outside_home() {
        let h = home("safety");
        let outside = home("safety-outside");
        file(&outside.join("precious/data.txt"), 1);
        fs::create_dir_all(h.join("code")).unwrap();
        std::os::unix::fs::symlink(outside.join("precious"), h.join("code/link")).unwrap();
        std::os::unix::fs::symlink(&outside, h.join("via")).unwrap();
        fs::create_dir_all(h.join("Documents")).unwrap();
        assert!(safe_target(&h, &h.join("code/link")).is_err(), "symlink");
        assert!(safe_target(&h, &h.join("via/precious")).is_err(), "symlinked parent");
        assert!(safe_target(&h, &h.join("Documents")).is_err(), "top of home");
        assert!(safe_target(&h, &outside.join("precious")).is_err(), "outside home");
        assert!(safe_target(&h, &h.join("code/../code")).is_err(), "dot-dot");
        assert!(safe_target(&h, &h.join("code")).is_err());
        fs::create_dir_all(h.join("code/app")).unwrap();
        assert!(safe_target(&h, &h.join("code/app")).is_ok());
        // A cache whose folder became a symlink is refused, not followed.
        fs::create_dir_all(h.join(".npm")).unwrap();
        std::os::unix::fs::symlink(outside.join("precious"), h.join(".npm/_cacache")).unwrap();
        let mut item = Item::new("cache:npm".into(), Group::Rebuilds, "npm cache".into(), Removal::Delete).path(h.join(".npm/_cacache"), None);
        item.sizes = vec![1];
        assert!(matches!(clean(&item, &h, &Procs::fake(&[])), Err(CleanError::Blocked(_))));
        assert!(outside.join("precious/data.txt").exists());
        let _ = fs::remove_dir_all(h);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn only_the_newest_device_files_per_phone_are_kept() {
        let h = home("devsupport");
        let root = h.join("Library/Developer/Xcode/iOS DeviceSupport");
        for (name, days) in [("iPhone17,2 26.1 (23B85)", 300), ("iPhone17,2 26.2.1 (23C71)", 200), ("iPhone17,2 27.0 (24A5390f)", 70), ("iPhone15,3 18.4 (22E240)", 400)] {
            file(&root.join(name).join("Info.plist"), days);
            age(&root.join(name), days);
        }
        let items = scan_all(&h, &Procs::fake(&[]));
        let ds = items.iter().find(|i| i.key == "devsupport:iOS").unwrap();
        assert_eq!(ds.title, "Old iOS device files ×2");
        assert_eq!(ds.detail.as_deref(), Some("26.1, 26.2.1"));
        assert_eq!(ds.removal, Removal::Delete);
        assert!(ds.recommended);
        assert_eq!(parse_support("16.4 (20E247) arm64e"), Some((String::new(), vec![16, 4], "16.4".into())));
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn precious_things_are_never_pre_ticked() {
        let h = home("precious");
        let arch = h.join("Library/Developer/Xcode/Archives/2025-01-02/Pew2 2025-01-02 10.11.12.xcarchive");
        file(&arch.join("Info.plist"), 500);
        age(&arch, 500);
        file(&h.join(".cache/huggingface/hub/models--Qwen--Qwen3-8B/snapshots/abc/model.safetensors"), 400);
        file(&h.join(".android/avd/Pixel_8.avd/config.ini"), 400);
        file(&h.join(".android/avd/Pixel_8.ini"), 400);
        let items = scan_all(&h, &Procs::fake(&[]));
        let yours: Vec<&Item> = items.iter().filter(|i| i.group == Group::Yours).collect();
        let titles: Vec<&str> = yours.iter().map(|i| i.title.as_str()).collect();
        // Xcode archives only exist on a Mac.
        assert_eq!(titles.len(), if cfg!(target_os = "macos") { 3 } else { 2 }, "{titles:?}");
        assert!(titles.contains(&"Model: Qwen/Qwen3-8B") && titles.contains(&"Android emulator: Pixel 8"));
        assert!(!cfg!(target_os = "macos") || titles.contains(&"Old Xcode archive"));
        assert!(yours.iter().all(|i| !i.recommended && i.removal == Removal::Trash));
        let cats: Vec<Category> = yours.iter().map(|i| i.category()).collect();
        assert!(cats.contains(&Category::Mobile) && cats.contains(&Category::Models));
        let avd = yours.iter().find(|i| i.key == "avd:Pixel_8").unwrap();
        assert_eq!(avd.paths.len(), 2, "the .avd folder and its .ini");
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn linux_trash_keeps_what_the_file_manager_needs_to_put_it_back() {
        let h = home("fdtrash");
        let trash_dir = h.join("Trash");
        file(&h.join("code/app/node_modules/a/index.js"), 1);
        file(&h.join("other/app/node_modules/b.js"), 1);
        let to_trash = |p: &Path, at: &str| {
            let (parent, name, _) = pinned::locate(&h, p, None).unwrap();
            trash_freedesktop(&parent, &name, p, &trash_dir, at)
        };
        to_trash(&h.join("code/app/node_modules"), "2026-10-24T12:30:05").unwrap();
        to_trash(&h.join("other/app/node_modules"), "2026-10-24T12:30:06").unwrap();
        assert!(!h.join("code/app/node_modules").exists());
        assert!(trash_dir.join("files/node_modules/a/index.js").exists());
        assert!(trash_dir.join("files/node_modules.2/b.js").exists(), "same name gets its own slot");
        let info = fs::read_to_string(trash_dir.join("info/node_modules.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
        assert!(info.contains("code/app/node_modules\nDeletionDate=2026-10-24T12:30:05\n"), "{info}");
        assert!(pinned::locate(&h, &h.join("missing"), None).is_err());
        assert!(!trash_dir.join("info/missing.trashinfo").exists(), "no info left behind on failure");
        let stamp = local_timestamp();
        assert_eq!((stamp.len(), &stamp[10..11]), (19, "T"));
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn size_counts_only_what_deleting_would_free() {
        let h = home("sizes");
        let data: Vec<u8> = (0..4 * 1024 * 1024u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        fs::create_dir_all(h.join("own")).unwrap();
        fs::write(h.join("own/a.bin"), &data).unwrap();
        let own = size_of(&h.join("own"));
        let four_mb = 4 * 1024 * 1024..5 * 1024 * 1024;
        assert!(four_mb.contains(&own), "a plain file counts fully: {own}");
        // A clone outside the folder shares every block: deleting frees ~nothing.
        fs::create_dir_all(h.join("store")).unwrap();
        fs::write(h.join("store/pkg.bin"), &data).unwrap();
        fs::create_dir_all(h.join("project/node_modules")).unwrap();
        let ok = Command::new("/bin/cp").args(["-c"]).arg(h.join("store/pkg.bin")).arg(h.join("project/node_modules/pkg.bin")).status().unwrap();
        assert!(ok.success(), "APFS clone");
        let cloned = size_of(&h.join("project/node_modules"));
        assert!(cloned < 64 * 1024, "a clone of the store frees nothing: {cloned}");
        // Hard links: only when every link is inside.
        fs::create_dir_all(h.join("links/in")).unwrap();
        fs::write(h.join("links/in/x.bin"), &data).unwrap();
        fs::hard_link(h.join("links/in/x.bin"), h.join("links/in/y.bin")).unwrap();
        let both = size_of(&h.join("links/in"));
        assert!(four_mb.contains(&both), "both links inside, counted once: {both}");
        fs::hard_link(h.join("links/in/x.bin"), h.join("links/outside.bin")).unwrap();
        assert!(size_of(&h.join("links/in")) < 64 * 1024, "a link outside keeps the data alive");
        // A symlink to big data counts as nothing, and isn't followed.
        fs::create_dir_all(h.join("via")).unwrap();
        std::os::unix::fs::symlink(h.join("own"), h.join("via/own")).unwrap();
        assert!(size_of(&h.join("via")) < 64 * 1024);
        // The fast path and the plain one agree on plain files.
        let mut t = Tally::default();
        let mut dirs = Vec::new();
        use std::os::unix::fs::MetadataExt;
        plain_dir(&h.join("own"), fs::metadata(&h).unwrap().dev(), &mut t, &mut dirs);
        assert_eq!(t.total(), own);
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn removal_only_touches_the_very_folder_the_scan_saw() {
        let h = home("pinned");
        let outside = home("pinned-outside");
        file(&outside.join("precious/keep.txt"), 1);
        let p = h.join("code/app");
        file(&p.join("package.json"), 200);
        file(&p.join("node_modules/a/index.js"), 200);
        // A symlink inside node_modules pointing out: removed as a link, never followed.
        std::os::unix::fs::symlink(outside.join("precious"), p.join("node_modules/escape")).unwrap();
        age_dirs(&h, 200);
        let item = scan_all(&h, &Procs::fake(&[])).into_iter().find(|i| i.key.ends_with("/app")).expect("offered");
        // Swapped for a different folder of the same name: refused.
        let real = h.join("code/app-moved");
        fs::rename(&p, &real).unwrap();
        file(&p.join("package.json"), 1);
        file(&p.join("node_modules/new/index.js"), 1);
        assert!(clean(&item, &h, &Procs::fake(&[])).is_err(), "a stand-in is refused");
        assert!(p.join("node_modules/new/index.js").exists(), "the stand-in is left alone");
        // Even with fresh-work checks out of the way, its identity gives it away.
        assert_eq!(pinned::locate(&h, &p.join("node_modules"), item.ids[0]).err(), Some(CHANGED.to_string()));
        // The real one, back in place, cleans: by handle, links not followed.
        fs::remove_dir_all(&p).unwrap();
        fs::rename(&real, &p).unwrap();
        let (parent, name, id) = pinned::locate(&h, &p.join("node_modules"), item.ids[0]).unwrap();
        pinned::remove_at(&parent, &name, id.dev, 0).unwrap();
        assert!(!p.join("node_modules").exists());
        assert!(outside.join("precious/keep.txt").exists(), "never followed the link out");
        // A parent swapped for a symlink is never walked through.
        fs::create_dir_all(outside.join("decoy/node_modules")).unwrap();
        fs::rename(&p, &real).unwrap();
        std::os::unix::fs::symlink(outside.join("decoy"), &p).unwrap();
        assert!(pinned::locate(&h, &p.join("node_modules"), None).is_err());
        assert!(outside.join("decoy/node_modules").exists());
        let _ = fs::remove_dir_all(h);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn open_files_are_read_from_lsof() {
        let out = b"p101\nfcwd\nn/Users/me/site\nftxt\nn/Users/me/site/node_modules/esbuild/bin/esbuild\nf12\nn/Users/me/x.log (deleted)\nf13\nn127.0.0.1:5173\np202\nf4\nn/tmp/a\nf5\nn/Users/me/caf\\xc3\\xa9 app/node_modules/x\n";
        let files = parse_lsof(out);
        assert_eq!(files[&101], vec![PathBuf::from("/Users/me/site"), PathBuf::from("/Users/me/site/node_modules/esbuild/bin/esbuild"), PathBuf::from("/Users/me/x.log")]);
        assert_eq!(files[&202], vec![PathBuf::from("/tmp/a"), PathBuf::from("/Users/me/caf\u{e9} app/node_modules/x")]);
        assert!(is_system_program(Some(Path::new("/System/Library/PrivateFrameworks/StorageManagement.framework/PlugIns/StorageManagementService"))));
        assert!(is_system_program(Some(Path::new("/usr/libexec/mds_stores"))));
        assert!(!is_system_program(Some(Path::new("/usr/bin/git"))), "tools you run count");
        assert!(!is_system_program(Some(Path::new("/usr/bin/ruby"))));
        assert!(!is_system_program(Some(Path::new("/usr/local/bin/node"))));
        assert!(!is_system_program(Some(Path::new("/opt/homebrew/bin/node"))));
        assert!(!is_system_program(Some(Path::new("/bin/zsh"))), "a shell sitting in a project counts");
        // The real thing: this test's own files show up.
        let mine = open_files().expect("lsof works");
        assert!(mine.get(&std::process::id()).is_some_and(|f| !f.is_empty()), "lsof sees this process");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn open_files_are_read_from_proc() {
        let h = home("procfs");
        let held = h.join("caf\u{e9} app/node_modules/x.js");
        file(&held, 1);
        let _f = fs::File::open(&held).unwrap();
        let mine = open_files().expect("/proc readable");
        assert!(mine[&std::process::id()].contains(&held), "sees this process's open file");
        // Another program holding it (Butters never counts itself).
        let mut other = Command::new("tail").arg("-f").arg(&held).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let who = Procs::current().using(&h.join("caf\u{e9} app"));
        let _ = other.kill();
        let _ = other.wait();
        assert_eq!(who.as_deref(), Some("tail"));
        assert!(!is_system_program(Some(Path::new("/usr/bin/node"))), "no exemptions on Linux");
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn when_in_use_cant_be_checked_nothing_is_ticked_or_cleaned() {
        let h = home("unknown");
        file(&h.join(".npm/_cacache/a"), 1);
        let blind = Procs { list: Vec::new(), open_known: false };
        let items = scan_all(&h, &blind);
        let npm = items.iter().find(|i| i.key == "cache:npm").unwrap();
        assert_eq!((npm.recommended, npm.note.as_deref()), (false, Some("Couldn't check what's in use")));
        assert!(matches!(clean(npm, &h, &blind), Err(CleanError::Blocked(_))));
        assert!(h.join(".npm/_cacache/a").exists());
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn deep_edits_and_work_since_the_scan_count_as_use() {
        let h = home("deepedit");
        let p = h.join("app");
        file(&p.join("package.json"), 300);
        file(&p.join("node_modules/a/index.js"), 300);
        file(&p.join("src/app/components/ui/forms/inputs/x.tsx"), 300);
        age_dirs(&h, 300);
        let items = scan_all(&h, &Procs::fake(&[]));
        let item = items.iter().find(|i| i.key.ends_with("/app")).expect("dormant project offered").clone();
        // An edit deep inside (no folder dates change) after the scan.
        file(&p.join("src/app/components/ui/forms/inputs/x.tsx"), 0);
        assert_eq!(clean(&item, &h, &Procs::fake(&[])), Err(CleanError::Blocked("Worked on since the scan. Look again".into())));
        assert!(p.join("node_modules").exists());
        // And a fresh scan doesn't offer it at all.
        assert!(!scan_all(&h, &Procs::fake(&[])).iter().any(|i| i.key.ends_with("/app")));
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn program_names_match_across_systems() {
        assert_eq!(program_name("Node.exe"), "node");
        assert_eq!(program_name("Xcode"), "xcode");
        assert_eq!(program_name("npm-cli.js"), "npm-cli.js");
        assert!(inside(Path::new("/a/b/c"), Path::new("/a/b")) && !inside(Path::new("/a/bc"), Path::new("/a/b")));
        let home = Path::new("/Users/me");
        assert_eq!(under(home, "AppData/Local/uv/cache"), home.join("AppData").join("Local").join("uv").join("cache"));
    }

    #[test]
    fn small_things_and_order() {
        let h = home("order");
        file(&h.join(".npm/_cacache/a"), 1);
        assert!(scan(&h, now(), 90, MIN_BYTES, &Procs::fake(&[]), &|_| {}).is_empty(), "tiny caches aren't worth a row");
        let _ = fs::remove_dir_all(h);
        assert_eq!(span(0), "1 day");
        assert_eq!(span(20 * DAY), "2 weeks");
        assert_eq!(span(200 * DAY), "6 months");
        assert_eq!(span(800 * DAY), "2 years");
        assert_eq!(runtime_name("com.apple.CoreSimulator.SimRuntime.iOS-26-1"), "iOS 26.1");
        assert!(is_udid("907B5197-3CD8-4F8A-A7AF-9A83D10BE6D4"));
        assert!(!is_udid("907B5197-3CD8-4F8A-A7AF-9A83D10BE6D4; rm"));
        assert!(!is_udid("-07B5197-3CD8-4F8A-A7AF-9A83D10BE6D4"), "never starts like an option");
        assert!(!is_udid("907B51973CD8-4F8A-A7AF-9A83D10BE6D4-"));
    }
}

// Windows behaviour (CI runs these on windows-latest).
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::fs;
    use std::os::windows::fs::OpenOptionsExt;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("butters-deep-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        canonical(&dir).unwrap()
    }

    #[test]
    fn a_folder_with_an_open_file_is_refused_whole() {
        let h = home("locked");
        let nm = h.join("app").join("node_modules");
        fs::create_dir_all(nm.join("a")).unwrap();
        fs::write(nm.join("a").join("index.js"), b"x").unwrap();
        fs::write(nm.join("b.js"), b"y").unwrap();
        // Held open without delete sharing, as a running tool would.
        let held = fs::OpenOptions::new().read(true).share_mode(0x1).open(nm.join("a").join("index.js")).unwrap();
        assert_eq!(remove(&h, &nm, None), Err(IN_USE.to_string()));
        assert!(nm.join("b.js").exists(), "nothing removed");
        drop(held);
        remove(&h, &nm, None).unwrap();
        assert!(!nm.exists());
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn hard_links_count_once_and_only_when_all_are_inside() {
        let h = home("winlinks");
        let data = vec![7u8; 1 << 20];
        fs::create_dir_all(h.join("in")).unwrap();
        fs::write(h.join("in").join("x.bin"), &data).unwrap();
        let one = size_of(&h.join("in"));
        assert!(one >= 1 << 20, "{one}");
        fs::hard_link(h.join("in").join("x.bin"), h.join("in").join("y.bin")).unwrap();
        assert_eq!(size_of(&h.join("in")), one, "two links inside, counted once");
        fs::hard_link(h.join("in").join("x.bin"), h.join("outside.bin")).unwrap();
        assert_eq!(size_of(&h.join("in")), 0, "a link outside keeps the data alive");
        let _ = fs::remove_dir_all(h);
    }

    #[test]
    fn paths_compare_without_letter_case() {
        assert!(inside(Path::new(r"C:\Users\Me\code\app"), Path::new(r"c:\users\me")));
        assert!(!inside(Path::new(r"C:\Users\Mel\x"), Path::new(r"C:\Users\Me")));
    }
}
