//! When each Docker container last did anything. Docker has no "last used"
//! field, so Butters works it out: a container is active when its traffic
//! (network + disk bytes) moves or it uses CPU between two looks. Looks happen
//! every 10 minutes in the background and whenever the panel opens; the
//! result is kept in docker-activity.json so it survives restarts.
//!
//! On the very first look, a container that has moved almost nothing since
//! it started is counted as quiet since it started; otherwise the clock
//! starts at that first look (and the reason says "at least").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::services::Container;

/// Traffic below this since start means "never really used".
const QUIET_SINCE_START: u64 = 8 * 1000 * 1000;
/// Traffic between two looks that counts as activity.
const ACTIVE_DELTA: u64 = 256 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub io: u64,
    pub last_active: u64,
    /// True when `last_active` is only when Butters first looked.
    pub lower_bound: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Activity {
    pub containers: HashMap<String, Seen>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Idle {
    pub secs: u64,
    pub lower_bound: bool,
}

impl Activity {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("docker-activity.json")
    }

    pub fn load(path: &Path) -> Activity {
        std::fs::read(path)
            .ok()
            .filter(|b| b.len() < 512 * 1024)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    /// Records a look at the running containers and forgets ones that are gone.
    pub fn observe(&mut self, containers: &[Container], now: u64) {
        let mut keep = std::collections::HashSet::new();
        for c in containers {
            keep.insert(c.id.clone());
            let next = match self.containers.get(&c.id) {
                None => {
                    let quiet = c.cpu < 1.0 && c.io < QUIET_SINCE_START;
                    match c.started_at {
                        Some(started) if quiet && started <= now => Seen { io: c.io, last_active: started, lower_bound: false },
                        _ => Seen { io: c.io, last_active: now, lower_bound: true },
                    }
                }
                Some(prev) => {
                    // Counters reset when a container restarts (io goes down).
                    let active = c.cpu >= 1.0 || c.io < prev.io || c.io - prev.io >= ACTIVE_DELTA;
                    if active {
                        Seen { io: c.io, last_active: now, lower_bound: false }
                    } else {
                        Seen { io: c.io, ..prev.clone() }
                    }
                }
            };
            self.containers.insert(c.id.clone(), next);
        }
        self.containers.retain(|id, _| keep.contains(id));
    }

    pub fn idle(&self, id: &str, now: u64) -> Option<Idle> {
        self.containers.get(id).map(|s| Idle { secs: now.saturating_sub(s.last_active), lower_bound: s.lower_bound })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    fn c(id: &str, io: u64, cpu: f32, started: u64) -> Container {
        Container { id: id.into(), name: id.into(), io, cpu, started_at: Some(started), ..Default::default() }
    }

    #[test]
    fn untouched_since_start_counts_from_start() {
        let mut a = Activity::default();
        let now = 100 * DAY;
        a.observe(&[c("db", 2_000_000, 0.1, now - 6 * DAY), c("api", 900_000_000, 0.2, now - 6 * DAY)], now);
        assert_eq!(a.idle("db", now), Some(Idle { secs: 6 * DAY, lower_bound: false }));
        // Lots of traffic at some point: we only know it's quiet since now.
        assert_eq!(a.idle("api", now), Some(Idle { secs: 0, lower_bound: true }));
    }

    #[test]
    fn traffic_or_cpu_resets_the_clock() {
        let mut a = Activity::default();
        let t0 = 50 * DAY;
        a.observe(&[c("x", 50_000_000, 0.0, t0 - DAY)], t0);
        a.observe(&[c("x", 50_000_100, 0.0, t0 - DAY)], t0 + 600); // a trickle: still idle
        assert_eq!(a.idle("x", t0 + 2 * DAY).unwrap().secs, 2 * DAY);
        a.observe(&[c("x", 51_000_000, 0.0, t0 - DAY)], t0 + 2 * DAY); // real traffic
        assert_eq!(a.idle("x", t0 + 2 * DAY), Some(Idle { secs: 0, lower_bound: false }));
        a.observe(&[c("x", 51_000_000, 4.0, t0 - DAY)], t0 + 3 * DAY); // busy CPU
        assert_eq!(a.idle("x", t0 + 3 * DAY).unwrap().secs, 0);
        a.observe(&[c("x", 10, 0.0, t0 + 3 * DAY)], t0 + 4 * DAY); // restarted: counters reset
        assert_eq!(a.idle("x", t0 + 4 * DAY).unwrap().secs, 0);
    }

    #[test]
    fn gone_containers_are_forgotten_and_file_round_trips() {
        let mut a = Activity::default();
        a.observe(&[c("a", 0, 0.0, 10), c("b", 0, 0.0, 10)], 100);
        a.observe(&[c("a", 0, 0.0, 10)], 200);
        assert!(a.idle("b", 200).is_none());
        let dir = std::env::temp_dir().join(format!("butters-dock-{}", std::process::id()));
        let path = Activity::path(&dir);
        a.save(&path).unwrap();
        assert_eq!(Activity::load(&path), a);
        let _ = std::fs::remove_dir_all(dir);
    }
}
