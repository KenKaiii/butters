//! What the panel would actually list for the lab (same merge rules as the
//! app: identical leftovers fold into one "×N" row). Read-only.
use std::collections::HashSet;
use butters_lib::engine::assess::{self, Context, Verdict};
use butters_lib::{platform, sampler};

fn main() {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).expect("home folder");
    let lab = std::path::Path::new(&home).join(".butters-lab").to_string_lossy().into_owned();
    let ports = sampler::listening_ports();
    let mut s = sampler::Sampler::new();
    let mut procs = s.snapshot(&ports);
    let mut keep: HashSet<u32> = procs.iter().filter(|p| p.cmd.iter().any(|a| a.contains(&lab)) || p.exe.as_deref().is_some_and(|e| e.contains(&lab)) || p.cwd.as_deref().is_some_and(|c| c.starts_with(&lab))).map(|p| p.pid).collect();
    loop { let n = keep.len(); for p in &procs { if p.parent.is_some_and(|pp| keep.contains(&pp)) { keep.insert(p.pid); } } if keep.len() == n { break; } }
    procs.retain(|p| keep.contains(&p.pid));
    for p in procs.iter_mut() { p.idle_secs = 3 * 3600 + 600; p.idle_is_lower_bound = false; }
    let ctx = Context { self_pid: std::process::id(), frontmost_pid: platform::frontmost_pid(), watched_secs: 4 * 3600, ..Default::default() };
    // Same fold as app.rs: orphaned + same kind + same title + same reason.
    let mut rows: Vec<(String, String, bool, u64, usize)> = Vec::new();
    for a in assess::assess(&procs, &ctx) {
        if let Verdict::Suggest { reason, recommended, .. } = a.verdict {
            let g = a.group;
            if g.orphaned && let Some(r) = rows.iter_mut().find(|r| r.0 == g.title && r.1 == reason) { r.3 += g.memory; r.4 += 1; continue; }
            rows.push((g.title, reason, recommended, g.memory, 1));
        }
    }
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)));
    let total: u64 = rows.iter().filter(|r| r.2).map(|r| r.3).sum();
    println!("{} rows, {} pre-ticked, frees {:.1} GB\n", rows.len(), rows.iter().filter(|r| r.2).count(), total as f64 / 1e9);
    for (t, r, rec, m, n) in rows {
        let title = if n > 1 { format!("{t} ×{n}") } else { t };
        println!("{} {:42} {:28} {:>7.0} MB", if rec { "[x]" } else { "[ ]" }, title, r, m as f64 / 1048576.0);
    }
}
