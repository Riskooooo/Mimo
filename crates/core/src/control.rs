//! Quick system controls: volume ("monte le son", "mets le volume à 30"),
//! mute, screen brightness, media keys (pause, next/previous track), and
//! locking or putting the PC to sleep. Understanding and wording only —
//! the shell talks to Windows.

use crate::info::Lang;
use crate::reminders::number;

/// How much "louder"/"plus lumineux" changes a level (out of 100).
pub const STEP: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Up,
    Down,
    /// An exact level, 0–100.
    Set(u8),
}

impl Level {
    /// The new level, from the `current` one (0–100).
    pub fn apply(self, current: u8) -> u8 {
        match self {
            Level::Up => current.saturating_add(STEP).min(100),
            Level::Down => current.saturating_sub(STEP),
            Level::Set(level) => level.min(100),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    Volume(Level),
    Mute,
    Unmute,
    Brightness(Level),
    PlayPause,
    NextTrack,
    PreviousTrack,
    Lock,
    Sleep,
}

const VOLUME_WORDS: &[&str] = &["son", "volume", "sound", "audio"];
const BRIGHTNESS_WORDS: &[&str] = &["luminosite", "brightness"];
const UP_WORDS: &[&str] = &[
    "monte", "monter", "augmente", "augmenter", "plus", "hausse", "up", "raise", "increase", "higher", "louder",
];
const DOWN_WORDS: &[&str] = &[
    "baisse", "baisser", "diminue", "diminuer", "reduis", "reduire", "moins", "down", "lower", "decrease",
    "reduce", "quieter", "dim",
];
const MAX_WORDS: &[&str] = &["fond", "max", "maximum", "full"];
const MUTE_WORDS: &[&str] = &["coupe", "couper", "mute", "silence"];
const UNMUTE_WORDS: &[&str] = &["remets", "remettre", "reactive", "reactiver", "retablis", "retablir", "rallume", "unmute"];
/// "plus fort", "moins fort" / "turn it up": volume without saying so.
const LOUDNESS_WORDS: &[&str] = &["fort", "louder", "quieter"];

/// Words that may surround a command without changing it.
const FILLERS: &[&str] = &[
    "mets", "met", "mettre", "fais", "le", "la", "les", "l", "un", "une", "en", "a", "au", "de", "du", "d", "moi",
    "me", "m", "the", "my", "this", "it", "to", "on", "turn", "set", "put", "go", "please", "ce", "cette",
    "cet", "mon", "ma", "tout",
];
/// What's playing, as people name it.
const MEDIA_WORDS: &[&str] = &[
    "musique", "chanson", "morceau", "titre", "son", "video", "music", "song", "track", "lecture",
];
const PAUSE_WORDS: &[&str] = &["pause", "play", "reprends", "reprendre", "resume", "unpause", "relance"];
const NEXT_WORDS: &[&str] = &["suivant", "suivante", "next", "skip", "passe"];
const PREVIOUS_WORDS: &[&str] = &["precedent", "precedente", "previous"];
const LOCK_WORDS: &[&str] = &["verrouille", "verrouiller", "verrouillage", "lock"];
const SLEEP_WORDS: &[&str] = &["veille", "sleep", "mise", "mode"];
const PC_WORDS: &[&str] = &[
    "pc", "ordinateur", "ordi", "session", "ecran", "windows", "computer", "screen", "laptop", "portable",
];

pub fn detect(tokens: &[String]) -> Option<ControlCommand> {
    let has_any = |words: &[&str]| tokens.iter().any(|t| words.contains(&t.as_str()));
    // Every word is one of `allowed` (or a filler), and there's at least one.
    let only = |allowed: &[&[&str]]| {
        !tokens.is_empty()
            && tokens
                .iter()
                .all(|t| FILLERS.contains(&t.as_str()) || allowed.iter().any(|words| words.contains(&t.as_str())))
    };

    if has_any(LOCK_WORDS) && only(&[LOCK_WORDS, PC_WORDS]) {
        return Some(ControlCommand::Lock);
    }
    if tokens.iter().any(|t| t == "veille" || t == "sleep") && only(&[SLEEP_WORDS, PC_WORDS]) {
        return Some(ControlCommand::Sleep);
    }

    let brightness = has_any(BRIGHTNESS_WORDS);
    let turn_it = tokens.iter().any(|t| t == "turn") && tokens.iter().any(|t| t == "it");
    let volume = has_any(VOLUME_WORDS) || has_any(LOUDNESS_WORDS) || turn_it;
    if brightness || volume {
        if let Some(level) = level(tokens) {
            return Some(if brightness { ControlCommand::Brightness(level) } else { ControlCommand::Volume(level) });
        }
    }
    if volume && !brightness {
        if has_any(UNMUTE_WORDS) {
            return Some(ControlCommand::Unmute);
        }
        if has_any(MUTE_WORDS) && only(&[MUTE_WORDS, VOLUME_WORDS]) {
            return Some(ControlCommand::Mute);
        }
    }
    if only(&[&["mute", "chut"]]) {
        return Some(ControlCommand::Mute);
    }
    if only(&[&["unmute"]]) {
        return Some(ControlCommand::Unmute);
    }

    // Media keys: nothing but the command and what's playing.
    if has_any(NEXT_WORDS) && only(&[NEXT_WORDS, MEDIA_WORDS]) {
        return Some(ControlCommand::NextTrack);
    }
    if has_any(PREVIOUS_WORDS) && only(&[PREVIOUS_WORDS, MEDIA_WORDS, &["reviens", "back"]]) {
        return Some(ControlCommand::PreviousTrack);
    }
    let stop_media = has_any(&["stop", "arrete"]) && has_any(MEDIA_WORDS);
    if (has_any(PAUSE_WORDS) || stop_media) && only(&[PAUSE_WORDS, MEDIA_WORDS, &["stop", "arrete"]]) {
        return Some(ControlCommand::PlayPause);
    }
    None
}

/// Up, down, or an exact level ("à 30", "50 %", "au max").
fn level(tokens: &[String]) -> Option<Level> {
    let has_any = |words: &[&str]| tokens.iter().any(|t| words.contains(&t.as_str()));
    if has_any(MAX_WORDS) {
        return Some(Level::Set(100));
    }
    if let Some(percent) = (0..tokens.len()).find_map(|i| percent(tokens, i)) {
        return Some(Level::Set(percent));
    }
    // "baisse" wins over "plus" in "baisse un peu plus".
    if has_any(DOWN_WORDS) {
        return Some(Level::Down);
    }
    if has_any(UP_WORDS) {
        return Some(Level::Up);
    }
    // "volume" alone isn't a command.
    None
}

/// A number from 0 to 100 at `start`: digits ("30", "30%") or words,
/// French tens included ("soixante dix", "quatre vingt", "cent").
fn percent(tokens: &[String], start: usize) -> Option<u8> {
    let word = |i: usize| tokens.get(i).map(String::as_str);
    let value = match (word(start)?, word(start + 1), word(start + 2)) {
        ("cent" | "hundred", ..) => 100,
        ("one" | "a", Some("hundred"), _) => 100,
        ("soixante", Some("dix"), _) => 70,
        ("quatre", Some("vingt" | "vingts"), Some("dix")) => 90,
        ("quatre", Some("vingt" | "vingts"), _) => 80,
        ("seventy", ..) => 70,
        ("eighty", ..) => 80,
        ("ninety", ..) => 90,
        (first, ..) => {
            let digits = first.trim_end_matches('%');
            match digits.parse::<u32>() {
                Ok(n) => n,
                Err(_) => number(tokens, start)?.0,
            }
        }
    };
    // "un", "une", "a" are articles here ("mets un peu plus"), not levels.
    if matches!(word(start)?, "un" | "une" | "a" | "one") && value <= 1 {
        return None;
    }
    u8::try_from(value).ok().filter(|v| *v <= 100)
}

pub fn format_volume(level: u8, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Volume : {level} %"),
        Lang::En => format!("Volume: {level}%"),
    }
}

pub fn format_muted(lang: Lang) -> String {
    match lang {
        Lang::Fr => "Son coupé.".to_string(),
        Lang::En => "Muted.".to_string(),
    }
}

pub fn format_unmuted(level: u8, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Son rétabli ({level} %)."),
        Lang::En => format!("Sound back on ({level}%)."),
    }
}

