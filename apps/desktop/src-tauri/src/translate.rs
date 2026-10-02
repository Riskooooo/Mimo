//! Quick translations through Google's free web endpoint
//! (`translate.googleapis.com`, `client=gtx`: no key, unofficial — it can
//! rate-limit or change). The text is sent to Google; nothing else is.
//!
//! "Translate what I copy next": the clipboard is watched through Windows'
//! clipboard sequence number, so copying the same text again still counts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mimo_core::translate::{target_language, Translation};
use mimo_core::Lang;
use serde_json::Value;

const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
/// Long enough to find and select the text, short enough not to linger.
const COPY_WAIT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(200);
/// Google's endpoint takes the text in the URL; keep requests reasonable.
const MAX_CHARS: usize = 4000;

#[link(name = "user32")]
extern "system" {
    fn GetClipboardSequenceNumber() -> u32;
}

/// Set to abandon a pending "wait for a copy".
pub static CANCEL_WAIT: AtomicBool = AtomicBool::new(false);

/// Translates `text` into the app language — or into the other one if the
/// text is already in it — unless `explicit` says otherwise.
pub fn translate(text: &str, app_lang: Lang, explicit: Option<Lang>) -> Result<Translation, String> {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let first_target = explicit.unwrap_or(app_lang);
    let (translated, source) = google(&text, first_target.code())?;

    let target = target_language(app_lang, &source, explicit);
    let translated = if target == first_target {
        translated
    } else {
        google(&text, target.code())?.0
    };
    Ok(Translation { original: text, translated, from: source, to: target.code().to_string() })
}

/// (translation, detected source language)
fn google(text: &str, target: &str) -> Result<(String, String), String> {
    let body = ureq::get("https://translate.googleapis.com/translate_a/single")
        .query("client", "gtx")
        .query("sl", "auto")
        .query("tl", target)
        .query("dt", "t")
        .query("q", text)
        .config()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .call()
        .map_err(|err| err.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|err| err.to_string())?;
    let json: Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;

    // [[["Bonjour","Hello",…], …], null, "en", …]
    let translated: String = json[0]
        .as_array()
        .ok_or("unexpected translation response")?
        .iter()
        .filter_map(|segment| segment[0].as_str())
        .collect();
    let source = json[2].as_str().unwrap_or("auto").to_string();
    Ok((translated, source))
}

/// Blocks until the user copies some text (or the wait times out / is
/// cancelled), then returns it.
pub fn wait_for_copy() -> Option<String> {
    CANCEL_WAIT.store(false, Ordering::SeqCst);
    let start_sequence = unsafe { GetClipboardSequenceNumber() };
    let started = Instant::now();
    while started.elapsed() < COPY_WAIT {
        if CANCEL_WAIT.load(Ordering::SeqCst) {
            return None;
        }
        std::thread::sleep(POLL);
        if unsafe { GetClipboardSequenceNumber() } != start_sequence {
            // The copying app may still be writing; give it a moment.
            std::thread::sleep(Duration::from_millis(80));
            let text = arboard::Clipboard::new().ok()?.get_text().ok()?;
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
        .map_err(|err| err.to_string())
}

/// Messages for the pill.
pub fn failure_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Traduction impossible pour le moment (connexion ?).",
        Lang::En => "Couldn't translate right now (offline?).",
    }
}

pub fn nothing_copied_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Aucun texte copié.",
        Lang::En => "No text was copied.",
    }
}
