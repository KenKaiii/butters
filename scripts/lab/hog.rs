// Stand-in for a native tool in the Butters lab. Copied to a real tool's
// path/name (claude, codex, qemu-system-aarch64, Chromium...) so the process
// looks exactly like it to Butters. Options (env): HOG_MB=n holds n MB,
// HOG_PORT=p listens on 127.0.0.1:p (0 = any), HOG_SPIN=1 burns a core,
// HOG_GROW=n leaks n MB every 2 s.
use std::io::Read;
use std::net::TcpListener;
use std::{env, thread, time::Duration};

fn main() {
    let get = |k: &str| env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
    let mut keep: Vec<Vec<u8>> = Vec::new();
    if let Some(mb) = get("HOG_MB") {
        // Touch every page (a vec![1; n] can be lazily zero-mapped and stay
        // virtual), so it shows up as resident memory like a real tool's.
        let mut v = vec![0u8; (mb * 1024 * 1024) as usize];
        for i in (0..v.len()).step_by(4096) {
            v[i] = 1;
        }
        keep.push(v);
    }
    if let Some(port) = get("HOG_PORT") {
        let l = TcpListener::bind(("127.0.0.1", port as u16)).or_else(|_| TcpListener::bind(("127.0.0.1", 0))).unwrap();
        thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut b = [0u8; 512];
                let _ = s.read(&mut b);
            }
        });
    }
    if env::var("HOG_SPIN").is_ok() {
        thread::spawn(|| {
            let mut x: u64 = 1;
            loop {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                std::hint::black_box(x);
            }
        });
    }
    let grow = get("HOG_GROW");
    loop {
        if let Some(mb) = grow {
            let mut v = vec![0u8; (mb * 1024 * 1024) as usize];
            for i in (0..v.len()).step_by(4096) {
                v[i] = 2;
            }
            keep.push(v);
        }
        thread::sleep(Duration::from_secs(2));
        std::hint::black_box(&keep);
    }
}
