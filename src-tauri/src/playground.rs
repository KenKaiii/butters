//! Butters Playground: a separate app (built with `--features playground`)
//! that starts a set of real, throwaway processes for Butters to find and
//! close, so the whole flow can be tried again and again.
//!
//! Safety: the Playground only ever *sees* the processes it started (every
//! other process on the machine is filtered out before any rule runs), and
//! its own data lives under a different app id. It can't touch your work.
//!
//! Each actor is a small node script laid out like the real tool
//! (~/.butters-playground/shop-web/node_modules/vite/bin/vite.js...), so
//! Butters classifies it exactly like the real thing. Scripts are fixed
//! strings in this file, written fresh on every reset.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;

/// One fake thing to close.
struct Actor {
    /// Shown in the Playground bar.
    label: &'static str,
    /// The script's path inside the playground folder; the folder layout
    /// (node_modules/vite/bin/vite.js etc.) is what makes Butters recognise it.
    path: &'static str,
    /// Extra args after the script.
    args: &'static [&'static str],
    /// What the process does.
    script: &'static str,
    /// Parent dies right away so it shows as a leftover ("Terminal closed").
    /// macOS/Linux only; on Windows these simply run as normal children.
    #[cfg_attr(not(unix), allow(dead_code))]
    orphan: bool,
    /// Working folder (shown as the project), inside the playground folder.
    project: &'static str,
}

/// Listens on a port and holds a little memory, like a dev server.
const SERVER: &str = "const b=Buffer.alloc(60*1024*1024,1);require('http').createServer((q,s)=>s.end('ok')).listen(0,'127.0.0.1');setInterval(()=>b[0]++,1e9)";
/// Takes 2.5 s to stop after a polite request, like an app that saves first.
const SLOW: &str = "const b=Buffer.alloc(40*1024*1024,1);require('http').createServer((q,s)=>s.end('ok')).listen(0,'127.0.0.1');setInterval(()=>b[0]++,1e9);process.on('SIGTERM',()=>setTimeout(()=>process.exit(0),2500))";
/// Holds memory and does nothing (an idle agent or MCP server).
const IDLE: &str = "const b=Buffer.alloc(120*1024*1024,1);setInterval(()=>b[0]++,1e9)";
/// Keeps one core busy.
const BUSY: &str = "for(;;){Math.sqrt(Math.random())}";
/// Grows ~20 MB every 2 s and never gives it back.
const LEAK: &str = "const k=[];setInterval(()=>k.push(Buffer.alloc(20*1024*1024,1)),2000)";

const ACTORS: &[Actor] = &[
    Actor { label: "Vite dev server", path: "node_modules/vite/bin/vite.js", args: &["--port", "5173"], script: SERVER, orphan: true, project: "shop-web" },
    Actor { label: "Next.js dev server", path: "node_modules/next/dist/bin/next", args: &["dev"], script: SERVER, orphan: true, project: "marketing-site" },
    Actor { label: "Storybook", path: "node_modules/storybook/bin/index.js", args: &["dev"], script: SLOW, orphan: true, project: "design-system" },
    Actor { label: "Claude Code session", path: "node_modules/@anthropic-ai/claude-code/cli.js", args: &[], script: IDLE, orphan: true, project: "api" },
    Actor { label: "MCP server", path: "node_modules/@modelcontextprotocol/server-memory/dist/index.js", args: &[], script: IDLE, orphan: true, project: "api" },
    Actor { label: "Busy build", path: "node_modules/webpack/bin/webpack.js", args: &["--watch"], script: BUSY, orphan: false, project: "shop-web" },
    Actor { label: "Leaky process", path: "leaky-worker.js", args: &[], script: LEAK, orphan: false, project: "shop-web" },
];

/// The playground folder: ~/.butters-playground (the user's own space, no
/// shared /tmp). Scripts are written fresh on every reset from the fixed
/// strings above.
fn root() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).ok_or("No home folder")?;
    Ok(PathBuf::from(home).join(".butters-playground"))
}

fn prepare(a: &Actor) -> Result<(PathBuf, PathBuf), String> {
    let root = root()?;
    let project = root.join(a.project);
    let script = project.join(a.path);
    if let Some(dir) = script.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&script, a.script).map_err(|e| e.to_string())?;
    Ok((project, script))
}

