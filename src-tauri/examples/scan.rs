//! Dry run: prints what Butters would suggest and protect on this machine.
//! Read-only; never closes anything. `cargo run --example scan`
use butters_lib::engine::assess::{self, fmt_bytes, Context, Verdict};
use butters_lib::{platform, sampler};

fn main() {
    let t = std::time::Instant::now();
    let ports = sampler::listening_ports();
    let mut s = sampler::Sampler::new();
    let procs = s.snapshot(&ports);
    let stats = s.stats();
    let ctx = Context { self_pid: std::process::id(), frontmost_pid: platform::frontmost_pid(), watched_secs: 0, ..Default::default() };
    let out = assess::assess(&procs, &ctx);
    println!("{} processes, scan {} ms, {:?}", procs.len(), t.elapsed().as_millis(), stats);
    for a in &out {
        let g = &a.group;
        let tag = match &a.verdict {
            Verdict::Suggest { reason, recommended, note } => format!("SUGGEST{} {reason} {}", if *recommended { "*" } else { "" }, note.clone().unwrap_or_default()),
            Verdict::Protected(p) => format!("PROTECT {}", p.text()),
            Verdict::Quiet => "quiet".into(),
        };
        println!("{:<16?} {:<40} {:>8} {:>5.0}% idle {:>6}s orphan={} ports={:?} host={:?} | {tag}",
            g.kind, g.title.chars().take(40).collect::<String>(), fmt_bytes(g.memory), g.cpu, g.idle_secs, g.orphaned, g.ports, g.host_app);
    }
}
