//! Window-control and settings commands local to the shell itself (not
//! `mimo-core` engine logic, so they live here rather than in
//! `mimo-commands`) — these all need Tauri APIs (paths, plugins, window
//! management) that have nothing to do with system analysis/automation.

use std::sync::Mutex;

use mimo_core::Settings;
use tauri::{AppHandle, Emitter, Manager, PhysicalSize, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use mimo_commands::AskResponseDto;
use mimo_core::reminders::ReminderCommand;
use mimo_core::tasks::{format_added, format_completed, format_deleted, format_not_found, format_summary, TaskCommand};
use mimo_core::translate::{copy_hint, TranslateRequest, Translation};
use mimo_core::Lang;
use mimo_core::{Engine, Intent};

use crate::activity::{midnight, Activity};
use crate::ai::{Ai, AiStatus};
use crate::capture::Recorder;
use crate::panel::{self, lang_code, Panel, PanelContent};
use crate::reminders::Reminders;
use crate::tasks::Tasks;
use crate::sounds::{Sound, Sounds};
use crate::speech::Speech;
use crate::suggestions::Suggestions;
use crate::voice::VoiceWake;
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
    // Finish a recording in progress, or its file would be unreadable.
    let _ = app.state::<Recorder>().stop(&app);
    app.state::<Ai>().shutdown();
    app.exit(0);
}

#[tauri::command]
pub fn get_settings(settings: tauri::State<'_, Mutex<Settings>>) -> Settings {
    settings.lock().expect("settings mutex poisoned").clone()
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
    persist(&app, &guard);
    Ok(guard.clone())
}

/// Rebinds the global "summon Mimo" shortcut. The new one is registered
/// before the old one is released, so a failure (invalid, or already owned
/// by another app) leaves the previous binding working.
#[tauri::command]
pub fn set_summon_shortcut(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    shortcut: String,
) -> Result<Settings, String> {
    let mut guard = settings.lock().expect("settings mutex poisoned");
    let french = mimo_core::Lang::from_code(&guard.language) == mimo_core::Lang::Fr;
    if mimo_core::validate_shortcut(&shortcut).is_err() {
        return Err(if french {
            "Raccourci invalide, essaie une autre touche.".to_string()
        } else {
            "Invalid shortcut, try another key.".to_string()
        });
    }
    if guard.summon_shortcut.eq_ignore_ascii_case(&shortcut) {
        return Ok(guard.clone());
    }

    let global = app.global_shortcut();
    global.register(shortcut.as_str()).map_err(|err| {
        eprintln!("[shortcut] {shortcut}: {err}");
        if french {
            format!("Impossible d'utiliser {shortcut} (déjà pris par une autre app ?).")
        } else {
            format!("Couldn't use {shortcut} (already taken by another app?).")
        }
    })?;
    let _ = global.unregister(guard.summon_shortcut.as_str());

    guard.summon_shortcut = shortcut;
    persist(&app, &guard);
    Ok(guard.clone())
}

/// Plays a UI sound, unless sounds are turned off.
#[tauri::command]
pub fn play_sound(
    settings: tauri::State<'_, Mutex<Settings>>,
    sounds: tauri::State<'_, Sounds>,
    sound: Sound,
) {
    if settings.lock().expect("settings mutex poisoned").sounds_enabled {
        sounds.play(sound);
    }
}

#[tauri::command]
pub fn set_sounds_enabled(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    enabled: bool,
) -> Settings {
    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.sounds_enabled = enabled;
    persist(&app, &guard);
    guard.clone()
}

/// Switches the whole app's language: replies (engine), the speech model
/// (voice wake) and, through the returned settings, the interface.
#[tauri::command]
pub fn set_language(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    voice: tauri::State<'_, VoiceWake>,
    language: String,
) -> Result<Settings, String> {
    mimo_core::validate_language(&language).map_err(|err| err.to_string())?;
    apply_language(&app, &voice, &language);

    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.language = language;
    persist(&app, &guard);
    Ok(guard.clone())
}

