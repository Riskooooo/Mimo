//! Screen captures: understanding "prends une capture d'écran", "enregistre
//! l'écran", "arrête l'enregistrement" (and the English equivalents), and
//! the replies. Capturing and saving is the shell's job.

use crate::info::Lang;

/// Subfolders of the user's Pictures and Videos folders.
pub const SCREENSHOT_FOLDER: &str = "Mimo Capture";
pub const RECORDING_FOLDER: &str = "Mimo Records";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCommand {
    Screenshot,
    StartRecording,
    StopRecording,
}

const SCREENSHOT_WORDS: &[&str] = &["screenshot", "screenshots", "capture", "captures"];
const RECORD_WORDS: &[&str] = &[
    "enregistre", "enregistrer", "enregistrement", "enregistres", "filme", "filmer", "record", "recording",
];
const SCREEN_WORDS: &[&str] = &["ecran", "screen"];
const VIDEO_WORDS: &[&str] = &["video", "videos"];
const STOP_WORDS: &[&str] = &[
    "arrete", "arreter", "stop", "stoppe", "stopper", "termine", "terminer", "fin", "finis", "coupe", "end",
    "finish", "halt",
];
const START_WORDS: &[&str] = &["lance", "commence", "demarre", "start", "begin"];
/// "ouvre l'outil Capture d'écran" opens the Snipping Tool app instead.
const APP_WORDS: &[&str] = &["outil", "snipping", "tool"];

pub fn detect(tokens: &[String]) -> Option<CaptureCommand> {
    let has_any = |words: &[&str]| tokens.iter().any(|t| words.contains(&t.as_str()));
    if has_any(APP_WORDS) {
        return None;
    }
    let screen = has_any(SCREEN_WORDS);
    let recording = has_any(RECORD_WORDS) || (has_any(VIDEO_WORDS) && (screen || has_any(SCREENSHOT_WORDS)));

    if recording {
        if has_any(STOP_WORDS) {
            return Some(CaptureCommand::StopRecording);
        }
        if screen || has_any(START_WORDS) || has_any(VIDEO_WORDS) {
            return Some(CaptureCommand::StartRecording);
        }
        return None;
    }
    has_any(SCREENSHOT_WORDS).then_some(CaptureCommand::Screenshot)
}

pub fn format_screenshot_saved(lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Capture enregistrée dans Images › {SCREENSHOT_FOLDER}."),
        Lang::En => format!("Screenshot saved to Pictures › {SCREENSHOT_FOLDER}."),
    }
}

pub fn format_recording_started(lang: Lang) -> String {
    match lang {
        Lang::Fr => "Enregistrement lancé. Dis « arrête l'enregistrement » pour l'arrêter.".to_string(),
        Lang::En => "Recording started. Say “stop recording” to stop it.".to_string(),
    }
}

pub fn format_recording_saved(lang: Lang, secs: u64) -> String {
    let length = match (secs / 60, secs % 60) {
        (0, s) => format!("{s} s"),
        (m, s) => format!("{m} min {s:02}"),
    };
    match lang {
        Lang::Fr => format!("Enregistrement de {length} sauvegardé dans Vidéos › {RECORDING_FOLDER}."),
        Lang::En => format!("{length} recording saved to Videos › {RECORDING_FOLDER}."),
    }
}

pub fn format_already_recording(lang: Lang) -> String {
    match lang {
        Lang::Fr => "L'écran est déjà en cours d'enregistrement.".to_string(),
        Lang::En => "The screen is already being recorded.".to_string(),
    }
}

pub fn format_not_recording(lang: Lang) -> String {
    match lang {
        Lang::Fr => "Aucun enregistrement en cours.".to_string(),
        Lang::En => "Nothing is being recorded.".to_string(),
    }
}

pub fn format_failed(lang: Lang, recording: bool) -> String {
    match (lang, recording) {
        (Lang::Fr, false) => "Impossible de prendre la capture.".to_string(),
        (Lang::Fr, true) => "Impossible d'enregistrer l'écran.".to_string(),
        (Lang::En, false) => "Couldn't take the screenshot.".to_string(),
        (Lang::En, true) => "Couldn't record the screen.".to_string(),
    }
}

/// As the speech models write them.
pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "take a screenshot", "screenshot", "capture the screen", "capture my screen", "record the screen",
            "record my screen", "start recording", "start recording the screen", "stop recording",
            "stop the recording", "end the recording",
        ],
        _ => &[
            "prends une capture d'écran", "fais une capture d'écran", "capture d'écran", "capture l'écran",
            "enregistre l'écran", "enregistre mon écran", "filme l'écran", "lance l'enregistrement",
            "lance un enregistrement de l'écran", "commence l'enregistrement", "arrête l'enregistrement",
            "stoppe l'enregistrement", "termine l'enregistrement", "fin de l'enregistrement",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn detect_text(text: &str) -> Option<CaptureCommand> {
        detect(&tokenize(text))
    }

    #[test]
    fn screenshots() {
        for text in ["prends une capture d'écran", "capture d'écran", "fais un screenshot", "take a screenshot", "capture my screen"] {
            assert_eq!(detect_text(text), Some(CaptureCommand::Screenshot), "{text}");
        }
    }

    #[test]
    fn recordings() {
        for text in ["enregistre l'écran", "filme mon écran", "lance l'enregistrement", "record my screen", "start recording", "fais une vidéo de l'écran"] {
            assert_eq!(detect_text(text), Some(CaptureCommand::StartRecording), "{text}");
        }
        for text in ["arrête l'enregistrement", "stoppe l'enregistrement de l'écran", "stop recording", "end the recording"] {
            assert_eq!(detect_text(text), Some(CaptureCommand::StopRecording), "{text}");
        }
    }

    #[test]
    fn other_requests_are_left_alone() {
        for text in ["ouvre l'outil capture d'écran", "open snipping tool", "ouvre youtube", "enregistre", "mets une vidéo"] {
            assert_eq!(detect_text(text), None, "{text}");
        }
    }

    #[test]
    fn recording_length() {
        assert_eq!(format_recording_saved(Lang::Fr, 42), "Enregistrement de 42 s sauvegardé dans Vidéos › Mimo Records.");
        assert_eq!(format_recording_saved(Lang::En, 125), "2 min 05 recording saved to Videos › Mimo Records.");
    }
}
