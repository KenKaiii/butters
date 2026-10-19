//! Docker containers and Ollama models: things that hold lots of memory but
//! should be stopped through their own tools, not by killing processes.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Debug, Clone, Default)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub cpu: f32,
    pub memory: u64,
    pub status: String,
    /// Compose project, if it belongs to one.
    pub project: Option<String>,
    /// Unix seconds when it was started.
    pub started_at: Option<u64>,
    /// Network + disk bytes moved since it started (cumulative).
    pub io: u64,
}

#[derive(Debug, Clone)]
pub struct LoadedModel {
    pub name: String,
    pub size: u64,
    /// Seconds until Ollama unloads it by itself, if known.
    pub expires_in: Option<i64>,
}

/// GUI apps don't inherit the shell's PATH, so look in the usual places.
fn find_docker() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        candidates.extend(
            [
                "/usr/local/bin/docker",
                "/opt/homebrew/bin/docker",
                "/Applications/Docker.app/Contents/Resources/bin/docker",
                "/Applications/OrbStack.app/Contents/MacOS/xbin/docker",
            ]
            .map(PathBuf::from),
        );
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(PathBuf::from(home).join(".orbstack/bin/docker"));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            candidates.push(PathBuf::from(pf).join("Docker\\Docker\\resources\\bin\\docker.exe"));
        }
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|d| d.join("docker.exe")));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Runs a program with fixed arguments (never through a shell), with a time limit.
fn run(program: &PathBuf, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("no output")?;
    // Read on a helper thread so a chatty program can't fill the pipe and stall.
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.take(4 * 1024 * 1024).read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().unwrap_or_default();
                return if status.success() { Ok(out) } else { Err("command failed".into()) };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
        }
    }
}

#[derive(Deserialize)]
struct PsLine {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Labels", default)]
    labels: String,
}

#[derive(Deserialize)]
struct StatsLine {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "CPUPerc", default)]
    cpu: String,
    #[serde(rename = "MemUsage", default)]
    mem: String,
    #[serde(rename = "NetIO", default)]
    net: String,
    #[serde(rename = "BlockIO", default)]
    block: String,
}

#[derive(Deserialize)]
struct DfLine {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "Reclaimable", default)]
    reclaimable: String,
}

/// One Docker size: "12.5MiB", "1.2kB", "3.4GB", "0B". Docker prints memory in
/// binary units (MiB) and traffic/disk in decimal ones (kB, MB, GB).
pub fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let idx = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: f64 = num.trim().parse().unwrap_or(0.0);
    let mult = match unit.trim() {
        "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0_f64.powi(3),
        "TiB" => 1024.0_f64.powi(4),
        _ => 0.0,
    };
    (n * mult).round() as u64
}

/// "12.5MiB / 7.6GiB" -> the first number (memory used).
fn parse_mem(s: &str) -> u64 {
    parse_size(s.split('/').next().unwrap_or(""))
}

/// "1.2kB / 3.4MB" -> both sides added (in + out).
fn parse_pair_sum(s: &str) -> u64 {
    s.split('/').map(parse_size).sum()
}

/// "2026-10-12T03:04:05.123456789Z" -> Unix seconds.
fn parse_started(s: &str) -> Option<u64> {
    let secs = parse_rfc3339_secs(s.trim())?;
    (secs > 0).then_some(secs as u64)
}

fn compose_project(labels: &str) -> Option<String> {
    labels
        .split(',')
        .find_map(|kv| kv.strip_prefix("com.docker.compose.project="))
        .filter(|p| !p.is_empty())
        .map(str::to_string)
}

/// "Total reclaimed space: 1.234GB" (or buildx's "Total:\t1.2GB") -> bytes.
fn parse_reclaimed(out: &str) -> u64 {
    out.lines()
        .rev()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("Total reclaimed space:").or_else(|| l.strip_prefix("Total:")).map(parse_size)
        })
        .unwrap_or(0)
}

