//! Core engine for Mimo.
//!
//! This crate owns all system-analysis and automation logic. It has no
//! dependency on Tauri (or any other UI toolkit), so it can be unit-tested,
//! reused, and evolved independently of the desktop shell in `apps/desktop`.
//! The shell only ever talks to this crate through `mimo-commands`.

pub mod activity;
pub mod apps;
pub mod engine;
pub mod error;
pub mod info;
pub mod intent;
pub mod notifications;
pub mod reminders;
pub mod settings;
pub mod shortcut;
pub mod suggest;
pub mod system;
pub mod tasks;
pub mod translate;

pub use engine::{Engine, EngineStatus};
pub use error::CoreError;
pub use apps::{AppCatalog, InstalledApp};
pub use info::{Day, Lang, Question, WeatherReport};
pub use intent::{voice_phrases, Intent};
pub use settings::{validate_language, Settings, LANGUAGES};
pub use shortcut::{validate_shortcut, DEFAULT_SUMMON_SHORTCUT};
