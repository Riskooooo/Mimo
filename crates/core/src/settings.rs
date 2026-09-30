use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// User-configurable preferences, persisted as JSON in the app's local data
/// directory. Kept as a plain, `serde`-only data type (no Tauri dependency)
/// so it can be loaded/saved from any context that can resolve a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Settings {
    pub launch_at_startup: bool,
}

impl Settings {
    /// Loads settings from `path`, falling back to defaults if the file is
    /// missing, unreadable, or corrupt — a fresh install (or a wiped one)
    /// should never fail to start.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, json)
    }
}
