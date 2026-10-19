//! A tiny local diary for the smoothness score: per day, how often the
//! machine was smooth (sampled once a minute) and how much the user freed.
//! Kept for 14 days in history.json next to settings. Never leaves the machine.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::alerts::Level;

const KEEP_DAYS: usize = 14;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// Days since the Unix epoch, in local-agnostic UTC days.
    pub day: u64,
    pub smooth: u32,
    pub busy: u32,
    pub strained: u32,
    pub freed: u64,
    pub closed: u32,
}

impl Day {
    fn samples(&self) -> u32 {
        self.smooth + self.busy + self.strained
    }

    /// 0-100: smooth minutes count fully, busy half, strained not at all.
    pub fn score(&self) -> Option<u8> {
        let n = self.samples();
        if n == 0 {
            return None;
        }
        let pts = self.smooth as f64 + self.busy as f64 * 0.5;
        Some(((pts / n as f64) * 100.0).round() as u8)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub days: Vec<Day>,
}

pub fn today() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

impl History {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("history.json")
    }

    pub fn load(path: &Path) -> History {
        std::fs::read(path)
            .ok()
            .filter(|b| b.len() < 256 * 1024)
            .and_then(|b| serde_json::from_slice::<History>(&b).ok())
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

    fn day_mut(&mut self, day: u64) -> &mut Day {
        if self.days.last().is_none_or(|d| d.day != day) {
            self.days.push(Day { day, ..Default::default() });
            self.days.sort_by_key(|d| d.day);
            self.days.dedup_by_key(|d| d.day);
            let n = self.days.len();
            if n > KEEP_DAYS {
                self.days.drain(..n - KEEP_DAYS);
            }
        }
        self.days.iter_mut().find(|d| d.day == day).expect("just inserted")
    }

    pub fn record_level(&mut self, day: u64, level: Level) {
        let d = self.day_mut(day);
        match level {
            Level::Smooth => d.smooth += 1,
            Level::Busy => d.busy += 1,
            Level::Strained => d.strained += 1,
        }
    }

    pub fn record_freed(&mut self, day: u64, bytes: u64) {
        let d = self.day_mut(day);
        d.freed = d.freed.saturating_add(bytes);
        d.closed += 1;
    }

    pub fn summary(&self, day: u64) -> Summary {
        let week: Vec<u64> = (0..7).rev().map(|ago| self.freed_on(day.saturating_sub(ago))).collect();
        Summary {
            score: self.days.iter().find(|d| d.day == day).and_then(Day::score),
            freed_today: self.freed_on(day),
            freed_week: week.iter().sum(),
            week,
        }
    }

    fn freed_on(&self, day: u64) -> u64 {
        self.days.iter().find(|d| d.day == day).map(|d| d.freed).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Today's "how buttery" score, once there's at least one sample.
    pub score: Option<u8>,
    pub freed_today: u64,
    pub freed_week: u64,
    /// Freed per day, oldest first, ending today (7 entries).
    pub week: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_weights_busy_half() {
        let mut h = History::default();
        for _ in 0..6 {
            h.record_level(100, Level::Smooth);
        }
        for _ in 0..2 {
            h.record_level(100, Level::Busy);
        }
        for _ in 0..2 {
            h.record_level(100, Level::Strained);
        }
        assert_eq!(h.summary(100).score, Some(70));
        assert_eq!(h.summary(101).score, None);
    }

    #[test]
    fn week_and_retention() {
        let mut h = History::default();
        for day in 80..100 {
            h.record_freed(day, 1000);
        }
        assert_eq!(h.days.len(), KEEP_DAYS);
        assert_eq!(h.days.first().unwrap().day, 86);
        let s = h.summary(99);
        assert_eq!(s.week, vec![1000; 7]);
        assert_eq!(s.freed_week, 7000);
        assert_eq!(s.freed_today, 1000);
        let gap = h.summary(110);
        assert_eq!(gap.freed_week, 0);
    }

    #[test]
    fn round_trip_and_bad_file() {
        let dir = std::env::temp_dir().join(format!("butters-hist-{}", std::process::id()));
        let path = History::path(&dir);
        let mut h = History::default();
        h.record_freed(5, 42);
        h.save(&path).unwrap();
        assert_eq!(History::load(&path), h);
        std::fs::write(&path, b"nope").unwrap();
        assert_eq!(History::load(&path), History::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