pub fn is_container_id(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Running containers with CPU, memory, traffic and start time. `None` when
/// Docker isn't installed or its engine isn't running.
pub fn docker_containers() -> Option<Vec<Container>> {
    let docker = find_docker()?;
    let ps = run(&docker, &["ps", "--no-trunc", "--format", "{{json .}}"], Duration::from_secs(4)).ok()?;
    let mut containers: Vec<Container> = ps
        .lines()
        .filter_map(|l| serde_json::from_str::<PsLine>(l).ok())
        .filter(|c| is_container_id(&c.id))
        .map(|c| Container {
            project: compose_project(&c.labels),
            id: c.id,
            name: c.names,
            image: c.image,
            status: c.status,
            ..Default::default()
        })
        .collect();
    if containers.is_empty() {
        return Some(containers);
    }
    if let Ok(stats) = run(&docker, &["stats", "--no-stream", "--no-trunc", "--format", "{{json .}}"], Duration::from_secs(8)) {
        for s in stats.lines().filter_map(|l| serde_json::from_str::<StatsLine>(l).ok()) {
            if let Some(c) = containers.iter_mut().find(|c| c.id.starts_with(&s.id) || s.id.starts_with(&c.id)) {
                c.cpu = s.cpu.trim_end_matches('%').trim().parse().unwrap_or(0.0);
                c.memory = parse_mem(&s.mem);
                c.io = parse_pair_sum(&s.net) + parse_pair_sum(&s.block);
            }
        }
    }
    // Start times, in one call. Ids are validated hex, passed as arguments.
    let mut args = vec!["inspect", "--format", "{{.Id}} {{.State.StartedAt}}"];
    args.extend(containers.iter().map(|c| c.id.as_str()));
    if let Ok(out) = run(&docker, &args, Duration::from_secs(6)) {
        for line in out.lines() {
            if let Some((id, started)) = line.split_once(' ')
                && let Some(c) = containers.iter_mut().find(|c| c.id == id) {
                    c.started_at = parse_started(started);
                }
        }
    }
    Some(containers)
}

/// What Docker could give back on disk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DockerDisk {
    /// Build cache Docker would rebuild on demand.
    pub build_cache: u64,
    /// Untagged ("dangling") images left over from rebuilds.
    pub dangling_images: u64,
}

pub fn docker_disk() -> Option<DockerDisk> {
    let docker = find_docker()?;
    let df = run(&docker, &["system", "df", "--format", "{{json .}}"], Duration::from_secs(15)).ok()?;
    let mut disk = DockerDisk::default();
    for row in df.lines().filter_map(|l| serde_json::from_str::<DfLine>(l).ok()) {
        if row.kind == "Build Cache" {
            disk.build_cache = parse_size(row.reclaimable.split('(').next().unwrap_or(""));
        }
    }
    if let Ok(out) = run(&docker, &["images", "--filter", "dangling=true", "--format", "{{.Size}}"], Duration::from_secs(8)) {
        disk.dangling_images = out.lines().map(parse_size).sum();
    }
    Some(disk)
}

/// Clears build cache older than 3 days (recent cache stays warm). Returns bytes freed.
pub fn docker_prune_build_cache() -> Result<u64, String> {
    let docker = find_docker().ok_or("Docker isn't available")?;
    run(&docker, &["builder", "prune", "--force", "--filter", "until=72h"], Duration::from_secs(120)).map(|o| parse_reclaimed(&o))
}

/// Removes untagged images only (never ones with a name). Returns bytes freed.
pub fn docker_prune_dangling_images() -> Result<u64, String> {
    let docker = find_docker().ok_or("Docker isn't available")?;
    run(&docker, &["image", "prune", "--force"], Duration::from_secs(120)).map(|o| parse_reclaimed(&o))
}

/// How much memory Docker's VM is allowed to use, if Docker reports it.
pub fn docker_vm_memory() -> Option<u64> {
    let docker = find_docker()?;
    let out = run(&docker, &["info", "--format", "{{.MemTotal}}"], Duration::from_secs(4)).ok()?;
    out.trim().parse().ok().filter(|n: &u64| *n > 0)
}

/// `docker stop`: Docker's own polite stop (SIGTERM, then kill after 10 s).
pub fn docker_stop(id: &str) -> Result<(), String> {
    if !is_container_id(id) {
        return Err("Not a container id".into());
    }
    let docker = find_docker().ok_or("Docker isn't available")?;
    run(&docker, &["stop", id], Duration::from_secs(30)).map(|_| ())
}

#[derive(Deserialize)]
struct PsResponse {
    #[serde(default)]
    models: Vec<PsModel>,
}

#[derive(Deserialize)]
struct PsModel {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    expires_at: Option<String>,
}

const OLLAMA: &str = "http://127.0.0.1:11434";

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(1500)))
        .max_redirects(0)
        .build()
        .into()
}

/// Models Ollama currently holds in memory. `None` when Ollama isn't running.
pub fn ollama_models() -> Option<Vec<LoadedModel>> {
    let mut resp = agent().get(&format!("{OLLAMA}/api/ps")).call().ok()?;
    let body: PsResponse = resp.body_mut().with_config().limit(1024 * 1024).read_json().ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    Some(
        body.models
            .into_iter()
            .filter(|m| is_model_name(&m.name))
            .map(|m| LoadedModel {
                expires_in: m.expires_at.as_deref().and_then(parse_rfc3339_secs).map(|t| t - now),
                name: m.name,
                size: m.size,
            })
            .collect(),
    )
}

