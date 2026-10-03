//! Turns a free-form request ("ouvre moi youtube sur le navigateur",
//! "search rust tutorials on youtube") into a concrete [`Intent`].
//!
//! This is deliberately a small, rule-based parser with a fixed whitelist of
//! sites and programs, plus the apps installed on the PC (an [`AppCatalog`]
//! the shell discovers): it never turns user text into an arbitrary command
//! line, and the only free-form thing it can produce is a URL whose host is
//! either a known site or a plain domain name the user typed out.

use std::collections::HashSet;

use crate::activity::{self, ActivityQuery};
use crate::apps::{AppCatalog, InstalledApp};
use crate::info::{self, Lang, Question};
use crate::reminders::{self, ReminderCommand};
use crate::tasks::{self, TaskCommand};
use crate::translate::{self, TranslateRequest};
use crate::{notifications, system};

/// What Mimo decided to do with a request. Executing it is the shell's job —
/// this crate only decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    OpenUrl { label: String, url: String },
    Search { engine: &'static str, query: String, url: String },
    LaunchApp { label: &'static str, program: &'static str },
    /// An app from the Start menu, launched by its Windows app id.
    LaunchInstalled { name: String, app_id: String },
    /// A question to answer (time, date, weather) rather than something to open.
    Ask { question: Question, lang: Lang },
    /// Show the PC check-up window.
    Diagnose { lang: Lang },
    /// Show (and, when asked by voice, read) the current notifications.
    ReadNotifications { lang: Lang },
    Reminder { command: ReminderCommand, lang: Lang },
    Task { command: TaskCommand, lang: Lang },
    Translate { request: TranslateRequest, lang: Lang },
    /// A question about the user's activity (screen time, a day's summary).
    Activity { query: ActivityQuery, lang: Lang },
    Unknown,
}

impl Intent {
    /// Short, human-readable confirmation shown in the pill.
    pub fn reply(&self, lang: Lang) -> String {
        let opening = |name: &str| match lang {
            Lang::Fr => format!("Ouverture de {name}…"),
            Lang::En => format!("Opening {name}…"),
        };
        match self {
            Intent::OpenUrl { label, .. } => opening(&localized_label(label, lang)),
            Intent::Search { engine, query, .. } => match lang {
                Lang::Fr => format!("Recherche de « {query} » sur {engine}…"),
                Lang::En => format!("Searching {engine} for “{query}”…"),
            },
            Intent::LaunchApp { label, .. } => opening(&localized_label(label, lang)),
            Intent::LaunchInstalled { name, .. } => opening(name),
            // The answer needs live data (clock, weather); the shell words it
            // with `info::format_*`.
            Intent::Ask { .. }
            | Intent::Diagnose { .. }
            | Intent::ReadNotifications { .. }
            | Intent::Reminder { .. }
            | Intent::Task { .. }
            | Intent::Translate { .. }
            | Intent::Activity { .. } => "…".to_string(),
            Intent::Unknown => match lang {
                Lang::Fr => "Désolé, je n'ai pas compris. Essaie « ouvre youtube ».".to_string(),
                Lang::En => "Sorry, I didn't get that. Try “open youtube”.".to_string(),
            },
        }
    }
}

/// Built-in labels are written in English; these are the ones that differ
/// in French (brand names are the same in both).
fn localized_label(label: &str, lang: Lang) -> String {
    let french = match label {
        "your browser" => "ton navigateur",
        "Settings" => "Paramètres",
        "Notepad" => "Bloc-notes",
        "Calculator" => "Calculatrice",
        "File Explorer" => "Explorateur de fichiers",
        _ => label,
    };
    match lang {
        Lang::Fr => french.to_string(),
        Lang::En => label.to_string(),
    }
}

struct Site {
    aliases: &'static [&'static str],
    label: &'static str,
    url: &'static str,
}

