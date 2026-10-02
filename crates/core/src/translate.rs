//! Quick translations: "traduis: hello how are you" (text given), or just
//! "traduis" / "hey mimo translate" — then the text comes from the next
//! thing the user copies. Translating is the shell's job (it needs the
//! network); this decides what to translate and into what.

use serde::Serialize;

use crate::info::Lang;
use crate::intent::tokenize_spans;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateRequest {
    Text { text: String, target: Option<Lang> },
    /// No text given: wait for the user to copy some.
    FromClipboard { target: Option<Lang> },
}

/// A finished translation, as shown in the pill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Translation {
    pub original: String,
    pub translated: String,
    /// Language codes ("en", "fr", "es"…).
    pub from: String,
    pub to: String,
}

const VERBS: &[&str] = &["traduis", "traduire", "traduit", "traduction", "translate", "translation"];
const GREETINGS: &[&str] = &["hey", "he", "eh", "et", "ok", "okay", "salut", "dis", "mimo", "memo", "please", "stp"];
/// What may sit between the verb and the text, or be all there is.
const FILLERS: &[&str] = &["moi", "ca", "this", "that", "it", "le", "texte", "the", "text", "stp", "svp", "please", "cela", "ceci"];

/// Recognizes a translation request; the text keeps its exact spelling.
pub fn detect(request: &str) -> Option<TranslateRequest> {
    let tokens = tokenize_spans(request);
    let mut i = tokens.iter().position(|t| !GREETINGS.contains(&t.folded.as_str()))?;
    if !VERBS.contains(&tokens[i].folded.as_str()) {
        return None;
    }
    i += 1;

    // "en anglais", "into French", "to english"
    let mut target = None;
    if let (Some(prep), Some(language)) = (tokens.get(i), tokens.get(i + 1)) {
        if matches!(prep.folded.as_str(), "en" | "to" | "into" | "vers") {
            target = match language.folded.as_str() {
                "anglais" | "english" => Some(Lang::En),
                "francais" | "french" => Some(Lang::Fr),
                _ => None,
            };
            if target.is_some() {
                i += 2;
            }
        }
    }

    // Everything after that, as typed. Only fillers left = nothing given.
    let rest_start = match tokens.get(i) {
        Some(token) => token.start,
        None => return Some(TranslateRequest::FromClipboard { target }),
    };
    let only_fillers = tokens[i..].iter().all(|t| FILLERS.contains(&t.folded.as_str()));
    // Text after the verb starts right after its token, keeping a leading
    // quote or such, minus the ":" people type.
    let verb_end = tokens[i - 1].end;
    let rest = request[verb_end..].trim_start();
    let rest = rest.strip_prefix(':').unwrap_or(rest).trim();
    let text = if target.is_some() { request[rest_start..].trim() } else { rest };
    let text = text.strip_prefix(':').unwrap_or(text).trim();
    let text = text.trim_matches(|c| matches!(c, '"' | '“' | '”' | '«' | '»')).trim();

    if text.is_empty() || only_fillers {
        Some(TranslateRequest::FromClipboard { target })
    } else {
        Some(TranslateRequest::Text { text: text.to_string(), target })
    }
}

/// Into the app's language, unless the text already is in it — then into
/// the other one (an English speaker translating French text, and back).
pub fn target_language(app: Lang, detected_source: &str, explicit: Option<Lang>) -> Lang {
    if let Some(target) = explicit {
        return target;
    }
    match (app, detected_source) {
        (Lang::Fr, "fr") => Lang::En,
        (Lang::En, "en") => Lang::Fr,
        _ => app,
    }
}

/// Shown while waiting for the user to copy something.
pub fn copy_hint(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Sélectionne un texte puis copie-le (Ctrl + C)…",
        Lang::En => "Select some text and copy it (Ctrl + C)…",
    }
}

pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &["translate", "translate this", "translate that", "translate it"],
        _ => &["traduis", "traduis ça", "traduis moi ça", "traduire", "traduction", "traduis le texte"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str, target: Option<Lang>) -> Option<TranslateRequest> {
        Some(TranslateRequest::Text { text: t.into(), target })
    }

    #[test]
    fn text_given_inline_keeps_its_spelling() {
        assert_eq!(detect("traduis: hello how are you today"), text("hello how are you today", None));
        assert_eq!(detect("Translate: Où est la gare ?"), text("Où est la gare ?", None));
        assert_eq!(detect("traduis \"I'm on my way!\""), text("I'm on my way!", None));
        assert_eq!(detect("traduis en anglais : je suis en retard"), text("je suis en retard", Some(Lang::En)));
        assert_eq!(detect("translate into french: see you tomorrow"), text("see you tomorrow", Some(Lang::Fr)));
    }

    #[test]
    fn no_text_means_use_the_clipboard() {
        assert_eq!(detect("traduis"), Some(TranslateRequest::FromClipboard { target: None }));
        assert_eq!(detect("hey mimo traduis ça"), Some(TranslateRequest::FromClipboard { target: None }));
        assert_eq!(detect("hey memo translate this"), Some(TranslateRequest::FromClipboard { target: None }));
    }

    #[test]
    fn other_requests_are_ignored() {
        assert_eq!(detect("ouvre youtube"), None);
        assert_eq!(detect("open google translate"), None);
    }

    #[test]
    fn target_is_the_app_language_or_the_other_one() {
        assert_eq!(target_language(Lang::Fr, "en", None), Lang::Fr);
        assert_eq!(target_language(Lang::Fr, "fr", None), Lang::En);
        assert_eq!(target_language(Lang::En, "es", None), Lang::En);
        assert_eq!(target_language(Lang::Fr, "fr", Some(Lang::Fr)), Lang::Fr);
    }
}