/// Ollama's documented way to unload a model right away: keep_alive = 0.
pub fn ollama_unload(name: &str) -> Result<(), String> {
    if !is_model_name(name) {
        return Err("Not a model name".into());
    }
    let body = serde_json::json!({ "model": name, "keep_alive": 0 });
    agent()
        .post(&format!("{OLLAMA}/api/generate"))
        .send_json(&body)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn is_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'/'))
}

/// Minimal RFC 3339 -> Unix seconds ("2026-10-19T10:15:30.123+02:00").
fn parse_rfc3339_secs(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let time_end = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(time_end);
    let mut t = time.split(':');
    let (hh, mm) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let ss = t.next()?.split('.').next()?.parse::<i64>().ok()?;
    let offset = match zone {
        "" | "Z" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let mut parts = z[1..].split(':');
            let oh = parts.next()?.parse::<i64>().ok()?;
            let om = parts.next().and_then(|p| p.parse::<i64>().ok()).unwrap_or(0);
            sign * (oh * 3600 + om * 60)
        }
    };
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_strings() {
        assert_eq!(parse_mem("12.5MiB / 7.6GiB"), (12.5 * 1024.0 * 1024.0) as u64);
        assert_eq!(parse_mem("1GiB / 2GiB"), 1024 * 1024 * 1024);
        assert_eq!(parse_mem("garbage"), 0);
    }

    #[test]
    fn container_ids_are_strict() {
        assert!(is_container_id("4f1c2a3b4c5d"));
        assert!(!is_container_id("4f1c; rm -rf"));
        assert!(!is_container_id("--help"));
        assert!(!is_container_id("abc"));
    }

    #[test]
    fn model_names_are_strict() {
        assert!(is_model_name("llama3.1:8b"));
        assert!(is_model_name("hf.co/org/model:Q4_K_M"));
        assert!(!is_model_name("a b"));
        assert!(!is_model_name(""));
    }

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_secs("2026-10-19T00:00:00Z"), Some(1_792_368_000));
        assert_eq!(parse_rfc3339_secs("2026-10-19T02:00:00.5+02:00"), Some(1_792_368_000));
    }

    #[test]
    fn docker_sizes_and_traffic() {
        assert_eq!(parse_size("0B"), 0);
        assert_eq!(parse_size("1.5kB"), 1500);
        assert_eq!(parse_size("2MB"), 2_000_000);
        assert_eq!(parse_size("1GiB"), 1 << 30);
        assert_eq!(parse_mem("12.5MiB / 7.6GiB"), (12.5 * 1024.0 * 1024.0) as u64);
        assert_eq!(parse_pair_sum("1.2kB / 3.4MB"), 1200 + 3_400_000);
        assert_eq!(parse_reclaimed("Deleted build cache objects:\nabc\n\nTotal reclaimed space: 1.25GB\n"), 1_250_000_000);
        assert_eq!(parse_reclaimed("ID\tRECLAIMABLE\nx\ttrue\nTotal:\t300MB"), 300_000_000);
        assert_eq!(parse_reclaimed("Total reclaimed space: 0B"), 0);
        assert_eq!(compose_project("com.docker.compose.service=db,com.docker.compose.project=shop"), Some("shop".into()));
        assert_eq!(compose_project(""), None);
        let started = parse_started("2026-10-12T03:04:05.123456789Z").unwrap();
        assert_eq!(parse_started("0001-01-01T00:00:00Z"), None);
        assert!(started > 1_790_000_000);
    }

    #[test]
    fn docker_json_lines_parse() {
        let stats = r#"{"BlockIO":"4.1MB / 0B","CPUPerc":"0.02%","Container":"abc","ID":"0123456789ab","MemPerc":"0.5%","MemUsage":"38.2MiB / 7.6GiB","Name":"db","NetIO":"1.2kB / 860B","PIDs":"7"}"#;
        let s: StatsLine = serde_json::from_str(stats).unwrap();
        assert_eq!(parse_pair_sum(&s.net) + parse_pair_sum(&s.block), 1200 + 860 + 4_100_000);
        let df = r#"{"Active":"0","Reclaimable":"12.3GB (100%)","Size":"12.3GB","TotalCount":"40","Type":"Build Cache"}"#;
        let d: DfLine = serde_json::from_str(df).unwrap();
        assert_eq!(parse_size(d.reclaimable.split('(').next().unwrap()), 12_300_000_000);
    }
}