const SITES: &[Site] = &[
    Site { aliases: &["youtube", "you tube", "yt"], label: "YouTube", url: "https://www.youtube.com" },
    Site { aliases: &["google"], label: "Google", url: "https://www.google.com" },
    Site { aliases: &["gmail", "mail", "mails", "email", "emails", "e mail", "boite mail", "boite de reception"], label: "Gmail", url: "https://mail.google.com" },
    Site { aliases: &["outlook", "hotmail"], label: "Outlook", url: "https://outlook.live.com" },
    Site { aliases: &["netflix"], label: "Netflix", url: "https://www.netflix.com" },
    Site { aliases: &["twitch"], label: "Twitch", url: "https://www.twitch.tv" },
    Site { aliases: &["github", "git hub"], label: "GitHub", url: "https://github.com" },
    Site { aliases: &["reddit"], label: "Reddit", url: "https://www.reddit.com" },
    Site { aliases: &["wikipedia", "wiki"], label: "Wikipedia", url: "https://www.wikipedia.org" },
    Site { aliases: &["facebook", "fb"], label: "Facebook", url: "https://www.facebook.com" },
    Site { aliases: &["instagram", "insta"], label: "Instagram", url: "https://www.instagram.com" },
    Site { aliases: &["twitter", "x"], label: "X", url: "https://x.com" },
    Site { aliases: &["linkedin"], label: "LinkedIn", url: "https://www.linkedin.com" },
    Site { aliases: &["amazon"], label: "Amazon", url: "https://www.amazon.com" },
    Site { aliases: &["spotify"], label: "Spotify", url: "https://open.spotify.com" },
    Site { aliases: &["deezer"], label: "Deezer", url: "https://www.deezer.com" },
    Site { aliases: &["chatgpt", "chat gpt", "chat g p t"], label: "ChatGPT", url: "https://chatgpt.com" },
    Site { aliases: &["claude"], label: "Claude", url: "https://claude.ai" },
    Site { aliases: &["maps", "google maps", "carte", "plan"], label: "Google Maps", url: "https://www.google.com/maps" },
    Site { aliases: &["drive", "google drive"], label: "Google Drive", url: "https://drive.google.com" },
    Site { aliases: &["traducteur", "traduction", "google traduction", "translate", "google translate"], label: "Google Translate", url: "https://translate.google.com" },
    Site { aliases: &["tiktok", "tik tok", "tic tac", "tick tock"], label: "TikTok", url: "https://www.tiktok.com" },
    Site { aliases: &["discord"], label: "Discord", url: "https://discord.com/app" },
    Site { aliases: &["whatsapp", "whats app", "what s app"], label: "WhatsApp", url: "https://web.whatsapp.com" },
    Site { aliases: &["leboncoin", "le bon coin", "bon coin"], label: "Leboncoin", url: "https://www.leboncoin.fr" },
    Site { aliases: &["navigateur", "browser", "internet", "web"], label: "your browser", url: "https://www.google.com" },
    // Not a website, but opened the same way (a protocol URL the OS handles).
    Site { aliases: &["parametres", "parametres windows", "reglages", "settings", "windows settings"], label: "Settings", url: "ms-settings:" },
];

struct App {
    aliases: &'static [&'static str],
    label: &'static str,
    program: &'static str,
}

const APPS: &[App] = &[
    App { aliases: &["bloc notes", "bloc note", "notepad"], label: "Notepad", program: "notepad.exe" },
    App { aliases: &["calculatrice", "calculette", "calculator", "calc"], label: "Calculator", program: "calc.exe" },
    App { aliases: &["explorateur", "explorateur de fichiers", "explorateur windows", "explorer", "file explorer", "fichiers", "files"], label: "File Explorer", program: "explorer.exe" },
    App { aliases: &["paint"], label: "Paint", program: "mspaint.exe" },
];

const OPEN_VERBS: &[&str] = &[
    "ouvre", "ouvres", "ouvrez", "ouvrir", "lance", "lances", "lancez", "lancer", "demarre",
    "demarrer", "affiche", "afficher", "montre", "montrer", "mets", "met", "va", "vas", "aller",
    "allez", "open", "launch", "start", "show", "run", "go",
];

const SEARCH_VERBS: &[&str] = &[
    "cherche", "cherches", "cherchez", "chercher", "recherche", "recherches", "rechercher",
    "trouve", "trouver", "search", "find", "google",
];

/// How many unrecognized words may precede the verb and still be skipped.
const MAX_STRAY_WORDS: usize = 2;

