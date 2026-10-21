//! Butters' own log: a plain text file of what it saw and did, so you can
//! check later how it behaved on a real machine. One event per line, UTC
//! timestamps:
//!
//!   2026-10-21T18:23:05Z INFO  Closed "Container: redis" (docker_container): Stopped redis
//!
//! macOS: ~/Library/Logs/com.kenkai.butters/butters.log
//! Windows: %LOCALAPPDATA%\com.kenkai.butters\logs\butters.log
//!
//! It never grows past about 2 MB: at 1 MB the file moves to butters.log.1
//! (replacing the older one) and a fresh file starts. Only names, reasons
//! and sizes go in; never command lines, arguments or environment.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;
const FILE_NAME: &str = "butters.log";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

/// Cheap to clone; every clone writes to the same file.
#[derive(Clone)]
pub struct Log {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    path: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    size: u64,
}

impl Log {
    /// Logs into `dir/butters.log`. If the folder can't be created or the
    /// file can't be opened, logging quietly does nothing: it must never
    /// stop Butters from working.
    pub fn open(dir: &Path) -> Log {
        Log::with_limit(dir, MAX_BYTES)
    }

    fn with_limit(dir: &Path, max_bytes: u64) -> Log {
        let _ = std::fs::create_dir_all(dir);
        let mut inner = Inner { path: dir.join(FILE_NAME), max_bytes, file: None, size: 0 };
        inner.reopen();
        Log { inner: Arc::new(Mutex::new(inner)) }
    }

    /// A log that writes nowhere (when there's no folder to put it in).
    pub fn disabled() -> Log {
        Log { inner: Arc::new(Mutex::new(Inner { path: PathBuf::new(), max_bytes: MAX_BYTES, file: None, size: 0 })) }
    }

    pub fn path(&self) -> PathBuf {
        self.lock().path.clone()
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        self.write(Level::Info, msg.as_ref());
    }

    pub fn warn(&self, msg: impl AsRef<str>) {
        self.write(Level::Warn, msg.as_ref());
    }

    pub fn error(&self, msg: impl AsRef<str>) {
        self.write(Level::Error, msg.as_ref());
    }

    pub fn write(&self, level: Level, msg: &str) {
        let line = format_line(SystemTime::now(), level, msg);
        #[cfg(debug_assertions)]
        eprint!("{line}");
        self.lock().append(&line);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Inner {
    fn reopen(&mut self) {
        self.file = OpenOptions::new().create(true).append(true).open(&self.path).ok();
        self.size = self.file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
    }

    fn append(&mut self, line: &str) {
        if self.file.is_none() {
            return;
        }
        if self.size > 0 && self.size + line.len() as u64 > self.max_bytes {
            self.file = None;
            let _ = std::fs::rename(&self.path, rotated(&self.path));
            self.reopen();
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line.as_bytes()).is_ok()
        {
            self.size += line.len() as u64;
        }
    }
}

fn rotated(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".1");
    PathBuf::from(p)
}

/// One finished line. Line breaks and other control characters in `msg`
/// become spaces, so a strange process name can't fake extra entries.
fn format_line(at: SystemTime, level: Level, msg: &str) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let clean: String = msg.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!("{} {} {}\n", utc_stamp(secs), level.tag(), clean.trim_end())
}

/// Seconds since 1970 as `YYYY-MM-DDTHH:MM:SSZ`.
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("butters-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(utc_stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_stamp(1_792_606_985), "2026-10-21T18:23:05Z");
    }

    #[test]
    fn one_event_is_one_line() {
        let at = UNIX_EPOCH + Duration::from_secs(60);
        let line = format_line(at, Level::Warn, "evil\nname\r\t2026-01-01 INFO fake\n");
        assert_eq!(line, "1970-01-01T00:01:00Z WARN  evil name  2026-01-01 INFO fake\n");
        assert_eq!(line.matches('\n').count(), 1);
    }

    #[test]
    fn appends_and_rotates_at_the_limit() {
        let dir = temp_dir("rotate");
        let log = Log::with_limit(&dir, 150);
        log.info("first entry, about forty bytes");
        log.info("second entry, about forty bytes");
        let main = dir.join(FILE_NAME);
        let before = std::fs::read_to_string(&main).unwrap();
        assert_eq!(before.lines().count(), 2);

        log.error("third entry pushes it over the limit");
        let old = std::fs::read_to_string(rotated(&main)).unwrap();
        let now = std::fs::read_to_string(&main).unwrap();
        assert_eq!(old, before, "old lines moved to butters.log.1");
        assert!(now.contains("ERROR third entry") && now.lines().count() == 1);

        // A new Log on the same folder keeps appending, never truncates.
        Log::with_limit(&dir, 150).info("after restart");
        assert_eq!(std::fs::read_to_string(&main).unwrap().lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwritable_folder_is_harmless() {
        let log = Log::disabled();
        log.info("goes nowhere");
        let file = temp_dir("blocked");
        std::fs::write(&file, b"not a folder").unwrap();
        Log::open(&file).error("still fine");
        let _ = std::fs::remove_file(&file);
    }
}
