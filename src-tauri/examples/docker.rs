//! Dry run: what Butters sees in Docker. Read-only; removes nothing.
//! `cargo run --example docker`
use butters_lib::services;

fn main() {
    let gb = |b: u64| format!("{:.2} GB", b as f64 / 1e9);
    match services::docker_containers() {
        None => println!("Docker isn't running"),
        Some(cs) => {
            println!("running containers: {}", cs.len());
            for c in &cs {
                println!("  {} ({}) cpu {:.1}% mem {} io {} started {:?} project {:?}", c.name, c.image, c.cpu, gb(c.memory), gb(c.io), c.started_at, c.project);
            }
        }
    }
    if let Some(d) = services::docker_disk() {
        println!("build cache: {} (last used {:?} s ago)", gb(d.build_cache), d.build_cache_idle);
        println!("untagged images: {}", gb(d.dangling_images));
        println!("old image versions: {} ({})", d.old_images.len(), gb(d.old_images.iter().map(|i| i.size).sum()));
        for i in &d.old_images {
            println!("  {}  {}", gb(i.size), i.reference);
        }
    }
    println!("VM memory limit: {:?}", services::docker_vm_memory().map(gb));
}
