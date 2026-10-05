use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::shortcut::DEFAULT_SUMMON_SHORTCUT;

/// Languages Mimo is available in — for its text and replies, and the
/// speech model it listens with (`resources/vosk/models/<code>`).
/// The first one is the default.
pub const LANGUAGES: &[&str] = &["en", "fr"];

/// User-configurable preferences, persisted as JSON in the app's local data
/// directory. Kept as a plain, `serde`-only data type (no Tauri dependency)
/// so it can be loaded/saved from any context that can resolve a path.
///
/// `#[serde(default)]` lets a settings file written by an older version
/// (missing newer fields) still load, with the new fields at their defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub launch_at_startup: bool,
    /// Global accelerator that brings the pill up in typing mode.
    pub summon_shortcut: String,
    /// Whether Mimo listens for "Hey Mimo" in the background.
    pub voice_wake_enabled: bool,
    /// The app's language, one of [`LANGUAGES`]: interface, replies and
    /// the speech model all follow it. (Was `voice_language` before it
    /// covered more than voice; older settings files still load.)
    #[serde(alias = "voice_language")]
    pub language: String,
    /// Short UI sounds (wake chime, success/error) on or off.
    pub sounds_enabled: bool,
    /// Learn which apps are used when (stored locally, can be turned off
    /// in settings).
    pub activity_enabled: bool,
    /// Let Mimo suggest things on its own (needs `activity_enabled`).
    pub suggestions_enabled: bool,
    /// The local AI (downloaded when first turned on).
    pub ai_enabled: bool,
    /// Mimo's main color ("#rrggbb"), in every window.
    pub accent_color: String,
    /// Tints the glass of Mimo's windows with that color.
    pub tinted_glass: bool,
    /// Mimo's little face in the pill (otherwise, the status dot).
    pub character_enabled: bool,
}

pub const DEFAULT_ACCENT_COLOR: &str = "#0a84ff";

/// A color as "#rrggbb".
pub fn validate_accent_color(color: &str) -> Result<(), CoreError> {
    let hex = color.strip_prefix('#').unwrap_or_default();
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(CoreError::InvalidColor(color.to_string()))
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            launch_at_startup: false,
            summon_shortcut: DEFAULT_SUMMON_SHORTCUT.to_string(),
            voice_wake_enabled: true,
            language: LANGUAGES[0].to_string(),
            sounds_enabled: true,
            activity_enabled: true,
            suggestions_enabled: true,
            ai_enabled: false,
            accent_color: DEFAULT_ACCENT_COLOR.to_string(),
            tinted_glass: false,
            character_enabled: true,
        }
    }
}

pub fn validate_language(language: &str) -> Result<(), CoreError> {
    if LANGUAGES.contains(&language) {
        Ok(())
    } else {
        Err(CoreError::UnsupportedLanguage(language.to_string()))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_colors_are_checked() {
        assert!(validate_accent_color("#0a84ff").is_ok());
        assert!(validate_accent_color("#FF375F").is_ok());
        for bad in ["0a84ff", "#0a84f", "#0a84fg", "red", "#0a84ff; x", ""] {
            assert!(validate_accent_color(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn older_settings_files_still_load() {
        let settings: Settings = serde_json::from_str(r#"{ "launch_at_startup": true }"#).unwrap();
        assert!(settings.launch_at_startup);
        assert_eq!(settings.summon_shortcut, DEFAULT_SUMMON_SHORTCUT);
        assert!(settings.voice_wake_enabled);
        assert!(settings.activity_enabled);
        assert!(settings.suggestions_enabled);
        assert!(!settings.ai_enabled);
        assert_eq!(settings.accent_color, DEFAULT_ACCENT_COLOR);
        assert!(!settings.tinted_glass);
        assert!(settings.character_enabled);
        assert_eq!(settings.language, "en");
        assert!(settings.sounds_enabled);
    }

    #[test]
    fn only_shipped_languages_are_valid() {
        assert!(validate_language("fr").is_ok());
        assert!(validate_language("en").is_ok());
        assert!(validate_language("de").is_err());
    }

    #[test]
    fn language_saved_under_its_old_name_still_loads() {
        let settings: Settings = serde_json::from_str(r#"{ "voice_language": "fr" }"#).unwrap();
        assert_eq!(settings.language, "fr");
    }
}
