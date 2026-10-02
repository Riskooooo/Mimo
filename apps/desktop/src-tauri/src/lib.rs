mod apps;
mod commands;
mod diagnostics;
mod notifications;
mod panel;
mod reminders;
mod sounds;
mod speech;
mod tasks;
mod translate;
mod voice;

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::sounds::{Sound, Sounds};
use crate::voice::VoiceWake;

/// Horizontal margin (in CSS px, matched by `.pill.full`'s `calc(100% - …)`)
/// left around the pill inside the window so its blur/shadow isn't clipped.
pub(crate) const WINDOW_MARGIN_X: u32 = 56;
/// Same idea, vertically.
pub(crate) const WINDOW_MARGIN_Y: u32 = 40;

/// Fraction of the screen's width the expanded pill should target.
const PILL_WIDTH_RATIO: f64 = 0.24;
const PILL_MIN_WIDTH: f64 = 360.0;
const PILL_MAX_WIDTH: f64 = 560.0;
pub(crate) const PILL_HEIGHT: f64 = 44.0;

/// Distance from the top of the screen to the top of the window, in logical px.
const TOP_OFFSET: f64 = 14.0;

/// Extra window height (logical px) given to the settings drawer when open —
/// matched by `.settings-panel.open`'s height in the frontend.
pub(crate) const SETTINGS_PANEL_HEIGHT: f64 = 264.0;

