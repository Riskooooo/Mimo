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
    /// "J'ai besoin d'aide": the pill asks (in `reply`) whether it's an
    /// emergency or a question about Mimo, with a button for each.
    pub help: bool,
}

impl AskResponseDto {
    pub fn failed(reply: String) -> Self {
        Self { ok: false, reply, answer: false, translation: None, awaiting_copy: false, help: false }
    }

    /// A successful answer to read (kept up longer by the pill).
    pub fn answer(reply: String) -> Self {
        Self { ok: true, reply, answer: true, translation: None, awaiting_copy: false, help: false }
    }

    /// A successful action ("Opening YouTube…").
    pub fn done(reply: String) -> Self {
        Self { ok: true, reply, answer: false, translation: None, awaiting_copy: false, help: false }
    }

    /// An answer that didn't work out, still worded for reading.
    pub fn failed_answer(reply: String) -> Self {
        Self { ok: false, answer: true, ..Self::failed(reply) }
    }
}

/// Carries out one of the user's own commands. Sites are http(s) URLs
/// (checked when saved) and apps come from Windows' Start menu list.
fn run_custom(app: &AppHandle, action: &mimo_core::custom::CustomAction, lang: mimo_core::Lang) -> AskResponseDto {
    use mimo_core::custom::CustomAction;
    let opening = |name: &str| match lang {
        mimo_core::Lang::Fr => format!("Ouverture de {name}…"),
        mimo_core::Lang::En => format!("Opening {name}…"),
    };
    let outcome = match action {
        CustomAction::Reply { text } => return AskResponseDto::answer(text.clone()),
        CustomAction::OpenUrl { url } if url.starts_with("https://") || url.starts_with("http://") => {
            let host = url.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or(url);
            app.opener().open_url(url, None::<&str>).map(|()| opening(host)).map_err(|err| err.to_string())
        }
        CustomAction::LaunchApp { name, app_id } => std::process::Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{app_id}"))
            .spawn()
            .map(|_| opening(name))
            .map_err(|err| err.to_string()),
        _ => Err("unsupported custom action".to_string()),
    };
    match outcome {
        Ok(reply) => AskResponseDto::done(reply),
        Err(err) => {
            eprintln!("[custom] {err}");
            AskResponseDto::failed(match lang {
                mimo_core::Lang::Fr => "Ça n'a pas marché.".to_string(),
                mimo_core::Lang::En => "That didn't work.".to_string(),
            })
        }
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
    match intent {
        Intent::Chat { topic, lang } => {
            use chrono::Timelike;
            let now = chrono::Local::now();
            let variant = now.second() as usize;
            return AskResponseDto::answer(mimo_core::chat::reply(*topic, *lang, variant, now.hour()));
        }
        Intent::Custom { action, lang } => return run_custom(app, action, *lang),
        Intent::Help { lang } => {
            return AskResponseDto { help: true, ..AskResponseDto::answer(mimo_core::chat::help_question(*lang).to_string()) };
        }
        _ => {}
    }
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
