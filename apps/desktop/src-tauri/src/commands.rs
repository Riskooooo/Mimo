//! Window-control and settings commands local to the shell itself (not
//! `mimo-core` engine logic, so they live here rather than in
//! `mimo-commands`) — these all need Tauri APIs (paths, plugins, window
//! management) that have nothing to do with system analysis/automation.

use std::sync::Mutex;

use mimo_core::Settings;
use tauri::{AppHandle, Emitter, Manager, PhysicalSize, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;

use crate::{PILL_HEIGHT, SETTINGS_PANEL_HEIGHT, WINDOW_MARGIN_Y};

/// Where the settings file lives for this install — `None` if the OS refuses
/// to hand back an app data directory (extremely unlikely, but every caller
/// already has to handle it since a filesystem write can always fail).
pub fn settings_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

#[tauri::command]
pub fn hide_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn get_settings(settings: tauri::State<'_, Mutex<Settings>>) -> Settings {
    *settings.lock().expect("settings mutex poisoned")
}

#[tauri::command]
pub fn set_launch_at_startup(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    enabled: bool,
) -> Result<Settings, String> {
    let autostart = app.autolaunch();
    let result = if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    };
    result.map_err(|err| err.to_string())?;

    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.launch_at_startup = enabled;
    if let Some(path) = settings_path(&app) {
        let _ = guard.save(&path);
    }
    Ok(*guard)
}

/// Wipes everything Mimo has stored locally and resets in-memory state to
/// defaults. Right now that's just the settings file, but this is written as
/// "erase the whole app data directory" so it stays correct as more local
/// data (history, caches, a database) gets added later, instead of needing a
/// new line here every time.
#[tauri::command]
pub fn erase_memory(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
) -> Result<Settings, String> {
    let _ = app.autolaunch().disable();

    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }

    let mut guard = settings.lock().expect("settings mutex poisoned");
    *guard = Settings::default();
    Ok(*guard)
}

/// Grows or shrinks the main window to make room for the settings drawer
/// below the status bar. Called before the drawer's own CSS transition
/// starts (see the frontend), so the extra space already exists — invisible
/// until the drawer animates into it — rather than the window visibly
/// snapping to a new size mid-animation.
#[tauri::command]
pub fn set_settings_panel_open(window: WebviewWindow, open: bool) {
    let Ok(size) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);

    let base_logical_height = PILL_HEIGHT + WINDOW_MARGIN_Y as f64;
    let logical_height = if open {
        base_logical_height + SETTINGS_PANEL_HEIGHT
    } else {
        base_logical_height
    };

    let physical_height = (logical_height * scale).round() as u32;
    let _ = window.set_size(PhysicalSize::new(size.width, physical_height));
}

/// Brings the main pill back and asks it to open its settings drawer —
/// used by the tray's right-click menu, since the pill may currently be
/// hidden (no taskbar entry to click instead).
#[tauri::command]
pub fn open_settings_from_tray(app: AppHandle) {
    if let Some(menu) = app.get_webview_window("tray-menu") {
        let _ = menu.hide();
    }
    if let Some(main) = app.get_webview_window("main") {
        crate::place_island(&main);
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.emit("mimo://reveal", ());
        let _ = main.emit("mimo://open-settings", ());
    }
}
