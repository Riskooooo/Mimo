use std::sync::Mutex;

use mimo_core::translate::Translation;
use mimo_core::{Engine, EngineStatus, Intent};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatusDto {
    pub running: bool,
}

impl From<EngineStatus> for EngineStatusDto {
    fn from(status: EngineStatus) -> Self {
        Self {
            running: status.running,
        }
    }
}

#[tauri::command]
pub fn engine_status(engine: State<'_, Mutex<Engine>>) -> EngineStatusDto {
    engine
        .lock()
        .expect("engine mutex poisoned")
        .status()
        .into()
}

#[derive(Debug, Clone, Serialize)]
pub struct AskResponseDto {
    pub ok: bool,
    pub reply: String,
    /// The reply is an answer to read (time, weather…), not just a
    /// confirmation — the pill keeps it up longer.
    pub answer: bool,
    /// A translation to show in full (the pill opens a drawer for it).
    pub translation: Option<Translation>,
    /// "Translate what I copy next": the pill shows `reply` as a hint and
    /// calls `translate_clipboard`.
    pub awaiting_copy: bool,
}

impl AskResponseDto {
    pub fn failed(reply: String) -> Self {
        Self { ok: false, reply, answer: false, translation: None, awaiting_copy: false }
    }

    /// A successful answer to read (kept up longer by the pill).
    pub fn answer(reply: String) -> Self {
        Self { ok: true, reply, answer: true, translation: None, awaiting_copy: false }
    }

    /// A successful action ("Opening YouTube…").
    pub fn done(reply: String) -> Self {
        Self { ok: true, reply, answer: false, translation: None, awaiting_copy: false }
    }

    /// An answer that didn't work out, still worded for reading.
    pub fn failed_answer(reply: String) -> Self {
        Self { ok: false, answer: true, ..Self::failed(reply) }
    }
}

/// What a typed or spoken request means, or why it can't be handled
/// (engine not running).
pub fn interpret(app: &AppHandle, request: &str) -> Result<Intent, String> {
    app.state::<Mutex<Engine>>()
        .lock()
        .expect("engine mutex poisoned")
        .interpret(request)
        .map_err(|err| err.to_string())
}

/// Carries out the intents this bridge knows: opening sites/apps and
/// answering questions. Apps are only ever launched from the core's fixed
/// whitelist of program names or by the Start menu id Windows lists for an
/// installed app — never from user text. Blocking (a question may wait on
/// the network): call it off the UI thread. Intents that need the desktop
/// shell (panels, reminders) are handled there before reaching this.
pub fn execute(app: &AppHandle, intent: &Intent) -> AskResponseDto {
    let lang = app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").language();
    if let Intent::Ask { question, lang } = intent {
        return match crate::info::answer(question, *lang) {
            Ok(reply) => AskResponseDto::answer(reply),
            Err(reply) => AskResponseDto::failed_answer(reply),
        };
    }

    let outcome = match intent {
        Intent::OpenUrl { url, .. } | Intent::Search { url, .. } => app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|err| err.to_string()),
        Intent::LaunchApp { program, .. } => std::process::Command::new(program)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string()),
        // `shell:AppsFolder\<id>` launches classic and Store apps alike. The
        // id comes from Windows' own Start menu list, and is passed as a
        // single argument (no shell involved).
        Intent::LaunchInstalled { app_id, .. } => std::process::Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{app_id}"))
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string()),
        _ => Err(intent.reply(lang)),
    };

    match outcome {
        Ok(()) => AskResponseDto::done(intent.reply(lang)),
        Err(reply) => AskResponseDto::failed(reply),
    }
}
