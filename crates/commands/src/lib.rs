//! Bridge layer exposing `mimo-core` to the Tauri frontend as commands.
//!
//! Keeping this crate separate from both `mimo-core` and the `desktop`
//! binary means the core engine never depends on Tauri, and the shell never
//! contains business logic directly.
//!
//! Commands live in a submodule rather than at the crate root: Tauri's
//! `#[tauri::command]` macro generates a hidden glue macro at module scope,
//! and a `pub fn` at the crate root collides with it (see
//! https://github.com/tauri-apps/tauri/issues/3198). Nesting one level
//! avoids the collision and matches Tauri's own documented pattern for
//! commands declared outside the app crate's `lib.rs`.

pub mod commands;
mod info;

use std::sync::Mutex;

pub use commands::{engine_status, execute, interpret, AskResponseDto, EngineStatusDto};

/// Builds the shared engine state for the shell to hand to `app.manage(...)`.
///
/// The engine is started once, here, rather than re-created on every command
/// call — commands only ever borrow it through Tauri's managed state, which
/// keeps `mimo-core` as the single, long-lived source of truth for whatever
/// analysis/automation state it grows to hold.
pub fn init_engine_state() -> Mutex<mimo_core::Engine> {
    let mut engine = mimo_core::Engine::new();
    let _ = engine.start();
    Mutex::new(engine)
}
