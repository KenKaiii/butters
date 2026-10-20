//! Scores the detection lab (scripts/lab/lab.py): runs the real Butters
//! engine on this machine, keeps only the lab's processes (everything under
//! ~/.butters-lab plus their children), and reports per scenario whether
//! Butters did what a heavy agent user would expect. Read-only.
//!
//!   python3 scripts/lab/lab.py up && cargo run --example lab
//!
//! "aged" scenarios get idle time set to 3 h, standing in for "left from
//! yesterday" (the real idle clock would need hours of waiting).
use std::collections::{HashMap, HashSet};

use butters_lib::engine::assess::{self, Context, Verdict};
use butters_lib::{platform, sampler};
use serde_json::Value;

fn main() {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).expect("home folder");
    let lab_dir = std::path::Path::new(&home).join(".butters-lab");
    let lab = lab_dir.to_string_lossy().into_owned();
    let projects = lab_dir.join("projects");
    let scenarios: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(lab_dir.join("scenarios.json")).expect("run lab.py up first")).unwrap();
    let state: Value = serde_json::from_str(&std::fs::read_to_string(lab_dir.join("state.json")).unwrap()).unwrap();
    let failed: HashMap<String, String> = state["failed"].as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
    let roots: HashMap<String, u32> = state["procs"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_u64().unwrap() as u32)).collect();

    let ports = sampler::listening_ports();
    let mut s = sampler::Sampler::new();
    let mut procs = s.snapshot(&ports);

    // Lab processes: command line or exe inside ~/.butters-lab, plus descendants.
    let mut keep: HashSet<u32> = procs
        .iter()
        .filter(|p| p.cmd.iter().any(|a| a.contains(&lab)) || p.exe.as_deref().is_some_and(|e| e.contains(&lab)) || p.cwd.as_deref().is_some_and(|c| c.starts_with(&lab)))
        .map(|p| p.pid)
        .collect();
    keep.extend(roots.values());
    loop {
        let n = keep.len();
        for p in &procs {
            if p.parent.is_some_and(|pp| keep.contains(&pp)) {
                keep.insert(p.pid);
            }
        }
        if keep.len() == n {
            break;
        }
    }
    // Which scenario owns each pid: walk up to a recorded root, else by cwd.
    let by_pid: HashMap<u32, Option<u32>> = procs.iter().map(|p| (p.pid, p.parent)).collect();
    let cwd_of: HashMap<u32, String> = procs.iter().filter_map(|p| p.cwd.clone().map(|c| (p.pid, c))).collect();
    let owner = |pid: u32| -> Option<String> {
        let mut cur = Some(pid);
        for _ in 0..30 {
            let c = cur?;
            if let Some((id, _)) = roots.iter().find(|(_, r)| **r == c) {
                return Some(id.clone());
            }
            cur = by_pid.get(&c).copied().flatten();
        }
        let cwd = cwd_of.get(&pid)?;
        let rel = std::path::Path::new(cwd).strip_prefix(&projects).ok()?;
        rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned())
    };
    let aged: HashSet<String> = scenarios.iter().filter(|s| s["aged"].as_bool() == Some(true)).map(|s| s["id"].as_str().unwrap().to_string()).collect();
    // The shell parents of non-orphan scenarios stand in for a terminal you
    // have open; mark them as children of a fake Ghostty so the "in a
    // terminal" rules apply. (Ghostty isn't frontmost: nothing is in use.)
    procs.retain(|p| keep.contains(&p.pid));
    for p in procs.iter_mut() {
        if let Some(id) = owner(p.pid)
            && aged.contains(&id) {
                p.idle_secs = 3 * 3600 + 600;
                p.idle_is_lower_bound = false;
            }
        if p.cmd.iter().any(|a| a.contains("worker.js")) {
            p.growth = Some((2_100 * 1024 * 1024, 40 * 60));
        }
    }
    let ctx = Context { self_pid: std::process::id(), frontmost_pid: platform::frontmost_pid(), watched_secs: 4 * 3600, ..Default::default() };
    let out = assess::assess(&procs, &ctx);

    // Verdict per scenario: the group that contains the scenario's processes.
    let mut results: HashMap<String, Vec<String>> = HashMap::new();
    for a in &out {
        let ids: HashSet<String> = a.group.members.iter().filter_map(|m| owner(*m)).collect();
        let v = match &a.verdict {
            Verdict::Suggest { reason, recommended, .. } => format!("{} \"{}\" — {}", if *recommended { "PRETICK" } else { "SUGGEST" }, a.group.title, reason),
            Verdict::Protected(p) => format!("PROTECT \"{}\" — {}", a.group.title, p.text()),
            Verdict::Quiet => format!("quiet \"{}\" ({:?})", a.group.title, a.group.kind),
        };
        for id in ids {
            results.entry(id).or_default().push(v.clone());
        }
    }
    let (mut ok, mut bad) = (0, 0);
    let mut fails = Vec::new();
    for sc in &scenarios {
        let id = sc["id"].as_str().unwrap();
        let expect = sc["expect"].as_str().unwrap();
        let who = sc["who"].as_str().unwrap();
        if let Some(err) = failed.get(id) {
            bad += 1;
            fails.push(id.to_string());
            println!("FAIL  {id:15} want {expect:8} {who}\n      didn't start: {err}");
            continue;
        }
        if !roots.contains_key(id) {
            println!("SKIP  {id:15} {who} (tool not installed)");
            continue;
        }
        let got = results.get(id).cloned().unwrap_or_else(|| vec!["(not seen at all)".into()]);
        let any = |w: &str| got.iter().any(|g| g.starts_with(w));
        let pass = match expect {
            "pretick" => any("PRETICK"),
            "suggest" => any("PRETICK") || any("SUGGEST"),
            "quiet" => !any("PRETICK") && !any("SUGGEST"),
            "protect" => any("PROTECT"),
            _ => false,
        };
        if pass {
            ok += 1;
        } else {
            bad += 1;
            fails.push(id.to_string());
        }
        println!("{} {id:15} want {expect:8} {who}\n      got: {}", if pass { "PASS " } else { "FAIL " }, got.join(" | "));
    }
    println!("\n{ok} pass, {bad} fail{}", if fails.is_empty() { String::new() } else { format!(": {}", fails.join(", ")) });
    if bad > 0 {
        std::process::exit(1);
    }
}