pub fn format_brightness(level: u8, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Luminosité : {level} %"),
        Lang::En => format!("Brightness: {level}%"),
    }
}

/// The confirmation for commands that don't report a level.
pub fn format_done(command: ControlCommand, lang: Lang) -> String {
    let (fr, en) = match command {
        ControlCommand::PlayPause => ("Lecture / pause.", "Play / pause."),
        ControlCommand::NextTrack => ("Morceau suivant.", "Next track."),
        ControlCommand::PreviousTrack => ("Morceau précédent.", "Previous track."),
        ControlCommand::Lock => ("Je verrouille le PC.", "Locking the PC."),
        ControlCommand::Sleep => ("Mise en veille…", "Going to sleep…"),
        ControlCommand::Mute => return format_muted(lang),
        ControlCommand::Volume(_) | ControlCommand::Unmute => ("Volume réglé.", "Volume set."),
        ControlCommand::Brightness(_) => ("Luminosité réglée.", "Brightness set."),
    };
    match lang {
        Lang::Fr => fr.to_string(),
        Lang::En => en.to_string(),
    }
}

pub fn format_failed(command: ControlCommand, lang: Lang) -> String {
    let (fr, en) = match command {
        ControlCommand::Volume(_) | ControlCommand::Mute | ControlCommand::Unmute => {
            ("Impossible de régler le son.", "Couldn't change the volume.")
        }
        ControlCommand::Brightness(_) => (
            "Je ne peux pas régler la luminosité de cet écran.",
            "I can't change this screen's brightness.",
        ),
        ControlCommand::PlayPause | ControlCommand::NextTrack | ControlCommand::PreviousTrack => {
            ("Impossible de contrôler la lecture.", "Couldn't control playback.")
        }
        ControlCommand::Lock => ("Impossible de verrouiller le PC.", "Couldn't lock the PC."),
        ControlCommand::Sleep => ("Impossible de mettre le PC en veille.", "Couldn't put the PC to sleep."),
    };
    match lang {
        Lang::Fr => fr.to_string(),
        Lang::En => en.to_string(),
    }
}

