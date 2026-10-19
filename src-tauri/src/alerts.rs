//! Gentle alerts: tell the user when the machine is struggling, at most once
//! per half hour per kind. Never acts on its own.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::platform::{Pressure, Thermal};
use crate::sampler::Stats;

const COOLDOWN: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Memory,
    Heat,
    Cpu,
}

#[derive(Default)]
pub struct AlertState {
    mem_streak: u32,
    cpu_streak: u32,
    last: HashMap<AlertKind, Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Smooth,
    Busy,
    Strained,
}

fn mem_ratio(s: &Stats) -> f64 {
    if s.mem_total == 0 { 0.0 } else { s.mem_used as f64 / s.mem_total as f64 }
}

pub fn level(s: &Stats) -> Level {
    let ratio = mem_ratio(s);
    let mem_bad = s.pressure == Pressure::Critical || (s.pressure == Pressure::Unknown && ratio >= 0.92);
    let mem_warn = s.pressure == Pressure::Warn || (s.pressure == Pressure::Unknown && ratio >= 0.85);
    if mem_bad || matches!(s.thermal, Thermal::Serious | Thermal::Critical) || s.cpu >= 85.0 {
        Level::Strained
    } else if mem_warn || s.thermal == Thermal::Fair || s.cpu >= 60.0 {
        Level::Busy
    } else {
        Level::Smooth
    }
}

impl AlertState {
    /// Call once per background sample (about a minute apart).
    pub fn check(&mut self, s: &Stats, now: Instant) -> Option<(AlertKind, &'static str)> {
        let ratio = mem_ratio(s);
        let mem_now = match s.pressure {
            Pressure::Critical => 2,
            Pressure::Warn => 1,
            Pressure::Unknown if ratio >= 0.92 => 1,
            _ => 0,
        };
        self.mem_streak = if mem_now > 0 { self.mem_streak + mem_now } else { 0 };
        self.cpu_streak = if s.cpu >= 85.0 { self.cpu_streak + 1 } else { 0 };

        let candidates = [
            (
                self.mem_streak >= 2,
                AlertKind::Memory,
                "Memory is low. Open Butters.",
            ),
            (
                matches!(s.thermal, Thermal::Serious | Thermal::Critical),
                AlertKind::Heat,
                "Running hot. Open Butters.",
            ),
            (
                self.cpu_streak >= 3,
                AlertKind::Cpu,
                "CPU maxed out. Open Butters.",
            ),
        ];
        for (hit, kind, msg) in candidates {
            if hit && self.last.get(&kind).is_none_or(|t| now.duration_since(*t) >= COOLDOWN) {
                self.last.insert(kind, now);
                return Some((kind, msg));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(cpu: f32, pressure: Pressure, thermal: Thermal) -> Stats {
        Stats { mem_used: 8, mem_total: 16, swap_used: 0, cpu, cores: 8, pressure, thermal }
    }

    #[test]
    fn calm_machine_never_alerts() {
        let mut a = AlertState::default();
        let t = Instant::now();
        for i in 0..10 {
            assert!(a.check(&stats(20.0, Pressure::Normal, Thermal::Nominal), t + Duration::from_secs(60 * i)).is_none());
        }
        assert_eq!(level(&stats(20.0, Pressure::Normal, Thermal::Nominal)), Level::Smooth);
    }

    #[test]
    fn memory_warn_needs_two_samples_and_respects_cooldown() {
        let mut a = AlertState::default();
        let t = Instant::now();
        let warn = stats(20.0, Pressure::Warn, Thermal::Nominal);
        assert!(a.check(&warn, t).is_none());
        assert_eq!(a.check(&warn, t + Duration::from_secs(60)).unwrap().0, AlertKind::Memory);
        assert!(a.check(&warn, t + Duration::from_secs(120)).is_none(), "cooldown");
        assert!(a.check(&warn, t + Duration::from_secs(31 * 60)).is_some(), "after cooldown");
    }

    #[test]
    fn critical_memory_alerts_at_once() {
        let mut a = AlertState::default();
        assert_eq!(a.check(&stats(20.0, Pressure::Critical, Thermal::Nominal), Instant::now()).unwrap().0, AlertKind::Memory);
    }

    #[test]
    fn sustained_cpu_alerts_after_three_samples() {
        let mut a = AlertState::default();
        let t = Instant::now();
        let hot = stats(95.0, Pressure::Normal, Thermal::Nominal);
        assert!(a.check(&hot, t).is_none());
        assert!(a.check(&hot, t + Duration::from_secs(60)).is_none());
        assert_eq!(a.check(&hot, t + Duration::from_secs(120)).unwrap().0, AlertKind::Cpu);
        assert_eq!(level(&hot), Level::Strained);
    }

    #[test]
    fn windows_falls_back_to_used_ratio() {
        let mut s = stats(10.0, Pressure::Unknown, Thermal::Unknown);
        s.mem_used = 15;
        s.mem_total = 16;
        assert_eq!(level(&s), Level::Strained);
    }
}