pub fn apply_language(app: &AppHandle, voice: &VoiceWake, language: &str) {
    app.state::<Mutex<Engine>>()
        .lock()
        .expect("engine mutex poisoned")
        .set_language(mimo_core::Lang::from_code(language));
    voice.set_language(app, language);
}

#[tauri::command]
pub fn set_voice_wake_enabled(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    voice: tauri::State<'_, VoiceWake>,
    enabled: bool,
) -> Settings {
    voice.set_enabled(&app, enabled);

    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.voice_wake_enabled = enabled;
    persist(&app, &guard);
    guard.clone()
}

/// Turns activity analysis on or off (sampling stops at once when off).
#[tauri::command]
pub fn set_activity_enabled(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    activity: tauri::State<'_, Activity>,
    enabled: bool,
) -> Settings {
    activity.set_enabled(enabled);

    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.activity_enabled = enabled;
    persist(&app, &guard);
    guard.clone()
}

/// Turns Mimo's own suggestions on or off (activity analysis keeps running).
#[tauri::command]
pub fn set_suggestions_enabled(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    enabled: bool,
) -> Settings {
    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.suggestions_enabled = enabled;
    persist(&app, &guard);
    guard.clone()
}

/// The user's answer to the suggestion on screen (`"accept"`, `"later"`,
/// `"never"`, `"dismiss"`); returns what the pill should say, if anything.
/// The pill can take focus again afterwards.
#[tauri::command]
pub fn answer_suggestion(app: AppHandle, window: WebviewWindow, choice: String) -> Option<String> {
    let reply = app.state::<Suggestions>().answer(&app, &choice);
    let _ = window.set_focusable(true);
    reply
}

/// Turns the local AI on (downloading it the first time) or off.
#[tauri::command]
pub fn set_ai_enabled(app: AppHandle, settings: tauri::State<'_, Mutex<Settings>>, enabled: bool) -> Settings {
    app.state::<Ai>().set_enabled(enabled);
    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.ai_enabled = enabled;
    persist(&app, &guard);
    guard.clone()
}

#[tauri::command]
pub fn get_ai_status(ai: tauri::State<'_, Ai>) -> AiStatus {
    ai.status()
}

/// Frees the disk space the model takes (the AI is off).
#[tauri::command]
pub fn delete_ai_files(ai: tauri::State<'_, Ai>) -> AiStatus {
    ai.delete_files();
    ai.status()
}

/// Mimo's main color, whether its glass is tinted with it and whether the
/// pill shows its little face (the "Personnaliser" window); every window
/// restyles on the change.
#[tauri::command]
pub fn set_appearance(
    app: AppHandle,
    settings: tauri::State<'_, Mutex<Settings>>,
    accent_color: String,
    tinted_glass: bool,
    character_enabled: bool,
) -> Result<Settings, String> {
    mimo_core::validate_accent_color(&accent_color).map_err(|err| err.to_string())?;
    let mut guard = settings.lock().expect("settings mutex poisoned");
    guard.accent_color = accent_color.to_lowercase();
    guard.tinted_glass = tinted_glass;
    guard.character_enabled = character_enabled;
    persist(&app, &guard);
    Ok(guard.clone())
}

