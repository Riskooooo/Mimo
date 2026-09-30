//! Core engine for Mimo.
//!
//! This crate owns all system-analysis and automation logic. It has no
//! dependency on Tauri (or any other UI toolkit), so it can be unit-tested,
//! reused, and evolved independently of the desktop shell in `apps/desktop`.
//! The shell only ever talks to this crate through `mimo-commands`.

pub mod engine;
pub mod error;
pub mod settings;

pub use engine::{Engine, EngineStatus};
pub use error::CoreError;
pub use settings::Settings;
