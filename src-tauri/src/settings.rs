use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Notify when memory is tight, the machine is hot, or the CPU stays pegged.
    pub alerts: bool,
    pub launch_at_login: bool,
    /// Soft sound effects in sync with the animations.
    pub sounds: bool,
    /// Groups the user asked Butters to leave alone.
    pub ignored: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { alerts: true, launch_at_login: false, sounds: true, ignored: Vec::new() }
    }
}

const MAX_IGNORED: usize = 500;

impl Settings {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("settings.json")
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .filter(|b| b.len() < 1024 * 1024)
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    /// Writes to a temp file and renames it, so a crash can't leave half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn sanitized(mut self) -> Settings {
        self.ignored.retain(|k| !k.is_empty() && k.len() <= 1024);
        self.ignored.dedup();
        self.ignored.truncate(MAX_IGNORED);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("butters-test-{}", std::process::id()));
        let path = Settings::path(&dir);
        assert_eq!(Settings::load(&path), Settings::default());
        let s = Settings { alerts: false, launch_at_login: true, sounds: false, ignored: vec!["app:/x".into()] };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        // Missing fields fall back to defaults.
        std::fs::write(&path, br#"{"alerts": false}"#).unwrap();
        assert_eq!(Settings::load(&path), Settings { alerts: false, ..Settings::default() });
        let _ = std::fs::remove_dir_all(dir);
    }
}
