//! The user's own commands ("Mes commandes", opened from the settings):
//! persisted as `custom_commands.json` in the app data folder and handed to
//! the engine, which checks them before anything else. The rules live in
//! `mimo_core::custom`; this owns the file, the window and its commands.

use std::path::PathBuf;
use std::sync::Mutex;

use mimo_core::custom::{validate, CustomCommand, CustomStore};
use mimo_core::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::voice::VoiceWake;

pub struct CustomCommands {
    store: Mutex<CustomStore>,
    path: Option<PathBuf>,
}

impl CustomCommands {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_data_dir().ok().map(|dir| dir.join("custom_commands.json"));
        let store: CustomStore = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        set_engine_commands(app, store.commands.clone());
        Self { store: Mutex::new(store), path }
    }

    /// Erase memory: no commands left.
    pub fn forget(&self, app: &AppHandle) {
        *self.lock() = CustomStore::default();
        set_engine_commands(app, Vec::new());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CustomStore> {
        self.store.lock().expect("custom commands mutex poisoned")
    }

    fn mutate(&self, app: &AppHandle, change: impl FnOnce(&mut CustomStore)) -> Vec<CustomCommand> {
        let mut store = self.lock();
        change(&mut store);
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*store) {
                let _ = std::fs::write(path, json);
            }
        }
        set_engine_commands(app, store.commands.clone());
        // Their phrases are part of the voice grammar.
        app.state::<VoiceWake>().refresh(app);
        store.commands.clone()
    }
}

fn set_engine_commands(app: &AppHandle, commands: Vec<CustomCommand>) {
    app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").set_custom_commands(commands);
}

#[tauri::command]
pub fn list_custom_commands(commands: tauri::State<'_, CustomCommands>) -> Vec<CustomCommand> {
    commands.lock().commands.clone()
}

/// Adds (id 0) or updates a command; the error is worded for the user.
#[tauri::command]
pub fn save_custom_command(
    app: AppHandle,
    commands: tauri::State<'_, CustomCommands>,
    command: CustomCommand,
) -> Result<Vec<CustomCommand>, String> {
    let lang = app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").language();
    let existing = commands.lock().commands.clone();
    let command = validate(command, &existing, lang)?;
    Ok(commands.mutate(&app, |store| {
        store.save(command);
    }))
}

#[tauri::command]
pub fn delete_custom_command(app: AppHandle, commands: tauri::State<'_, CustomCommands>, id: u64) -> Vec<CustomCommand> {
    commands.mutate(&app, |store| store.remove(id))
}

#[derive(Serialize)]
pub struct AppChoice {
    name: String,
    app_id: String,
}

/// Start menu apps, by name, for the "open an app" action.
#[tauri::command]
pub fn list_installed_apps(engine: tauri::State<'_, Mutex<Engine>>) -> Vec<AppChoice> {
    let engine = engine.lock().expect("engine mutex poisoned");
    let mut apps: Vec<AppChoice> = engine
        .installed_apps()
        .apps()
        .iter()
        .map(|a| AppChoice { name: a.name.clone(), app_id: a.id.clone() })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// Shows the "Mes commandes" window, centered.
#[tauri::command]
pub fn open_commands_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("commands") {
        if !window.is_visible().unwrap_or(false) {
            let _ = window.center();
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn close_commands_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("commands") {
        let _ = window.hide();
    }
}