#[tauri::command]
pub fn open_customize_window(app: AppHandle) {
    if let Some(menu) = app.get_webview_window("tray-menu") {
        let _ = menu.hide();
    }
    if let Some(window) = app.get_webview_window("customize") {
        if !window.is_visible().unwrap_or(false) {
            let _ = window.center();
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn close_customize_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("customize") {
        let _ = window.hide();
    }
}

/// Best-effort write: a failed save only loses the preference on restart,
/// which isn't worth failing the user's toggle over. Every window hears of
/// the change (the tray menu's labels and AI switch, the theme color…).
fn persist(app: &AppHandle, settings: &Settings) {
    if let Some(path) = settings_path(app) {
        let _ = settings.save(&path);
    }
    let _ = app.emit("mimo://settings-changed", settings.clone());
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
    voice: tauri::State<'_, VoiceWake>,
) -> Result<Settings, String> {
    let _ = app.autolaunch().disable();
    voice.set_enabled(&app, false);
    apply_language(&app, &voice, &Settings::default().language);
    app.state::<Reminders>().cancel(None);
    app.state::<Tasks>().clear();
    app.state::<Activity>().forget();
    app.state::<Suggestions>().forget();
    app.state::<crate::custom::CustomCommands>().forget(&app);
    // Its engine holds the model file open, and a download may be writing.
    let ai = app.state::<Ai>();
    ai.set_enabled(false);
    ai.shutdown();
    std::thread::sleep(std::time::Duration::from_millis(300));

    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Back to the defaults, which turn listening and activity analysis on:
    // restart them, or the toggles would show "on" with nothing running.
    let defaults = Settings::default();
    voice.set_enabled(&app, defaults.voice_wake_enabled);
    app.state::<Activity>().set_enabled(defaults.activity_enabled);
    ai.delete_files();

    let mut guard = settings.lock().expect("settings mutex poisoned");
    if !guard.summon_shortcut.eq_ignore_ascii_case(&defaults.summon_shortcut) {
        let global = app.global_shortcut();
        let _ = global.unregister(guard.summon_shortcut.as_str());
        let _ = global.register(defaults.summon_shortcut.as_str());
    }
    *guard = defaults;
    let _ = app.emit("mimo://settings-changed", guard.clone());
    Ok(guard.clone())
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

/// Handles a typed or spoken request. Desktop-only features (check-up,
/// notifications, reminders) are handled here; everything else goes to
/// `mimo-commands`. Async + a blocking task: some requests take a moment
/// (system scan, network) and sync commands would block the UI thread.
/// `spoken`: asked by voice — notifications are then read aloud.
#[tauri::command]
pub async fn ask_mimo(app: AppHandle, request: String, spoken: bool) -> AskResponseDto {
    tauri::async_runtime::spawn_blocking(move || handle_request(&app, &request, spoken))
        .await
        .unwrap_or_else(|err| AskResponseDto::failed(err.to_string()))
}

fn handle_request(app: &AppHandle, request: &str, spoken: bool) -> AskResponseDto {
    let intent = match mimo_commands::interpret(app, request) {
        Ok(intent) => intent,
        Err(reply) => return AskResponseDto::failed(reply),
    };
    // What the rules don't get, the local AI may.
    if intent == Intent::Unknown {
        if let Some(response) = ask_ai(app, request, spoken) {
            return response;
        }
    }
    handle_intent(app, intent, spoken)
}

fn handle_intent(app: &AppHandle, intent: Intent, spoken: bool) -> AskResponseDto {
    match intent {
        Intent::Diagnose { lang } => {
            let content = diagnostic(lang);
            let PanelContent::Diagnostic { summary, .. } = &content else { unreachable!() };
            let reply = summary.clone();
            panel::show(app, content);
            AskResponseDto::answer(reply)
        }
        Intent::ReadNotifications { lang } => {
            use mimo_core::notifications::{speech, summary};
            let apps = app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").installed_apps().clone();
            let (items, error) = match crate::notifications::read(&apps) {
                Ok(items) => (items, None),
                Err(err) => {
                    eprintln!("[notifications] {err}");
                    (Vec::new(), Some(err))
                }
            };
            let reply = match (&error, lang) {
                (Some(_), mimo_core::Lang::Fr) => "Impossible de lire les notifications.".to_string(),
                (Some(_), mimo_core::Lang::En) => "Couldn't read the notifications.".to_string(),
                (None, _) => summary(&items, lang),
            };
            if spoken && error.is_none() {
                app.state::<Speech>().say(&speech(&items, lang), lang);
            }
            let ok = error.is_none();
            panel::show(app, PanelContent::Notifications { lang: lang_code(lang), items, error });
            AskResponseDto { ok, ..AskResponseDto::answer(reply) }
        }
        Intent::Reminder { command, lang } => {
            use mimo_core::reminders::{format_cancelled, format_created, format_pending};
            let reminders = app.state::<Reminders>();
            match command {
                ReminderCommand::Create(request) => {
                    let message = request.message.clone();
                    let (hour, minute) = reminders.add(request.kind, request.when, request.message);
                    AskResponseDto::answer(format_created(lang, request.kind, hour, minute, message.as_deref()))
                }
                ReminderCommand::List => {
                    let items = reminders.list();
                    let reply = format_pending(lang, items.len());
                    panel::show(app, PanelContent::Reminders { lang: lang_code(lang), items });
                    AskResponseDto::answer(reply)
                }
                ReminderCommand::Cancel(kind) => {
                    let count = reminders.cancel(kind);
                    AskResponseDto::answer(format_cancelled(lang, kind, count))
                }
            }
        }
        Intent::Task { command, lang } => handle_task(app, command, lang),
        Intent::Activity { query, lang } => handle_activity(app, query, lang),
        Intent::Capture { command, lang } => handle_capture(app, command, lang),
        Intent::Control { command, lang } => {
            let (reply, ok) = crate::control::run(command, lang);
            AskResponseDto { ok, ..AskResponseDto::done(reply) }
        }
        Intent::Recall { command, lang } => handle_recall(app, command, lang),
        Intent::TextTool { tool, lang } => handle_text_tool(app, tool, lang, spoken),
        Intent::Translate { request, lang } => match request {
            TranslateRequest::Text { text, target } => match crate::translate::translate(&text, lang, target) {
                Ok(translation) => AskResponseDto {
                    reply: translation.translated.clone(),
                    translation: Some(translation),
                    ..AskResponseDto::answer(String::new())
                },
                Err(err) => {
                    eprintln!("[translate] {err}");
                    AskResponseDto::failed_answer(crate::translate::failure_message(lang).to_string())
                }
            },
            TranslateRequest::FromClipboard { target } => {
                *PENDING_TRANSLATION_TARGET.lock().expect("translation mutex poisoned") = target;
                AskResponseDto { awaiting_copy: true, ..AskResponseDto::answer(copy_hint(lang).to_string()) }
            }
        },
        other => mimo_commands::execute(app, &other),
    }
}

fn handle_capture(app: &AppHandle, command: mimo_core::capture::CaptureCommand, lang: Lang) -> AskResponseDto {
    use mimo_core::capture::*;
    let recorder = app.state::<Recorder>();
    let failed = |err: String, recording: bool| {
        eprintln!("[capture] {err}");
        AskResponseDto::failed_answer(format_failed(lang, recording))
    };
    match command {
        CaptureCommand::Screenshot => match crate::capture::screenshot(app) {
            Ok(_) => AskResponseDto::answer(format_screenshot_saved(lang)),
            Err(err) => failed(err, false),
        },
        CaptureCommand::StartRecording => match recorder.start(app) {
            Ok(true) => AskResponseDto::answer(format_recording_started(lang)),
            Ok(false) => AskResponseDto::failed_answer(format_already_recording(lang)),
            Err(err) => failed(err, true),
        },
        CaptureCommand::StopRecording => match recorder.stop(app) {
            Ok(Some((_, length))) => AskResponseDto::answer(format_recording_saved(lang, length.as_secs())),
            Ok(None) => AskResponseDto::failed_answer(format_not_recording(lang)),
            Err(err) => failed(err, true),
        },
    }
}

/// Hands a request the rules didn't understand to the local AI: it either
/// rewords it as one of Mimo's own commands (parsed and checked like any
/// other request) or answers it. `None`: no AI, or it couldn't help.
fn ask_ai(app: &AppHandle, request: &str, spoken: bool) -> Option<AskResponseDto> {
    use mimo_core::ai::{allowed, parse_understanding, understanding_prompt, understanding_schema, Understanding};
    let ai = app.state::<Ai>();
    if !ai.usable() {
        return None;
    }
    let lang = app_language(app);
    let now = chrono::Local::now().format("%A %d/%m/%Y, %H:%M").to_string();
    let output = ai
        .chat(&understanding_prompt(request, lang, &now), Some(understanding_schema()), 300, AI_TIMEOUT)
        .map_err(|err| eprintln!("[ai] {err}"))
        .ok()?;
    match parse_understanding(&output)? {
        Understanding::Command(command) => {
            let intent = mimo_commands::interpret(app, &command).ok()?;
            eprintln!("[ai] {request:?} -> {command:?} -> {intent:?}");
            allowed(&intent).then(|| handle_intent(app, intent, spoken))
        }
        Understanding::Answer(text) => {
            let text = mimo_core::ai::clean_text(&text);
            if spoken {
                app.state::<Speech>().say(&text, lang);
            }
            let title = match lang {
                Lang::Fr => "Réponse",
                Lang::En => "Answer",
            };
            Some(text_response(text, title, None))
        }
    }
}

/// How long a request to the local AI may take (it may have to load first).
const AI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
/// Longer than this, a text gets a card instead of the pill's one line.
const CARD_AFTER_CHARS: usize = 90;

/// A text to read: in the pill if short, in a card if not (or if it was
/// made from something, shown below it).
fn text_response(text: String, title: &str, original: Option<String>) -> AskResponseDto {
    if original.is_none() && text.chars().count() <= CARD_AFTER_CHARS {
        return AskResponseDto::answer(text);
    }
    AskResponseDto {
        card: Some(mimo_commands::Card { title: title.to_string(), text: text.clone(), original }),
        ..AskResponseDto::answer(text)
    }
}

/// "Résume / corrige / reformule / explique ce que j'ai copié."
fn handle_text_tool(app: &AppHandle, tool: mimo_core::ai::TextTool, lang: Lang, spoken: bool) -> AskResponseDto {
    use mimo_core::ai::{
        clean_text, failed_message, nothing_copied_message, text_tool_prompt, text_tool_title, unavailable_message, TextTool,
    };
    // About 2 500 tokens, leaving room for the answer in the context.
    const MAX_CHARS: usize = 8000;
    let ai = app.state::<Ai>();
    if !ai.usable() {
        return AskResponseDto::failed_answer(unavailable_message(lang, ai.is_downloading()).to_string());
    }
    let copied = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).unwrap_or_default();
    let copied = copied.trim();
    if copied.is_empty() {
        return AskResponseDto::failed_answer(nothing_copied_message(lang).to_string());
    }
    let text: String = copied.chars().take(MAX_CHARS).collect();
    let max_tokens = match tool {
        TextTool::Summarize | TextTool::Explain => 400,
        TextTool::Fix | TextTool::Rephrase => 2000,
    };
    match ai.chat(&text_tool_prompt(tool, &text, lang), None, max_tokens, std::time::Duration::from_secs(180)) {
        Ok(output) => {
            let result = clean_text(&output);
            if spoken && matches!(tool, TextTool::Summarize | TextTool::Explain) {
                app.state::<Speech>().say(&result, lang);
            }
            text_response(result, text_tool_title(tool, lang), Some(text))
        }
        Err(err) => {
            eprintln!("[ai] {err}");
            AskResponseDto::failed_answer(failed_message(lang).to_string())
        }
    }
}

/// A template-made answer reworded as a natural sentence by the local AI,
/// when it's on — the facts as they are otherwise (or if it fails).
fn natural(app: &AppHandle, facts: String, lang: Lang) -> String {
    let ai = app.state::<Ai>();
    if !ai.usable() {
        return facts;
    }
    match ai.chat(&mimo_core::ai::recap_prompt(&facts, lang), None, 200, std::time::Duration::from_secs(30)) {
        Ok(output) => {
            let text = mimo_core::ai::clean_text(&output);
            if text.is_empty() {
                facts
            } else {
                text
            }
        }
        Err(err) => {
            eprintln!("[ai] {err}");
            facts
        }
    }
}

/// "Résumé de ma journée" opens the activity panel; "combien de temps sur
/// Discord" is answered in the pill.
fn handle_activity(app: &AppHandle, query: mimo_core::activity::ActivityQuery, lang: Lang) -> AskResponseDto {
    use mimo_core::activity::{disabled_message, find_app, format_app_time};
    if !app.state::<Activity>().is_enabled() {
        return AskResponseDto::failed_answer(disabled_message(lang).to_string());
    }
    let content = activity_content(app, query.period, lang);
    let PanelContent::Activity { apps, summary, .. } = &content else { unreachable!() };
    match query.app {
        Some(said) => AskResponseDto::answer(format_app_time(&said, find_app(apps, &said), query.period, lang)),
        None => {
            let reply = summary.clone();
            panel::show(app, content);
            AskResponseDto::answer(natural(app, reply, lang))
        }
    }
}

/// "Qu'est-ce que je faisais hier vers 15h" is answered in the pill;
/// "rouvre ce que j'avais ouvert" relaunches those apps (not the ones still
/// running).
fn handle_recall(app: &AppHandle, command: mimo_core::recall::RecallCommand, lang: Lang) -> AskResponseDto {
    use chrono::TimeZone;
    use mimo_core::activity::disabled_message;
    use mimo_core::recall::{answer, describe_last_time, format_reopen, last_stretch, plan_reopen, RecallCommand};
    let activity = app.state::<Activity>();
    if !activity.is_enabled() {
        return AskResponseDto::failed_answer(disabled_message(lang).to_string());
    }
    let now = chrono::Local::now();
    let unix = |at: chrono::NaiveDateTime| chrono::Local.from_local_datetime(&at).earliest().map_or(0, |t| t.timestamp());

    match command {
        RecallCommand::WhatWasI(moment) => {
            let span = moment.resolve(now.naive_local());
            let (from, to) = (unix(span.from), unix(span.to));
            let sessions = activity.sessions(from, to);
            let facts = answer(&sessions, from, to, span.point.map(unix), span.moment, lang);
            let reply = if sessions.is_empty() { facts } else { natural(app, facts, lang) };
            text_response(reply, &span.moment.describe(lang), None)
        }
        RecallCommand::Reopen(moment) => {
            let (from, to, when) = match moment {
                Some(moment) => {
                    let span = moment.resolve(now.naive_local());
                    (unix(span.from), unix(span.to), span.moment.describe(lang))
                }
                None => {
                    // The last stretch before a break, within the last 2 days.
                    let recent = activity.sessions(now.timestamp() - 2 * 86_400, now.timestamp());
                    let (from, to) = last_stretch(&recent, now.timestamp());
                    (from, to, describe_last_time(lang).to_string())
                }
            };
            let sessions = activity.sessions(from, to);
            let apps = app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").installed_apps().clone();
            let plan = plan_reopen(&sessions, from, to, &crate::suggestions::running_apps(), &apps);
            for target in &plan.launch {
                if let Err(err) = std::process::Command::new("explorer.exe")
                    .arg(format!("shell:AppsFolder\\{}", target.app_id))
                    .spawn()
                {
                    eprintln!("[recall] can't launch {}: {err}", target.name);
                }
            }
            let reply = format_reopen(&plan, &when, lang);
            if plan.launch.is_empty() {
                AskResponseDto::answer(reply)
            } else {
                AskResponseDto::done(reply)
            }
        }
    }
}

fn activity_content(app: &AppHandle, period: mimo_core::activity::Period, lang: Lang) -> PanelContent {
    use mimo_core::activity::{format_summary, minutes_per_bucket, total_secs, usage_by_app, Period};
    let now = chrono::Local::now().timestamp();
    let (from, bucket, count) = match period {
        Period::Today => (midnight(0), 3600, 24),
        Period::Week => (midnight(6), 86_400, 7),
    };
    let sessions = app.state::<Activity>().sessions(from, now);
    let apps = usage_by_app(&sessions, from, now);
    PanelContent::Activity {
        lang: lang_code(lang),
        period,
        total_secs: total_secs(&sessions, from, now),
        summary: format_summary(&apps, period, lang),
        chart: minutes_per_bucket(&sessions, from, bucket, count),
        chart_start: from,
        apps,
    }
}

/// Explicit target ("traduis en anglais") of a pending "translate what I
/// copy next", for `translate_clipboard`.
static PENDING_TRANSLATION_TARGET: Mutex<Option<Lang>> = Mutex::new(None);

fn handle_task(app: &AppHandle, command: TaskCommand, lang: Lang) -> AskResponseDto {
    let tasks = app.state::<Tasks>();
    let now = crate::tasks::now();
    let response = match command {
        TaskCommand::Add { title, due } => {
            let task = tasks.add(title, due);
            AskResponseDto::answer(format_added(&task, now, lang))
        }
        TaskCommand::Complete { query } => match tasks.remove_matching(&query) {
            Some(task) => AskResponseDto::answer(format_completed(&task, lang)),
            None => AskResponseDto::failed_answer(format_not_found(&query, lang)),
        },
        TaskCommand::Delete { query } => match tasks.remove_matching(&query) {
            Some(task) => AskResponseDto::answer(format_deleted(&task, lang)),
            None => AskResponseDto::failed_answer(format_not_found(&query, lang)),
        },
        TaskCommand::List { today_only } => {
            let reply = format_summary(&tasks.snapshot(), now, today_only, lang);
            panel::show(app, tasks_content(app, lang));
            return AskResponseDto::answer(reply);
        }
    };
    panel::refresh_if_showing(app, tasks_content(app, lang));
    response
}

fn tasks_content(app: &AppHandle, lang: Lang) -> PanelContent {
    let tasks = app.state::<Tasks>();
    let summary = format_summary(&tasks.snapshot(), crate::tasks::now(), false, lang);
    PanelContent::Tasks { lang: lang_code(lang), items: tasks.list(), summary }
}

fn app_language(app: &AppHandle) -> Lang {
    app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").language()
}

/// Ticks a task off from the panel (it's then gone from the list).
#[tauri::command]
pub fn complete_task(app: AppHandle, id: u64) -> PanelContent {
    app.state::<Tasks>().remove(id);
    let content = tasks_content(&app, app_language(&app));
    *app.state::<Panel>().current_mut() = Some(content.clone());
    content
}

/// Adds a task typed in the panel ("acheter du pain demain 18h").
#[tauri::command]
pub fn add_task_text(app: AppHandle, text: String) -> PanelContent {
    if let Some((title, due)) = mimo_core::tasks::parse_new_task(&text) {
        app.state::<Tasks>().add(title, due);
    }
    let content = tasks_content(&app, app_language(&app));
    *app.state::<Panel>().current_mut() = Some(content.clone());
    content
}

/// Saves a task edited in the panel. `date` is "2026-10-03", `time`
/// "14:00" (both optional; a time without a date is dropped).
#[tauri::command]
pub fn update_task(app: AppHandle, id: u64, title: String, date: Option<String>, time: Option<String>) -> PanelContent {
    let title = title.trim();
    if !title.is_empty() {
        let date = date.and_then(|d| chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok());
        let time = time.and_then(|t| chrono::NaiveTime::parse_from_str(&t, "%H:%M").ok());
        app.state::<Tasks>().update(id, title.to_string(), date, time);
    }
    let content = tasks_content(&app, app_language(&app));
    *app.state::<Panel>().current_mut() = Some(content.clone());
    content
}

/// Adds a reminder typed in the panel ("appeler maman demain à 18h").
/// Fails with a hint when there's no moment or it has already passed.
#[tauri::command]
pub fn add_reminder_text(app: AppHandle, text: String, lang: String) -> Result<PanelContent, String> {
    use chrono::TimeZone;
    use mimo_core::reminders::{parse_new_reminder, ReminderTime};
    let french = lang != "en";
    let now = chrono::Local::now();
    let Some(new) = parse_new_reminder(&text) else {
        return Err(if french {
            "Précise quand : « dans 10 min », « demain à 9 h »…".to_string()
        } else {
            "Say when: “in 10 min”, “tomorrow at 9am”…".to_string()
        });
    };
    let due = match new.time {
        ReminderTime::In { seconds } => now.timestamp() + seconds as i64,
        ReminderTime::On(due) => {
            let (date, time) = due.resolve(now.naive_local());
            let date = date.unwrap_or(now.date_naive());
            // A day alone ("vendredi") rings in the morning.
            let time = time.unwrap_or(chrono::NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"));
            chrono::Local
                .from_local_datetime(&date.and_time(time))
                .earliest()
                .map(|t| t.timestamp())
                .unwrap_or(0)
        }
    };
    if due <= now.timestamp() {
        return Err(if french { "Ce moment est déjà passé.".to_string() } else { "That time has already passed.".to_string() });
    }
    let reminders = app.state::<Reminders>();
    reminders.add_at(new.kind, due, new.message);
    let content = PanelContent::Reminders { lang: if french { "fr" } else { "en" }, items: reminders.list() };
    *app.state::<Panel>().current_mut() = Some(content.clone());
    Ok(content)
}

/// Waits for the user to copy some text, then translates it.
#[tauri::command]
pub async fn translate_clipboard(app: AppHandle) -> Result<Translation, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let lang = app_language(&app);
        let target = PENDING_TRANSLATION_TARGET.lock().expect("translation mutex poisoned").take();
        let text = crate::translate::wait_for_copy()
            .ok_or_else(|| crate::translate::nothing_copied_message(lang).to_string())?;
        crate::translate::translate(&text, lang, target).map_err(|err| {
            eprintln!("[translate] {err}");
            crate::translate::failure_message(lang).to_string()
        })
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub fn cancel_translation() {
    crate::translate::CANCEL_WAIT.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    crate::translate::copy_to_clipboard(&text)
}

/// Grows the main window by `height` logical px below the bar (0 = back
/// to just the bar), for drawers like the translation card — same
/// resize-then-animate approach as the settings drawer.
#[tauri::command]
pub fn set_pill_drawer(window: WebviewWindow, height: f64) {
    let Ok(size) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let logical_height = PILL_HEIGHT + WINDOW_MARGIN_Y as f64 + height.clamp(0.0, 400.0);
    let physical_height = (logical_height * scale).round() as u32;
    let _ = window.set_size(PhysicalSize::new(size.width, physical_height));
}

fn diagnostic(lang: mimo_core::Lang) -> PanelContent {
    use mimo_core::system::{advise, summary};
    let snapshot = crate::diagnostics::collect();
    let advice = advise(&snapshot, lang);
    let summary = summary(&advice, lang);
    PanelContent::Diagnostic { lang: lang_code(lang), snapshot, advice, summary }
}

/// What the panel window should show (fetched when it loads).
#[tauri::command]
pub fn get_panel_content(panel: tauri::State<'_, Panel>) -> Option<PanelContent> {
    panel.current()
}

#[tauri::command]
pub fn close_panel(app: AppHandle) {
    app.state::<Speech>().stop();
    if let Some(window) = app.get_webview_window("panel") {
        let _ = window.hide();
    }
}

/// Re-runs the check-up from the panel's refresh button.
#[tauri::command]
pub async fn refresh_diagnostic(app: AppHandle, lang: String) -> Option<PanelContent> {
    let lang = if lang == "en" { mimo_core::Lang::En } else { mimo_core::Lang::Fr };
    let content = tauri::async_runtime::spawn_blocking(move || diagnostic(lang)).await.ok()?;
    panel::show(&app, content.clone());
    Some(content)
}

/// Deletes one reminder from the panel; returns the updated list.
#[tauri::command]
pub fn remove_reminder(app: AppHandle, id: u64, lang: String) -> PanelContent {
    let reminders = app.state::<Reminders>();
    reminders.remove(id);
    let lang = if lang == "en" { "en" } else { "fr" };
    let content = PanelContent::Reminders { lang, items: reminders.list() };
    *app.state::<Panel>().current_mut() = Some(content.clone());
    content
}

/// Stops reading aloud (e.g. the pill was dismissed).
#[tauri::command]
pub fn stop_speaking(speech: tauri::State<'_, Speech>) {
    speech.stop();
}