/// Words that sit between the verb and the thing to open ("ouvre-*moi le
/// site de* youtube", "go *to* github").
const OPEN_FILLERS: &[&str] = &[
    "moi", "me", "m", "nous", "le", "la", "les", "l", "un", "une", "des", "du", "de", "d", "sur",
    "the", "a", "an", "to", "on", "my", "mon", "ma", "mes", "site", "website", "page", "appli",
    "application", "app", "logiciel", "programme",
];

const SEARCH_FILLERS: &[&str] = &["moi", "me", "m", "for", "pour"];

/// Wake words and greetings that may lead a request, especially a spoken one.
const LEADING_GREETINGS: &[&str] = &[
    "hey", "he", "eh", "et", "ok", "okay", "salut", "dis", "mimo", "memo",
];

const POLITENESS: &[&[&str]] = &[
    &["s", "il", "te", "plait"],
    &["s", "il", "vous", "plait"],
    &["stp"],
    &["svp"],
    &["please"],
    &["merci"],
    &["thanks"],
    &["thank", "you"],
];

const PREAMBLES: &[&[&str]] = &[
    &["est", "ce", "que", "tu", "peux"],
    &["est", "ce", "que", "tu", "pourrais"],
    &["tu", "peux"],
    &["peux", "tu"],
    &["tu", "pourrais"],
    &["pourrais", "tu"],
    &["je", "veux"],
    &["je", "voudrais"],
    &["j", "aimerais"],
    &["can", "you"],
    &["could", "you"],
    &["i", "want", "to"],
    &["i", "d", "like", "to"],
];

/// "…in the browser" adds nothing: every URL opens in the default browser.
const BROWSER_SUFFIXES: &[&[&str]] = &[
    &["sur", "le", "navigateur"],
    &["dans", "le", "navigateur"],
    &["sur", "mon", "navigateur"],
    &["dans", "mon", "navigateur"],
    &["sur", "internet"],
    &["sur", "le", "web"],
    &["sur", "chrome"],
    &["dans", "chrome"],
    &["sur", "firefox"],
    &["sur", "edge"],
    &["in", "the", "browser"],
    &["in", "my", "browser"],
    &["on", "the", "browser"],
    &["in", "chrome"],
    &["on", "the", "internet"],
];