/// As the speech models write them; levels are enumerated by tens.
pub fn voice_phrases(lang: &str) -> Vec<String> {
    let mut phrases: Vec<String> = Vec::new();
    match lang {
        "en" => {
            phrases.extend(
                [
                    "turn up the volume", "turn the volume up", "volume up", "turn it up", "louder",
                    "turn down the volume", "turn the volume down", "volume down", "turn it down", "quieter",
                    "mute", "mute the sound", "unmute", "unmute the sound", "max volume", "full volume",
                    "increase the brightness", "turn up the brightness", "brightness up", "decrease the brightness",
                    "turn down the brightness", "brightness down", "max brightness", "pause", "pause the music",
                    "play", "play the music", "resume", "resume the music", "next song", "next track", "next",
                    "skip", "skip this song", "previous song", "previous track", "lock the pc", "lock the computer",
                    "lock my computer", "lock the screen", "put the pc to sleep", "put the computer to sleep",
                    "sleep mode",
                ]
                .map(String::from),
            );
            let tens = ["ten", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety", "one hundred"];
            for n in tens {
                phrases.push(format!("set the volume to {n}"));
                phrases.push(format!("volume {n}"));
                phrases.push(format!("set the brightness to {n}"));
            }
        }
        _ => {
            phrases.extend(
                [
                    "monte le son", "augmente le son", "monte le volume", "augmente le volume", "plus fort",
                    "baisse le son", "baisse le volume", "diminue le volume", "diminue le son", "moins fort",
                    "coupe le son", "remets le son", "réactive le son", "mets le son à fond", "volume au maximum",
                    "augmente la luminosité", "monte la luminosité", "baisse la luminosité",
                    "diminue la luminosité", "luminosité au maximum", "pause", "mets pause", "mets en pause",
                    "mets la musique en pause", "reprends", "reprends la musique", "relance la musique",
                    "chanson suivante", "musique suivante", "morceau suivant", "suivant", "passe à la suivante",
                    "chanson précédente", "morceau précédent", "précédent", "verrouille le pc",
                    "verrouille l'ordinateur", "verrouille la session", "verrouille l'écran",
                    "mets le pc en veille", "mets l'ordinateur en veille", "mise en veille",
                ]
                .map(String::from),
            );
            let tens = ["dix", "vingt", "trente", "quarante", "cinquante", "soixante", "soixante-dix", "quatre-vingts", "quatre-vingt-dix", "cent"];
            for n in tens {
                phrases.push(format!("mets le son à {n}"));
                phrases.push(format!("mets le volume à {n}"));
                phrases.push(format!("volume à {n}"));
                phrases.push(format!("mets la luminosité à {n}"));
            }
        }
    }
    phrases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn detect_text(text: &str) -> Option<ControlCommand> {
        detect(&tokenize(text))
    }

    #[test]
    fn volume() {
        use ControlCommand::Volume;
        for text in ["monte le son", "augmente le volume", "plus fort", "turn up the volume", "turn it up", "louder", "volume up"] {
            assert_eq!(detect_text(text), Some(Volume(Level::Up)), "{text}");
        }
        for text in ["baisse le son", "diminue le volume", "moins fort", "turn the volume down", "quieter", "volume down"] {
            assert_eq!(detect_text(text), Some(Volume(Level::Down)), "{text}");
        }
        assert_eq!(detect_text("mets le son à 30"), Some(Volume(Level::Set(30))));
        assert_eq!(detect_text("volume à 45 %"), Some(Volume(Level::Set(45))));
        assert_eq!(detect_text("mets le volume à cinquante"), Some(Volume(Level::Set(50))));
        assert_eq!(detect_text("mets le volume à soixante-dix"), Some(Volume(Level::Set(70))));
        assert_eq!(detect_text("mets le volume à quatre-vingt-dix"), Some(Volume(Level::Set(90))));
        assert_eq!(detect_text("set the volume to twenty"), Some(Volume(Level::Set(20))));
        assert_eq!(detect_text("mets le son à fond"), Some(Volume(Level::Set(100))));
        assert_eq!(detect_text("max volume"), Some(Volume(Level::Set(100))));
    }

    #[test]
    fn mute() {
        for text in ["coupe le son", "mute", "mute the sound", "chut"] {
            assert_eq!(detect_text(text), Some(ControlCommand::Mute), "{text}");
        }
        for text in ["remets le son", "réactive le son", "unmute"] {
            assert_eq!(detect_text(text), Some(ControlCommand::Unmute), "{text}");
        }
    }

    #[test]
    fn brightness() {
        use ControlCommand::Brightness;
        assert_eq!(detect_text("augmente la luminosité"), Some(Brightness(Level::Up)));
        assert_eq!(detect_text("baisse la luminosité"), Some(Brightness(Level::Down)));
        assert_eq!(detect_text("mets la luminosité à 70"), Some(Brightness(Level::Set(70))));
        assert_eq!(detect_text("max brightness"), Some(Brightness(Level::Set(100))));
        assert_eq!(detect_text("turn down the brightness"), Some(Brightness(Level::Down)));
    }

    #[test]
    fn media() {
        for text in ["pause", "mets pause", "mets en pause", "mets la musique en pause", "reprends la musique", "play", "resume the music", "stop la musique"] {
            assert_eq!(detect_text(text), Some(ControlCommand::PlayPause), "{text}");
        }
        for text in ["chanson suivante", "suivant", "passe à la suivante", "next song", "skip", "skip this song"] {
            assert_eq!(detect_text(text), Some(ControlCommand::NextTrack), "{text}");
        }
        for text in ["chanson précédente", "précédent", "previous track"] {
            assert_eq!(detect_text(text), Some(ControlCommand::PreviousTrack), "{text}");
        }
    }

    #[test]
    fn lock_and_sleep() {
        for text in ["verrouille le pc", "verrouille l'ordinateur", "verrouille la session", "lock the computer", "lock"] {
            assert_eq!(detect_text(text), Some(ControlCommand::Lock), "{text}");
        }
        for text in ["mets le pc en veille", "mise en veille", "put the computer to sleep", "sleep mode"] {
            assert_eq!(detect_text(text), Some(ControlCommand::Sleep), "{text}");
        }
    }

    #[test]
    fn other_requests_are_left_alone() {
        for text in [
            "ouvre youtube", "mets le son de damso", "mets squeezie sur youtube", "joue damso", "volume",
            "qu'est-ce que je faisais la veille", "ouvre spotify", "écoute damso", "play damso on spotify",
            "le prochain", "stop", "lance l'enregistrement",
        ] {
            assert_eq!(detect_text(text), None, "{text}");
        }
    }

    #[test]
    fn levels_are_clamped() {
        assert_eq!(Level::Up.apply(95), 100);
        assert_eq!(Level::Down.apply(5), 0);
        assert_eq!(Level::Up.apply(40), 50);
        assert_eq!(Level::Set(30).apply(80), 30);
    }
}