#[derive(Default)]
pub struct Playground {
    /// (pid, start time) of every process the Playground started.
    pids: Mutex<HashSet<u32>>,
    children: Mutex<Vec<Child>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundStatus {
    pub running: usize,
    pub total: usize,
    pub node: bool,
}

fn node() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = ["/opt/homebrew/bin/node", "/usr/local/bin/node"].iter().map(PathBuf::from).collect();
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".volta/bin/node"));
        if let Ok(dir) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut v: Vec<PathBuf> = dir.flatten().map(|e| e.path().join("bin/node")).collect();
            v.sort();
            candidates.extend(v.into_iter().rev());
        }
    }
    #[cfg(windows)]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            candidates.insert(0, PathBuf::from(pf).join("nodejs\\node.exe"));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(if cfg!(windows) { "node.exe" } else { "node" })));
    }
    candidates.into_iter().find(|p| p.is_file())
}

impl Playground {
    /// Is this pid one of ours (still the same process)?
    pub fn owns(&self, pid: u32) -> bool {
        self.pids.lock().map(|p| p.contains(&pid)).unwrap_or(false)
    }

    pub fn pids(&self) -> HashSet<u32> {
        self.pids.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// Stops anything still running (including leftovers from an earlier run
    /// that was force-quit), then starts every actor fresh.
    pub fn reset(&self) -> Result<PlaygroundStatus, String> {
        self.stop_all();
        stop_strays();
        let node = node().ok_or("Node.js isn't installed, so the Playground can't start its test processes")?;
        let mut pids = self.pids.lock().map_err(|_| "busy")?;
        let mut children = self.children.lock().map_err(|_| "busy")?;
        for a in ACTORS {
            let (project, script) = prepare(a)?;
            #[cfg(unix)]
            if a.orphan {
                // sh starts node in the background and exits at once, so node
                // is re-parented to launchd: a leftover whose terminal is gone.
                let out = Command::new("/bin/sh")
                    .arg("-c")
                    .arg("\"$@\" >/dev/null 2>&1 & echo $!")
                    .arg("butters-playground")
                    .arg(&node)
                    .arg(&script)
                    .args(a.args)
                    .current_dir(&project)
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| e.to_string())?;
                if let Ok(pid) = String::from_utf8_lossy(&out.stdout).trim().parse::<u32>() {
                    pids.insert(pid);
                }
                continue;
            }
            let child = Command::new(&node)
                .arg(&script)
                .args(a.args)
                .current_dir(&project)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            pids.insert(child.id());
            children.push(child);
        }
        drop(pids);
        drop(children);
        Ok(self.status())
    }

    pub fn status(&self) -> PlaygroundStatus {
        let running = self.pids().into_iter().filter(|p| alive(*p)).count();
        PlaygroundStatus { running, total: ACTORS.len(), node: node().is_some() }
    }

    /// Kills every process the Playground started (on reset and on quit).
    pub fn stop_all(&self) {
        if let Ok(mut children) = self.children.lock() {
            for c in children.iter_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
            children.clear();
        }
        if let Ok(mut pids) = self.pids.lock() {
            for pid in pids.drain() {
                kill(pid);
            }
        }
    }

    pub fn labels() -> Vec<&'static str> {
        ACTORS.iter().map(|a| a.label).collect()
    }
}

/// Kills any of the signed-in user's processes running a script from the
/// playground folder. Only that folder: nothing else can match.
fn stop_strays() {
    let Ok(root) = root() else { return };
    let root = root.to_string_lossy().into_owned();
    let me = sysinfo::get_current_pid().ok();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always).with_user(sysinfo::UpdateKind::Always),
    );
    let my_uid = me.and_then(|m| sys.process(m)).and_then(|p| p.user_id().cloned());
    for (pid, p) in sys.processes() {
        let ours = my_uid.is_some() && p.user_id() == my_uid.as_ref();
        let in_playground = p.cmd().iter().skip(1).take(2).any(|a| a.to_string_lossy().starts_with(&root));
        if ours && in_playground && Some(*pid) != me {
            kill(pid.as_u32());
        }
    }
}

fn alive(pid: u32) -> bool {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[p]), true, sysinfo::ProcessRefreshKind::nothing());
    sys.process(p).is_some_and(|p| !matches!(p.status(), sysinfo::ProcessStatus::Zombie))
}

fn kill(pid: u32) {
    #[cfg(unix)]
    {
        let _ = crate::actions::send_signal(pid, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let mut sys = sysinfo::System::new();
        let p = sysinfo::Pid::from_u32(pid);
        sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[p]), true, sysinfo::ProcessRefreshKind::nothing());
        if let Some(proc_) = sys.process(p) {
            proc_.kill();
        }
    }
}

impl Drop for Playground {
    fn drop(&mut self) {
        self.stop_all();
    }
}