struct SearchEngine {
    suffixes: &'static [&'static [&'static str]],
    label: &'static str,
    url_prefix: &'static str,
}

const YOUTUBE_SEARCH: SearchEngine = SearchEngine {
    suffixes: &[&["sur", "youtube"], &["dans", "youtube"], &["on", "youtube"]],
    label: "YouTube",
    url_prefix: "https://www.youtube.com/results?search_query=",
};

const WIKIPEDIA_SEARCH: SearchEngine = SearchEngine {
    suffixes: &[&["sur", "wikipedia"], &["dans", "wikipedia"], &["on", "wikipedia"]],
    label: "Wikipedia",
    url_prefix: "https://fr.wikipedia.org/w/index.php?search=",
};

const GOOGLE_SEARCH: SearchEngine = SearchEngine {
    suffixes: &[&["sur", "google"], &["dans", "google"], &["on", "google"]],
    label: "Google",
    url_prefix: "https://www.google.com/search?q=",
};

/// Requests voice recognition is primed with (as a grammar), so short
/// commands are matched against known phrasings instead of transcribed
/// freely — far more reliable with small speech models. Spelled the way
/// each model writes words (accents, "chat g p t"); every phrase parses to
/// a known intent (see tests). Searches aren't listed: their queries are
/// open-ended, so only the verb is, followed by "[unk]".
pub fn voice_phrases(language: &str, apps: &AppCatalog) -> Vec<String> {
    let (verbs, targets, browser_suffix, search_verbs, app_verbs): (&[&str], &[&str], &str, &[&str], &[&str]) =
        match language {
            "en" => (
                EN_VOICE_VERBS,
                EN_VOICE_TARGETS,
                "in the browser",
                &["search", "search for", "find"],
                EN_APP_VERBS,
            ),
            _ => (
                FR_VOICE_VERBS,
                FR_VOICE_TARGETS,
                "sur le navigateur",
                &["cherche", "recherche", "cherche moi"],
                FR_APP_VERBS,
            ),
        };

    let mut phrases = Vec::new();
    for target in targets {
        phrases.push(target.to_string());
        for verb in verbs {
            phrases.push(format!("{verb} {target}"));
            if !BROWSER_TARGETS.contains(target) {
                phrases.push(format!("{verb} {target} {browser_suffix}"));
            }
        }
    }
    phrases.extend(search_verbs.iter().map(|verb| format!("{verb} [unk]")));
    phrases.extend(info::voice_questions(language).iter().map(|q| q.to_string()));
    phrases.extend(system::voice_phrases(language).iter().map(|q| q.to_string()));
    phrases.extend(notifications::voice_phrases(language).iter().map(|q| q.to_string()));
    phrases.extend(reminders::voice_phrases(language));
    phrases.extend(tasks::voice_phrases(language).iter().map(|q| q.to_string()));
    phrases.extend(translate::voice_phrases(language).iter().map(|q| q.to_string()));
    phrases.extend(activity::voice_phrases(language).iter().map(|q| q.to_string()));

    // Installed apps, by name, with the verbs people use for programs.
    let mut seen: HashSet<String> = phrases.iter().cloned().collect();
    for name in apps.apps().iter().filter_map(InstalledApp::spoken_name) {
        let with_verbs = app_verbs.iter().map(|verb| format!("{verb} {name}"));
        for phrase in std::iter::once(name.clone()).chain(with_verbs) {
            if seen.insert(phrase.clone()) {
                phrases.push(phrase);
            }
        }
    }
    phrases
}

const FR_APP_VERBS: &[&str] = &["ouvre", "ouvre moi", "lance", "lance moi", "démarre"];
const EN_APP_VERBS: &[&str] = &["open", "launch", "start"];

const BROWSER_TARGETS: &[&str] = &["le navigateur", "internet", "the browser"];

const FR_VOICE_VERBS: &[&str] = &[
    "ouvre", "ouvre moi", "lance", "lance moi", "mets", "mets moi", "affiche", "démarre",
    "va sur", "montre moi",
];

const FR_VOICE_TARGETS: &[&str] = &[
    "youtube", "google", "gmail", "mes mails", "netflix", "twitch", "github", "reddit",
    "wikipédia", "facebook", "instagram", "twitter", "linkedin", "amazon", "spotify", "deezer",
    "chat g p t", "claude", "google maps", "google drive", "le traducteur", "tic tac", "discord",
    "whatsapp", "leboncoin", "le navigateur", "internet", "les paramètres", "la calculatrice",
    "le bloc notes", "l'explorateur", "paint",
];

const EN_VOICE_VERBS: &[&str] = &["open", "launch", "start", "go to", "show me"];

const EN_VOICE_TARGETS: &[&str] = &[
    "youtube", "google", "gmail", "my email", "netflix", "twitch", "github", "git hub", "reddit",
    "wikipedia", "facebook", "instagram", "twitter", "linkedin", "amazon", "spotify",
    "chat g p t", "claude", "google maps", "google drive", "google translate", "tick tock",
    "discord", "whats app", "the browser", "settings", "the calculator", "notepad",
    "file explorer", "paint",
];

/// Parses a request with no installed apps known (fixed tables only),
/// answering in the language the request looks written in.
pub fn parse(request: &str) -> Intent {
    parse_with(request, &AppCatalog::default())
}

/// Like [`parse`], with the installed apps known.
pub fn parse_with(request: &str, apps: &AppCatalog) -> Intent {
    parse_in(request, apps, None)
}

/// Parses a request; answers (time, weather, reminders…) are worded in
/// `lang` — the app's language setting — or, if `None`, in the language
/// the request looks written in. Understanding works in both languages
/// either way.
pub fn parse_in(request: &str, apps: &AppCatalog, lang: Option<Lang>) -> Intent {
    let mut tokens = tokenize(request);
    let lang_or_detected = lang.unwrap_or_else(|| info::detect_lang(&tokens));

    // These work on the raw text: the text to translate and task titles
    // keep their exact spelling.
    if let Some(request) = translate::detect(request) {
        return Intent::Translate { request, lang: lang_or_detected };
    }
    if let Some(command) = tasks::detect(request) {
        return Intent::Task { command, lang: lang_or_detected };
    }

    for phrase in POLITENESS {
        remove_phrase(&mut tokens, phrase);
    }
    while tokens.first().is_some_and(|t| LEADING_GREETINGS.contains(&t.as_str())) {
        tokens.remove(0);
    }
    while PREAMBLES.iter().any(|p| strip_prefix(&mut tokens, p)) {}
    // ("va sur le navigateur" *is* the request, not a suffix: keep it when
    // only a verb would be left.)
    while BROWSER_SUFFIXES
        .iter()
        .any(|s| tokens.len() > s.len() + 1 && strip_suffix(&mut tokens, s))
    {}

    // Assistant features with distinctive words come first: "lance un
    // diagnostic", "mets un réveil" aren't about opening something.
    let lang = lang_or_detected;
    if let Some(query) = activity::detect(&tokens) {
        return Intent::Activity { query, lang };
    }
    if let Some(command) = reminders::detect(&tokens) {
        return Intent::Reminder { command, lang };
    }
    if system::is_diagnostic_request(&tokens) {
        return Intent::Diagnose { lang };
    }
    if notifications::is_notifications_request(&tokens) {
        return Intent::ReadNotifications { lang };
    }

    // Speech recognition can leave a stray word or two (e.g. the tail of
    // the wake phrase) in front of the verb: "mot ouvre youtube".
    let is_verb = |t: &String| OPEN_VERBS.contains(&t.as_str()) || SEARCH_VERBS.contains(&t.as_str());
    if let Some(verb_at) = tokens.iter().take(MAX_STRAY_WORDS + 1).position(is_verb) {
        tokens.drain(..verb_at);
    }

    let Some(first) = tokens.first().map(String::as_str) else {
        return Intent::Unknown;
    };

    // A question ("quelle heure est-il", "météo à Lyon") — unless it starts
    // with a verb: "ouvre la météo" opens the Météo app.
    if !OPEN_VERBS.contains(&first) && !SEARCH_VERBS.contains(&first) {
        if let Some((question, _)) = info::detect(&tokens) {
            return Intent::Ask { question, lang };
        }
    }

    if SEARCH_VERBS.contains(&first) && tokens.len() > 1 {
        tokens.remove(0);
        return parse_search(tokens);
    }

    // No verb at all ("mes mails", "the calculator") just names what to open.
    if OPEN_VERBS.contains(&first) {
        tokens.remove(0);
    }
    while tokens.first().is_some_and(|t| OPEN_FILLERS.contains(&t.as_str())) {
        tokens.remove(0);
    }

    resolve_target(&tokens, apps).unwrap_or(Intent::Unknown)
}

fn parse_search(mut tokens: Vec<String>) -> Intent {
    let mut engine = &GOOGLE_SEARCH;
    for candidate in [&YOUTUBE_SEARCH, &WIKIPEDIA_SEARCH, &GOOGLE_SEARCH] {
        if candidate.suffixes.iter().any(|s| strip_suffix(&mut tokens, s)) {
            engine = candidate;
            break;
        }
    }
    while tokens.first().is_some_and(|t| SEARCH_FILLERS.contains(&t.as_str())) {
        tokens.remove(0);
    }
    if tokens.is_empty() {
        return Intent::Unknown;
    }

    let query = tokens.join(" ");
    Intent::Search {
        engine: engine.label,
        url: format!("{}{}", engine.url_prefix, encode_query(&query)),
        query,
    }
}

/// Most specific first: an installed app named exactly that (so "spotify"
/// opens the Spotify app when it's installed, the website otherwise), the
/// known sites and built-in programs, an installed app by part of its name
/// ("chrome"), and finally a typed-out domain.
fn resolve_target(tokens: &[String], apps: &AppCatalog) -> Option<Intent> {
    let joined = tokens.join(" ");
    let target = joined.as_str();
    if target.is_empty() {
        return None;
    }
    let installed = |app: &InstalledApp| Intent::LaunchInstalled {
        name: app.name.clone(),
        app_id: app.id.clone(),
    };
    if let Some(app) = apps.exact(tokens) {
        return Some(installed(app));
    }
    if let Some(site) = SITES.iter().find(|s| s.aliases.contains(&target)) {
        return Some(Intent::OpenUrl {
            label: site.label.to_string(),
            url: site.url.to_string(),
        });
    }
    if let Some(app) = APPS.iter().find(|a| a.aliases.contains(&target)) {
        return Some(Intent::LaunchApp {
            label: app.label,
            program: app.program,
        });
    }
    if let Some(app) = apps.partial(tokens) {
        return Some(installed(app));
    }
    domain_url(target).map(|url| Intent::OpenUrl {
        label: target.to_string(),
        url,
    })
}

/// Accepts a typed-out domain ("example.com", "https://docs.rs/serde") —
/// one token, a dot, and an alphabetic TLD — and makes it an https URL.
fn domain_url(target: &str) -> Option<String> {
    if target.contains(' ') {
        return None;
    }
    let without_scheme = target
        .strip_prefix("https://")
        .or_else(|| target.strip_prefix("http://"))
        .unwrap_or(target);
    let host = without_scheme.split('/').next()?;
    let tld = host.rsplit('.').next()?;
    if !host.contains('.') || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(format!("https://{without_scheme}"))
}

/// Lowercases, folds French accents, and splits into words. Dots are kept
/// inside tokens so domains survive; elsewhere punctuation and hyphens
/// ("peux-tu", "bloc-notes", "s'il") become word breaks.
pub(crate) fn tokenize(input: &str) -> Vec<String> {
    tokenize_spans(input).into_iter().map(|token| token.folded).collect()
}

/// A word of the input: folded for matching, with where it sits in the
/// original text (so titles can be rebuilt with their exact spelling and
/// punctuation, e.g. "l'examen").
pub(crate) struct Token {
    pub folded: String,
    pub start: usize,
    pub end: usize,
}

pub(crate) fn tokenize_spans(input: &str) -> Vec<Token> {
    let is_word_char = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-' | '/' | ':');
    let is_edge = |c: char| matches!(c, '.' | '-' | '/' | ':');

    // Runs of word characters, with byte spans.
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut current: Option<usize> = None;
    for (i, c) in input.char_indices() {
        match (is_word_char(c), current) {
            (true, None) => current = Some(i),
            (false, Some(start)) => {
                segments.push((start, i));
                current = None;
            }
            _ => {}
        }
    }
    if let Some(start) = current {
        segments.push((start, input.len()));
    }

    let mut tokens = Vec::new();
    let mut push = |start: usize, end: usize| {
        if start < end {
            let folded: String = input[start..end].to_lowercase().chars().map(fold_accent).collect();
            tokens.push(Token { folded, start, end });
        }
    };
    for (mut start, mut end) in segments {
        // Trim punctuation at the edges ("youtube." → "youtube").
        while start < end && input[start..end].starts_with(is_edge) {
            start += input[start..end].chars().next().map_or(1, char::len_utf8);
        }
        while start < end && input[start..end].ends_with(is_edge) {
            end -= input[start..end].chars().next_back().map_or(1, char::len_utf8);
        }
        if input[start..end].contains('.') {
            // Domains stay whole.
            push(start, end);
            continue;
        }
        // Elsewhere, hyphens and the like separate words ("peux-tu").
        let mut piece = start;
        for (i, c) in input[start..end].char_indices() {
            if matches!(c, '-' | '/' | ':') {
                push(piece, start + i);
                piece = start + i + c.len_utf8();
            }
        }
        push(piece, end);
    }
    tokens
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
        'ç' => 'c',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' | 'ì' => 'i',
        'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ÿ' => 'y',
        _ => c,
    }
}

