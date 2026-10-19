//! Watching over time, from the once-a-minute background sample:
//! - leak watch: processes whose memory keeps climbing;
//! - fan culprits: who used the most CPU over the last half hour.
//!
//! Everything is kept small and in memory: a few numbers per heavy process,
//! and the top few CPU users per minute for 30 minutes.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

/// Only processes at least this big are tracked for leaks.
const LEAK_TRACK_MIN: u64 = 200 * MB;
/// A memory checkpoint every 10 minutes, kept for an hour.
const CHECKPOINT_SECS: u64 = 10 * 60;
const CHECKPOINTS: usize = 7;
/// Growth that counts as a leak within the window.
const LEAK_GROWTH: u64 = GB;
/// CPU history for the fan view.
const CPU_MINUTES: usize = 30;
const CPU_TOP_PER_MINUTE: usize = 8;

/// One process at one sample.
#[derive(Debug, Clone)]
pub struct Reading {
    pub pid: u32,
    pub start: u64,
    /// What a person calls it: the app name for app helpers, else the process name.
    pub label: String,
    pub memory: u64,
    pub cpu: f32,
}

#[derive(Debug, Default)]
struct MemTrack {
    label: String,
    /// (seconds since the epoch, bytes), oldest first.
    points: VecDeque<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Growth {
    pub bytes: u64,
    pub secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Leak {
    pub pid: u32,
    pub start: u64,
    pub label: String,
    pub growth: Growth,
    pub memory: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Culprit {
    pub label: String,
    /// Average CPU (% of one core) over the minutes in the window.
    pub avg: f32,
    pub peak: f32,
    /// One value per minute, oldest first (0 where it wasn't in the top list).
    pub series: Vec<f32>,
}

#[derive(Debug, Default)]
pub struct Trends {
    mem: HashMap<(u32, u64), MemTrack>,
    /// Per minute: top CPU users by label.
    minutes: VecDeque<Vec<(String, f32)>>,
}

/// Growth across the checkpoints, if it looks like a steady climb: at least
/// three checkpoints (20 minutes), never dropping more than 10% along the way,
/// and up by at least `LEAK_GROWTH` from the lowest point.
pub fn steady_growth(points: &VecDeque<(u64, u64)>) -> Option<Growth> {
    if points.len() < 3 {
        return None;
    }
    let mut prev = points[0].1;
    for &(_, b) in points.iter().skip(1) {
        if b < prev - prev / 10 {
            return None;
        }
        prev = prev.max(b);
    }
    let (t0, b0) = points.iter().copied().min_by_key(|&(_, b)| b)?;
    let (t1, b1) = *points.back()?;
    let grew = b1.saturating_sub(b0);
    (grew >= LEAK_GROWTH && t1 > t0).then_some(Growth { bytes: grew, secs: t1 - t0 })
}

impl Trends {
    /// Feed one background sample (about once a minute).
    pub fn record(&mut self, now: u64, readings: &[Reading]) {
        // Leak watch: a checkpoint per tracked process every 10 minutes.
        let mut alive = std::collections::HashSet::with_capacity(readings.len());
        for r in readings {
            let key = (r.pid, r.start);
            alive.insert(key);
            let tracked = self.mem.contains_key(&key);
            if !tracked && r.memory < LEAK_TRACK_MIN {
                continue;
            }
            let t = self.mem.entry(key).or_insert_with(|| MemTrack { label: r.label.clone(), points: VecDeque::new() });
            if t.points.back().is_none_or(|&(at, _)| now.saturating_sub(at) >= CHECKPOINT_SECS) {
                t.points.push_back((now, r.memory));
                while t.points.len() > CHECKPOINTS {
                    t.points.pop_front();
                }
            }
        }
        self.mem.retain(|k, _| alive.contains(k));

        // Fan culprits: sum CPU per label for this minute, keep the top few.
        let mut by_label: HashMap<&str, f32> = HashMap::new();
        for r in readings {
            if r.cpu >= 1.0 {
                *by_label.entry(r.label.as_str()).or_default() += r.cpu;
            }
        }
        let mut top: Vec<(String, f32)> = by_label.into_iter().map(|(l, c)| (l.to_string(), c)).collect();
        top.sort_by(|a, b| b.1.total_cmp(&a.1));
        top.truncate(CPU_TOP_PER_MINUTE);
        self.minutes.push_back(top);
        while self.minutes.len() > CPU_MINUTES {
            self.minutes.pop_front();
        }
    }

    pub fn growth_of(&self, pid: u32, start: u64) -> Option<Growth> {
        self.mem.get(&(pid, start)).and_then(|t| steady_growth(&t.points))
    }

    pub fn leaks(&self) -> Vec<Leak> {
        let mut out: Vec<Leak> = self
            .mem
            .iter()
            .filter_map(|(&(pid, start), t)| {
                let growth = steady_growth(&t.points)?;
                Some(Leak { pid, start, label: t.label.clone(), memory: t.points.back()?.1, growth })
            })
            .collect();
        out.sort_by_key(|l| std::cmp::Reverse(l.growth.bytes));
        out
    }

    /// Who worked the CPU hardest over the window, biggest first.
    pub fn culprits(&self, limit: usize) -> (Vec<Culprit>, usize) {
        let n = self.minutes.len();
        let mut series: HashMap<&str, Vec<f32>> = HashMap::new();
        for (i, minute) in self.minutes.iter().enumerate() {
            for (label, cpu) in minute {
                series.entry(label.as_str()).or_insert_with(|| vec![0.0; n])[i] = *cpu;
            }
        }
        let mut out: Vec<Culprit> = series
            .into_iter()
            .map(|(label, s)| Culprit {
                label: label.to_string(),
                avg: s.iter().sum::<f32>() / n.max(1) as f32,
                peak: s.iter().copied().fold(0.0, f32::max),
                series: s,
            })
            .filter(|c| c.avg >= 1.0)
            .collect();
        out.sort_by(|a, b| b.avg.total_cmp(&a.avg));
        out.truncate(limit);
        (out, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(pid: u32, label: &str, memory: u64, cpu: f32) -> Reading {
        Reading { pid, start: 1, label: label.into(), memory, cpu }
    }

    #[test]
    fn steady_climb_is_a_leak_and_a_sawtooth_is_not() {
        let mut t = Trends::default();
        for i in 0..7u64 {
            let at = i * CHECKPOINT_SECS;
            t.record(at, &[r(1, "node", 600 * MB + i * 300 * MB, 5.0), r(2, "Chrome", if i % 2 == 0 { 3 * GB } else { GB }, 2.0)]);
        }
        let leaks = t.leaks();
        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].label, "node");
        assert_eq!(leaks[0].growth.bytes, 1800 * MB);
        assert_eq!(leaks[0].growth.secs, 6 * CHECKPOINT_SECS);
        assert!(t.growth_of(2, 1).is_none(), "drops back down: not a leak");
    }

    #[test]
    fn small_or_young_processes_are_not_leaks() {
        let mut t = Trends::default();
        // Too small to track.
        for i in 0..7u64 {
            t.record(i * CHECKPOINT_SECS, &[r(1, "tiny", 50 * MB, 0.0)]);
        }
        assert!(t.leaks().is_empty());
        // Big growth but only two checkpoints so far.
        let mut t = Trends::default();
        t.record(0, &[r(3, "big", 300 * MB, 0.0)]);
        t.record(CHECKPOINT_SECS, &[r(3, "big", 3 * GB, 0.0)]);
        assert!(t.leaks().is_empty());
    }

    #[test]
    fn checkpoints_only_every_ten_minutes_and_dead_processes_drop() {
        let mut t = Trends::default();
        for s in 0..30u64 {
            t.record(s * 60, &[r(1, "node", 500 * MB, 0.0)]);
        }
        assert_eq!(t.mem[&(1, 1)].points.len(), 3); // at 0, 10 and 20 min
        t.record(31 * 60, &[]);
        assert!(t.mem.is_empty());
    }

    #[test]
    fn culprits_average_over_the_window() {
        let mut t = Trends::default();
        for i in 0..10 {
            let mut rs = vec![r(1, "Google Chrome", 0, 40.0), r(2, "Google Chrome", 0, 20.0)];
            if i < 5 {
                rs.push(r(3, "node", 0, 100.0));
            }
            rs.push(r(4, "idle", 0, 0.2));
            t.record(i * 60, &rs);
        }
        let (c, minutes) = t.culprits(5);
        assert_eq!(minutes, 10);
        assert_eq!(c[0].label, "Google Chrome");
        assert!((c[0].avg - 60.0).abs() < 0.01, "helpers add up under one name");
        assert_eq!(c[1].label, "node");
        assert!((c[1].avg - 50.0).abs() < 0.01);
        assert_eq!(c[1].peak, 100.0);
        assert_eq!(c[1].series.len(), 10);
        assert!(c.iter().all(|x| x.label != "idle"));
    }

    #[test]
    fn cpu_window_is_thirty_minutes() {
        let mut t = Trends::default();
        for i in 0..45 {
            t.record(i * 60, &[r(1, "x", 0, 10.0)]);
        }
        assert_eq!(t.culprits(3).1, CPU_MINUTES);
    }
}