/// Gap (physical px) between the tray-menu popup and the tray icon it opened from.
const TRAY_MENU_GAP: i32 = 8;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        summon(app, "shortcut");
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            mimo_commands::engine_status,
            commands::ask_mimo,
            commands::get_panel_content,
            commands::close_panel,
            commands::refresh_diagnostic,
            commands::remove_reminder,
            commands::stop_speaking,
            commands::complete_task,
            commands::add_task_text,
            commands::translate_clipboard,
            commands::cancel_translation,
            commands::copy_text,
            commands::set_pill_drawer,
            commands::hide_window,
            commands::quit_app,
            commands::get_settings,
            commands::set_launch_at_startup,
            commands::erase_memory,
            commands::set_settings_panel_open,
            commands::open_settings_from_tray,
            commands::set_summon_shortcut,
            commands::set_voice_wake_enabled,
            commands::set_language,
            commands::set_sounds_enabled,
            commands::play_sound,
        ])
        .setup(|app| {
            app.manage(mimo_commands::init_engine_state());

            let settings = commands::settings_path(app.handle())
                .map(|path| mimo_core::Settings::load(&path))
                .unwrap_or_default();

            // A shortcut another app already owns shouldn't stop Mimo from
            // starting — it just stays unbound until changed in settings.
            if let Err(err) = app.global_shortcut().register(settings.summon_shortcut.as_str()) {
                eprintln!("could not register summon shortcut {:?}: {err}", settings.summon_shortcut);
            }

            app.state::<Mutex<mimo_core::Engine>>()
                .lock()
                .expect("engine mutex poisoned")
                .set_language(mimo_core::Lang::from_code(&settings.language));
            let voice = VoiceWake::new(voice_assets_dir(app.handle()), &settings.language);
            if settings.voice_wake_enabled {
                voice.set_enabled(app.handle(), true);
            }
            app.manage(voice);
            app.manage(Sounds::new());
            app.manage(speech::Speech::default());
            app.manage(panel::Panel::default());
            let reminders = reminders::Reminders::load(app.handle());
            reminders.start(app.handle().clone());
            app.manage(reminders);
            app.manage(tasks::Tasks::load(app.handle()));
            apps::keep_catalog_current(app.handle().clone());
            app.manage(Mutex::new(settings));

            if let Some(window) = app.get_webview_window("main") {
                place_island(&window);
                let _ = window.show();
            }

            if let Some(menu) = app.get_webview_window("tray-menu") {
                let menu_for_blur = menu.clone();
                menu.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        let _ = menu_for_blur.hide();
                    }
                });
            }

            setup_tray(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Where the Vosk runtime and speech models live: bundled next to the app
/// when packaged, or straight from the source tree in dev builds (where
/// `scripts/fetch-voice-models.ps1` downloads them).
fn voice_assets_dir(app: &AppHandle) -> std::path::PathBuf {
    let bundled = app.path().resource_dir().ok().map(|dir| dir.join("vosk"));
    match bundled {
        Some(dir) if dir.exists() || !cfg!(debug_assertions) => dir,
        _ => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("vosk"),
    }
}

/// Brings the pill up because the user asked for Mimo — via the global
/// shortcut (`source = "shortcut"`, typed prompt) or "Hey Mimo"
/// (`"voice"`, listening). The frontend picks the mode from the payload.
pub(crate) fn summon(app: &AppHandle, source: &str) {
    // Instant feedback that "Hey Mimo" was heard, before any animation.
    if source == "voice" {
        let enabled = app
            .state::<Mutex<mimo_core::Settings>>()
            .lock()
            .expect("settings mutex poisoned")
            .sounds_enabled;
        if enabled {
            app.state::<Sounds>().play(Sound::Wake);
        }
    }

    if let Some(menu) = app.get_webview_window("tray-menu") {
        let _ = menu.hide();
    }
    if let Some(window) = app.get_webview_window("main") {
        // Re-placing a visible window would snap it back to the base height
        // under an open settings drawer; the frontend closes that itself.
        if !window.is_visible().unwrap_or(false) {
            place_island(&window);
        }
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("mimo://summon", source);
    }
}

/// Sizes and positions the island window relative to the primary monitor, so
/// it looks proportionate and sits top-centered regardless of resolution,
/// DPI scaling, or multi-monitor layout.
pub(crate) fn place_island(window: &WebviewWindow) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };

    let scale = monitor.scale_factor();
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();

    let screen_logical_width = monitor_size.width as f64 / scale;
    let pill_width = (screen_logical_width * PILL_WIDTH_RATIO).clamp(PILL_MIN_WIDTH, PILL_MAX_WIDTH);

    let logical_width = pill_width + WINDOW_MARGIN_X as f64;
    let logical_height = PILL_HEIGHT + WINDOW_MARGIN_Y as f64;

    let physical_width = (logical_width * scale).round() as u32;
    let physical_height = (logical_height * scale).round() as u32;
    let _ = window.set_size(PhysicalSize::new(physical_width, physical_height));

    let physical_x = monitor_pos.x + (monitor_size.width as i32 - physical_width as i32) / 2;
    let physical_y = monitor_pos.y + (TOP_OFFSET * scale).round() as i32;
    let _ = window.set_position(PhysicalPosition::new(physical_x, physical_y));
}

/// Adds a tray icon so the pill can be brought back after being minimized —
/// there's no taskbar entry (the window is `skipTaskbar`), so this is
/// currently the only way back once hidden. Left-click reveals the pill;
/// right-click opens a small glass-styled menu (Settings / Close).
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("default window icon is missing from tauri.conf.json");

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Mimo")
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle();

            match &event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => {
                    if let Some(window) = app.get_webview_window("main") {
                        place_island(&window);
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = window.emit("mimo://reveal", ());
                    }
                }
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } => {
                    if let Some(menu) = app.get_webview_window("tray-menu") {
                        if let Ok(size) = menu.outer_size() {
                            let scale = menu.scale_factor().unwrap_or(1.0);
                            let tray_pos = rect.position.to_physical::<i32>(scale);
                            let tray_size = rect.size.to_physical::<i32>(scale);

                            let x = tray_pos.x + tray_size.width - size.width as i32;
                            let y = tray_pos.y - size.height as i32 - (TRAY_MENU_GAP as f64 * scale).round() as i32;
                            let _ = menu.set_position(PhysicalPosition::new(x, y));
                        }
                        let _ = menu.show();
                        let _ = menu.set_focus();
                        let _ = menu.emit("mimo://tray-menu-open", ());
                    }
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}