fn remove_phrase(tokens: &mut Vec<String>, phrase: &[&str]) {
    let mut i = 0;
    while i + phrase.len() <= tokens.len() {
        if tokens[i..i + phrase.len()].iter().zip(phrase).all(|(t, p)| t == p) {
            tokens.drain(i..i + phrase.len());
        } else {
            i += 1;
        }
    }
}

fn strip_prefix(tokens: &mut Vec<String>, phrase: &[&str]) -> bool {
    let matches = tokens.len() >= phrase.len() && tokens.iter().zip(phrase).all(|(t, p)| t == p);
    if matches {
        tokens.drain(..phrase.len());
    }
    matches
}

fn strip_suffix(tokens: &mut Vec<String>, phrase: &[&str]) -> bool {
    let Some(start) = tokens.len().checked_sub(phrase.len()) else {
        return false;
    };
    let matches = tokens[start..].iter().zip(phrase).all(|(t, p)| t == p);
    if matches {
        tokens.truncate(start);
    }
    matches
}

fn encode_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url_of(request: &str) -> Option<String> {
        match parse(request) {
            Intent::OpenUrl { url, .. } | Intent::Search { url, .. } => Some(url),
            _ => None,
        }
    }

    #[test]
    fn opens_known_site_from_natural_french() {
        assert_eq!(url_of("ouvre moi youtube sur le navigateur").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(url_of("Hey Mimo, ouvre-moi YouTube s'il te plaît !").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(url_of("est-ce que tu peux lancer le site de netflix").as_deref(), Some("https://www.netflix.com"));
        assert_eq!(url_of("va sur github").as_deref(), Some("https://github.com"));
    }

    #[test]
    fn opens_known_site_from_english() {
        assert_eq!(url_of("open youtube in the browser").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(url_of("go to reddit please").as_deref(), Some("https://www.reddit.com"));
    }

    #[test]
    fn bare_site_name_opens_it() {
        assert_eq!(url_of("youtube").as_deref(), Some("https://www.youtube.com"));
    }

    #[test]
    fn opening_the_browser_itself_works() {
        assert_eq!(url_of("ouvre le navigateur").as_deref(), Some("https://www.google.com"));
    }

    #[test]
    fn typed_domains_become_https_urls() {
        assert_eq!(url_of("ouvre example.com").as_deref(), Some("https://example.com"));
        assert_eq!(url_of("open https://docs.rs/serde").as_deref(), Some("https://docs.rs/serde"));
        assert_eq!(url_of("ouvre truc"), None);
    }

    #[test]
    fn launches_whitelisted_apps() {
        assert_eq!(
            parse("ouvre le bloc-notes"),
            Intent::LaunchApp { label: "Notepad", program: "notepad.exe" }
        );
        assert_eq!(
            parse("lance la calculatrice"),
            Intent::LaunchApp { label: "Calculator", program: "calc.exe" }
        );
    }

    #[test]
    fn searches_default_to_google() {
        assert_eq!(
            parse("cherche des recettes de crêpes"),
            Intent::Search {
                engine: "Google",
                query: "des recettes de crepes".into(),
                url: "https://www.google.com/search?q=des+recettes+de+crepes".into(),
            }
        );
    }

    #[test]
    fn searches_can_target_youtube() {
        assert_eq!(
            url_of("cherche moi lofi hip hop sur youtube").as_deref(),
            Some("https://www.youtube.com/results?search_query=lofi+hip+hop")
        );
    }

    #[test]
    fn search_queries_are_percent_encoded() {
        assert_eq!(encode_query("c++ & rust"), "c%2B%2B+%26+rust");
    }

    #[test]
    fn stray_words_before_the_verb_are_skipped() {
        assert_eq!(url_of("mot ouvre youtube").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(url_of("a uh open you tube").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(url_of("un deux trois ouvre youtube"), None);
    }

    #[test]
    fn every_voice_phrase_is_understood() {
        for language in ["fr", "en"] {
            for phrase in voice_phrases(language, &AppCatalog::default()) {
                if phrase.ends_with("[unk]") {
                    continue;
                }
                assert_ne!(parse(&phrase), Intent::Unknown, "{language}: {phrase:?}");
            }
        }
    }

    fn installed() -> AppCatalog {
        AppCatalog::new(
            [
                ("Spotify", "SpotifyAB.SpotifyMusic!Spotify"),
                ("Explorateur de fichiers", "Microsoft.Windows.Explorer"),
                ("Google Chrome", "Chrome"),
                ("Paramètres", "windows.immersivecontrolpanel"),
            ]
            .into_iter()
            .filter_map(|(name, id)| InstalledApp::new(name, id))
            .collect(),
        )
    }

    #[test]
    fn installed_apps_win_over_websites() {
        assert_eq!(
            parse_with("lance spotify", &installed()),
            Intent::LaunchInstalled {
                name: "Spotify".into(),
                app_id: "SpotifyAB.SpotifyMusic!Spotify".into()
            }
        );
        assert_eq!(
            parse_with("open spotify", &AppCatalog::default()),
            Intent::OpenUrl { label: "Spotify".into(), url: "https://open.spotify.com".into() }
        );
    }

    #[test]
    fn installed_apps_match_by_part_of_their_name() {
        assert!(matches!(
            parse_with("ouvre chrome", &installed()),
            Intent::LaunchInstalled { name, .. } if name == "Google Chrome"
        ));
        // Known sites still come before partial app matches.
        assert!(matches!(parse_with("ouvre google", &installed()), Intent::OpenUrl { .. }));
    }

    #[test]
    fn files_opens_the_file_explorer() {
        assert!(matches!(
            parse_with("lance fichiers", &AppCatalog::default()),
            Intent::LaunchApp { program: "explorer.exe", .. }
        ));
        assert!(matches!(
            parse_with("lance l'explorateur de fichiers", &installed()),
            Intent::LaunchInstalled { name, .. } if name == "Explorateur de fichiers"
        ));
    }

    #[test]
    fn voice_phrases_include_installed_apps() {
        let phrases = voice_phrases("fr", &installed());
        assert!(phrases.contains(&"lance spotify".to_string()));
        assert!(phrases.contains(&"ouvre paramètres".to_string()));
        for phrase in phrases.iter().filter(|p| !p.ends_with("[unk]")) {
            assert_ne!(parse_with(phrase, &installed()), Intent::Unknown, "{phrase:?}");
        }
    }

    #[test]
    fn questions_are_answered_not_opened() {
        assert!(matches!(
            parse("quelle heure est-il ?"),
            Intent::Ask { question: Question::Time, lang: Lang::Fr }
        ));
        assert!(matches!(
            parse("hey mimo what's the weather"),
            Intent::Ask { question: Question::Weather { .. }, lang: Lang::En }
        ));
        // With a verb, it's the app.
        let meteo_app = AppCatalog::new(InstalledApp::new("Météo", "Microsoft.BingWeather!App").into_iter().collect());
        assert!(matches!(parse_with("ouvre la météo", &meteo_app), Intent::LaunchInstalled { .. }));
    }

    #[test]
    fn assistant_requests_win_over_opening_things() {
        assert!(matches!(parse("lance un diagnostic"), Intent::Diagnose { lang: Lang::Fr }));
        assert!(matches!(parse("run a diagnostic"), Intent::Diagnose { lang: Lang::En }));
        assert!(matches!(parse("lis-moi mes notifications"), Intent::ReadNotifications { lang: Lang::Fr }));
        assert!(matches!(
            parse("mets un réveil à 7h"),
            Intent::Reminder { command: ReminderCommand::Create(_), lang: Lang::Fr }
        ));
        assert!(matches!(
            parse("remind me in 5 minutes to stretch"),
            Intent::Reminder { command: ReminderCommand::Create(_), lang: Lang::En }
        ));
    }

    #[test]
    fn language_setting_decides_the_answer_language() {
        // An English question with the app set to French gets a French answer.
        assert!(matches!(
            parse_in("what time is it", &AppCatalog::default(), Some(Lang::Fr)),
            Intent::Ask { question: Question::Time, lang: Lang::Fr }
        ));
        assert_eq!(parse("ouvre youtube").reply(Lang::Fr), "Ouverture de YouTube…");
        assert_eq!(parse("open youtube").reply(Lang::En), "Opening YouTube…");
        assert_eq!(parse("lance la calculatrice").reply(Lang::Fr), "Ouverture de Calculatrice…");
        assert_eq!(parse("open notepad").reply(Lang::En), "Opening Notepad…");
    }

    #[test]
    fn tasks_and_translations_are_recognized() {
        assert!(matches!(parse("ajoute une tâche appeler le plombier demain"), Intent::Task { command: TaskCommand::Add { .. }, .. }));
        assert!(matches!(parse("qu'est-ce que j'ai à faire aujourd'hui"), Intent::Task { command: TaskCommand::List { today_only: true }, .. }));
        assert!(matches!(parse("traduis: hello"), Intent::Translate { request: TranslateRequest::Text { .. }, .. }));
        assert!(matches!(parse("hey mimo traduis"), Intent::Translate { request: TranslateRequest::FromClipboard { .. }, .. }));
        // Still opens the site.
        assert!(matches!(parse("ouvre google translate"), Intent::OpenUrl { .. }));
    }

    #[test]
    fn nonsense_is_unknown() {
        assert_eq!(parse("fais moi un café"), Intent::Unknown);
        assert_eq!(parse(""), Intent::Unknown);
        assert_eq!(parse("hey mimo"), Intent::Unknown);
    }
}
