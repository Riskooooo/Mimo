//! "Hey Mimo" voice wake — fully offline, on Vosk with the small French and
//! English models from `resources/vosk` (fetched by
//! `scripts/fetch-voice-models.ps1`). No audio or text ever leaves the PC.
//!
//! While enabled, one background thread owns the microphone:
//! - a grammar-restricted recognizer (only the wake phrase, plus "[unk]"
//!   for everything else) listens continuously — cheap, and hard to fool;
//! - as soon as the wake phrase shows up, even mid-sentence (via partial
//!   results), the audio that follows goes to two recognizers until that
//!   utterance ends: one restricted to Mimo's known commands
//!   (`mimo_core::voice_phrases`), which small models match far more
//!   reliably than they transcribe, and a free-form one used only when the
//!   request isn't one of those (a search, typically).
//!
//! "mimo" isn't in either model's vocabulary, so the wake phrase is matched
//! as "hey mémo" / "hey memo", which sound the same.

mod mic;
mod vosk;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use mimo_core::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use self::mic::Microphone;
use self::vosk::{vocabulary, Model, Recognizer, Vosk};

/// Back-off after a failure (no microphone, missing files…), doubling up to
/// a cap. Toggling the setting or changing language retries at once.
const RETRY_DELAY_MIN: Duration = Duration::from_secs(2);
const RETRY_DELAY_MAX: Duration = Duration::from_secs(60);

/// After the wake phrase: give up if nothing is said within this long…
const REQUEST_START_TIMEOUT: Duration = Duration::from_secs(6);
/// …and cut off a request that never seems to end.
const REQUEST_MAX: Duration = Duration::from_secs(12);

const AUDIO_POLL: Duration = Duration::from_millis(250);

/// The wake phrase is only recognized a moment after it's said, by which
/// time the start of the request may already have gone by. This much
/// recent audio is replayed into the request recognizer so the first word
/// isn't clipped ("pauvre youtube" instead of "ouvre youtube").
const PRE_ROLL: Duration = Duration::from_millis(600);

/// Wake grammar per model language: only words the model knows count.
fn wake_grammar(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &["hey memo", "memo", "[unk]"],
        _ => &["hé mémo", "hey mémo", "eh mémo", "mémo", "[unk]"],
    }
}

const WAKE_WORDS: &[&str] = &["mémo", "memo"];
/// How free-form recognition may spell the wake word.
const WAKE_WORDS_FREE: &[&str] = &["mémo", "memo", "mimo"];
const GREETINGS: &[&str] = &["hé", "hey", "eh"];

/// Sent to the main window after a wake: either what was said, or why
/// listening failed. Both `None` means nothing was said in time.
#[derive(Clone, Serialize)]
struct VoiceResult {
    text: Option<String>,
    error: Option<String>,
}

struct Control {
    enabled: bool,
    language: String,
    /// Bumped on every change, so the listening loop (and a thread backing
    /// off after an error) notices and restarts with the new settings.
    generation: u64,
    thread_started: bool,
}

type Shared = Arc<(Mutex<Control>, Condvar)>;

/// Handle to the voice-wake thread, kept in Tauri's managed state.
pub struct VoiceWake {
    shared: Shared,
    assets: PathBuf,
}

impl VoiceWake {
    pub fn new(assets: PathBuf, language: &str) -> Self {
        Self {
            shared: Arc::new((
                Mutex::new(Control {
                    enabled: false,
                    language: language.to_string(),
                    generation: 0,
                    thread_started: false,
                }),
                Condvar::new(),
            )),
            assets,
        }
    }

    /// Starts or stops listening. The thread is spawned lazily on first
    /// enable and then parks (with the microphone released) while disabled.
    pub fn set_enabled(&self, app: &AppHandle, enabled: bool) {
        self.update(app, |control| {
            let changed = control.enabled != enabled;
            control.enabled = enabled;
            changed
        });
    }

    /// Rebuilds the recognizers' grammar (e.g. the installed apps changed).
    pub fn refresh(&self, app: &AppHandle) {
        self.update(app, |_| true);
    }

    pub fn set_language(&self, app: &AppHandle, language: &str) {
        self.update(app, |control| {
            let changed = control.language != language;
            control.language = language.to_string();
            changed
        });
    }

    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Control) -> bool) {
        let (lock, cvar) = &*self.shared;
        let mut control = lock.lock().expect("voice mutex poisoned");
        if change(&mut control) {
            control.generation += 1;
        }
        if control.enabled && !control.thread_started {
            let shared = self.shared.clone();
            let assets = self.assets.clone();
            let app = app.clone();
            let spawned = std::thread::Builder::new()
                .name("mimo-voice".into())
                .spawn(move || run(app, shared, assets));
            control.thread_started = spawned.is_ok();
        }
        cvar.notify_all();
    }
}

fn current_generation(shared: &Shared) -> u64 {
    shared.0.lock().expect("voice mutex poisoned").generation
}

/// Parks the thread until listening is enabled.
fn wait_until_enabled(shared: &Shared) -> (u64, String) {
    let (lock, cvar) = &**shared;
    let control = cvar
        .wait_while(lock.lock().expect("voice mutex poisoned"), |c| !c.enabled)
        .expect("voice mutex poisoned");
    (control.generation, control.language.clone())
}

fn wait_for_retry(shared: &Shared, generation: u64, failures: u32) {
    let delay = RETRY_DELAY_MIN
        .saturating_mul(1 << failures.saturating_sub(1).min(5))
        .min(RETRY_DELAY_MAX);
    let (lock, cvar) = &**shared;
    let _ = cvar
        .wait_timeout_while(lock.lock().expect("voice mutex poisoned"), delay, |c| {
            c.generation == generation
        })
        .expect("voice mutex poisoned");
}

/// What stays loaded between listening sessions: the engine for the whole
/// run, the model until the language changes.
#[derive(Default)]
struct Loaded {
    vosk: Option<Arc<Vosk>>,
    model: Option<LoadedModel>,
}

struct LoadedModel {
    language: String,
    model: Model,
    vocabulary: Option<HashSet<String>>,
}

fn run(app: AppHandle, shared: Shared, assets: PathBuf) {
    let mut loaded = Loaded::default();
    let mut failures: u32 = 0;
    let mut last_generation = u64::MAX;

    loop {
        let (generation, language) = wait_until_enabled(&shared);
        if generation != last_generation {
            last_generation = generation;
            failures = 0;
        }

        match listen(&app, &shared, &assets, generation, &language, &mut loaded) {
            // Settings changed (disabled, or another language): go around.
            Ok(()) => failures = 0,
            Err(err) => {
                failures += 1;
                eprintln!("[voice] failure #{failures}: {err}");
                let _ = app.emit("mimo://voice-error", localize_error(&err, &language));
                wait_for_retry(&shared, generation, failures);
            }
        }
    }
}

/// Listens until the settings change (`Ok`) or something breaks (`Err`).
fn listen(
    app: &AppHandle,
    shared: &Shared,
    assets: &Path,
    generation: u64,
    language: &str,
    loaded: &mut Loaded,
) -> Result<(), String> {
    let vosk = match &loaded.vosk {
        Some(vosk) => vosk.clone(),
        None => loaded.vosk.insert(Vosk::load(&assets.join("lib"))?).clone(),
    };
    if loaded.model.as_ref().is_none_or(|m| m.language != language) {
        loaded.model = None; // free the previous language first
        let model = Model::load(&vosk, &assets.join("models").join(language))?;
        let model_dir = assets.join("models").join(language);
        loaded.model = Some(LoadedModel {
            language: language.to_string(),
            model,
            vocabulary: vocabulary(&model_dir),
        });
    }
    let LoadedModel { model, vocabulary, .. } = loaded.model.as_ref().expect("model loaded above");

    let mic = Microphone::open()?;
    let rate = mic.sample_rate() as f32;
    let started = Instant::now();
    let grammar = command_grammar(app, language, vocabulary.as_ref());
    let mut wake = Recognizer::with_grammar(model, rate, wake_grammar(language))?;
    let mut request = RequestRecognizers {
        commands: Recognizer::with_grammar(model, rate, &grammar)?,
        dictation: Recognizer::free(model, rate)?,
    };
    eprintln!(
        "[voice] listening for wake phrase ({language}, {} Hz, {} command phrases, ready in {:?})",
        mic.sample_rate(),
        grammar.len(),
        started.elapsed()
    );

    let pre_roll_samples = (PRE_ROLL.as_secs_f32() * rate) as usize;
    let mut recent = AudioHistory::new(pre_roll_samples);

    while current_generation(shared) == generation {
        let Some(chunk) = mic.next(AUDIO_POLL)? else {
            continue;
        };
        let ended = wake.accept(&chunk)?;
        recent.push(chunk);
        let heard = if ended { wake.text() } else { wake.partial() };
        if !is_wake_phrase(&heard, ended) {
            continue;
        }

        eprintln!("[voice] wake phrase heard: {heard:?}");
        wake.reset();
        crate::summon(app, "voice");

        let text = listen_for_request(&mic, &mut request, recent.take())?;
        eprintln!("[voice] request: {text:?}");
        let _ = app.emit("mimo://voice-result", VoiceResult { text, error: None });
        mic.drain();
    }
    Ok(())
}

/// The last few hundred milliseconds of audio, oldest first.
struct AudioHistory {
    chunks: VecDeque<Vec<i16>>,
    samples: usize,
    capacity: usize,
}

impl AudioHistory {
    fn new(capacity: usize) -> Self {
        Self { chunks: VecDeque::new(), samples: 0, capacity }
    }

    fn push(&mut self, chunk: Vec<i16>) {
        self.samples += chunk.len();
        self.chunks.push_back(chunk);
        while self.samples > self.capacity {
            let Some(oldest) = self.chunks.pop_front() else { break };
            self.samples -= oldest.len();
        }
    }

    fn take(&mut self) -> Vec<Vec<i16>> {
        self.samples = 0;
        self.chunks.drain(..).collect()
    }
}

/// Known commands (installed apps included), also with the wake phrase in
/// front: the pre-roll replays it, and the request is often said in the
/// same breath.
fn command_grammar(app: &AppHandle, language: &str, vocabulary: Option<&HashSet<String>>) -> Vec<String> {
    let mut commands = app
        .state::<Mutex<Engine>>()
        .lock()
        .expect("engine mutex poisoned")
        .voice_phrases(language);
    if let Some(vocabulary) = vocabulary {
        commands.retain(|phrase| speakable(phrase, vocabulary));
    }
    with_wake_prefixes(commands, language)
}

/// Every word is one the model knows (Vosk would otherwise drop the
/// unknown ones and keep a truncated phrase that matches the wrong thing).
fn speakable(phrase: &str, vocabulary: &HashSet<String>) -> bool {
    phrase
        .split_whitespace()
        .all(|word| word == "[unk]" || vocabulary.contains(word))
}

fn with_wake_prefixes(commands: Vec<String>, language: &str) -> Vec<String> {
    let wake_prefixes: Vec<&str> = wake_grammar(language)
        .iter()
        .copied()
        .filter(|phrase| *phrase != "[unk]")
        .collect();

    let mut grammar = commands.clone();
    for prefix in wake_prefixes {
        grammar.extend(commands.iter().map(|command| format!("{prefix} {command}")));
    }
    grammar.push("[unk]".to_string());
    grammar
}

struct RequestRecognizers {
    /// Restricted to known commands (plus "[unk]").
    commands: Recognizer,
    /// Free-form fallback.
    dictation: Recognizer,
}

/// Collects the utterance right after the wake phrase. Both recognizers hear
/// the same audio; the command one decides when the utterance is over.
fn listen_for_request(
    mic: &Microphone,
    rec: &mut RequestRecognizers,
    pre_roll: Vec<Vec<i16>>,
) -> Result<Option<String>, String> {
    rec.commands.reset();
    rec.dictation.reset();
    // Free-form text of the current utterance; the dictation recognizer may
    // close a phrase before the command one does.
    let mut dictated = String::new();

    let feed = |rec: &mut RequestRecognizers, dictated: &mut String, chunk: &[i16]| {
        if rec.dictation.accept(chunk)? {
            dictated.push(' ');
            dictated.push_str(&rec.dictation.text());
        }
        rec.commands.accept(chunk)
    };

    let start = Instant::now();
    let mut speaking = false;
    for chunk in &pre_roll {
        feed(rec, &mut dictated, chunk)?;
    }

    loop {
        let elapsed = start.elapsed();
        if elapsed > REQUEST_MAX || (!speaking && elapsed > REQUEST_START_TIMEOUT) {
            dictated.push(' ');
            dictated.push_str(&rec.dictation.finish());
            return Ok(choose_request(&rec.commands.finish(), &dictated));
        }
        let Some(chunk) = mic.next(AUDIO_POLL)? else {
            continue;
        };
        if feed(rec, &mut dictated, &chunk)? {
            dictated.push(' ');
            dictated.push_str(&rec.dictation.finish());
            let matched = rec.commands.text();
            eprintln!("[voice] heard: commands={matched:?} dictation={:?}", dictated.trim());
            if let Some(request) = choose_request(&matched, &dictated) {
                return Ok(Some(request));
            }
            // Only the wake phrase or noise so far: keep waiting.
            dictated.clear();
        } else if !speaking && clean_request(&rec.commands.partial()).is_some() {
            speaking = true;
        }
    }
}

/// A clean match against the known commands wins. If the command recognizer
/// heard *something* it couldn't place ("[unk]", e.g. a search query), the
/// free-form transcription is used instead. Nothing but the wake phrase
/// means there's no request yet.
fn choose_request(matched: &str, dictated: &str) -> Option<String> {
    let command = clean_request(matched)?;
    if !command.contains("[unk]") {
        return Some(command);
    }
    clean_request(dictated).filter(|text| !text.contains("[unk]"))
}

/// Voice errors are written in English where they happen; this gives the
/// French version of the ones a user can actually act on.
fn localize_error(err: &str, language: &str) -> String {
    if language != "fr" {
        return err.to_string();
    }
    let french = [
        ("Voice files are missing", "Fichiers vocaux manquants : lance `npm run setup:voice` dans apps/desktop, puis réactive l'option."),
        ("No microphone found", "Aucun micro détecté."),
        ("Couldn't open the microphone", "Impossible d'ouvrir le micro."),
        ("Couldn't start the microphone", "Impossible de démarrer le micro."),
        ("The microphone stopped working", "Le micro ne répond plus (débranché ?)."),
        ("Unsupported microphone format", "Format de micro non pris en charge."),
        ("Couldn't load the speech engine", "Impossible de charger le moteur vocal."),
        ("Couldn't load the voice model", "Impossible de charger le modèle vocal."),
        ("Couldn't start speech recognition", "Impossible de démarrer la reconnaissance vocale."),
        ("Speech recognition failed", "La reconnaissance vocale a échoué."),
    ];
    french
        .iter()
        .find(|(english, _)| err.starts_with(english))
        .map(|(_, fr)| fr.to_string())
        .unwrap_or_else(|| err.to_string())
}

/// A wake word right after a greeting ("hey mémo"), or said on its own as a
/// whole utterance. A lone "mémo" inside a longer sentence doesn't count.
fn is_wake_phrase(text: &str, utterance_ended: bool) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.iter().enumerate().any(|(i, word)| {
        WAKE_WORDS.contains(word)
            && ((i > 0 && GREETINGS.contains(&words[i - 1])) || (utterance_ended && words.len() == 1))
    })
}

/// The request part of an utterance: whatever follows the (last) wake word
/// — anything before it is the wake phrase itself or noise — minus a
/// leading greeting. `None` if nothing is left.
fn clean_request(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let after_wake = match words.iter().rposition(|w| WAKE_WORDS_FREE.contains(w)) {
        Some(i) => &words[i + 1..],
        None => &words[..],
    };
    let request: Vec<&str> = after_wake
        .iter()
        .copied()
        .skip_while(|w| GREETINGS.contains(w))
        .collect();
    (!request.is_empty()).then(|| request.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_phrase_needs_greeting_or_to_stand_alone() {
        assert!(is_wake_phrase("hé mémo", false));
        assert!(is_wake_phrase("[unk] hey memo", false));
        assert!(is_wake_phrase("mémo", true));
        assert!(!is_wake_phrase("mémo", false));
        assert!(!is_wake_phrase("[unk] mémo [unk]", true));
        assert!(!is_wake_phrase("[unk]", true));
    }

    #[test]
    fn known_commands_win_and_unknown_speech_falls_back_to_dictation() {
        assert_eq!(
            choose_request("hé mémo ouvre youtube", "aimant ouvrables youtube").as_deref(),
            Some("ouvre youtube")
        );
        assert_eq!(
            choose_request("cherche [unk]", "cherche recettes de crêpes").as_deref(),
            Some("cherche recettes de crêpes")
        );
        assert_eq!(choose_request("hé mémo", "aimant"), None);
        assert_eq!(
            choose_request("[unk] mémo ouvre youtube", "un symbole mimo ouvrit youtube").as_deref(),
            Some("ouvre youtube")
        );
        assert_eq!(choose_request("", ""), None);
    }

    #[test]
    fn command_grammar_includes_wake_prefixed_variants() {
        let grammar = with_wake_prefixes(mimo_core::voice_phrases("fr", &Default::default()), "fr");
        assert!(grammar.contains(&"ouvre youtube".to_string()));
        assert!(grammar.contains(&"hé mémo ouvre youtube".to_string()));
        assert_eq!(grammar.last().map(String::as_str), Some("[unk]"));
    }

    #[test]
    fn phrases_with_unknown_words_are_not_speakable() {
        let vocabulary: HashSet<String> = ["lance", "spotify"].map(String::from).into();
        assert!(speakable("lance spotify", &vocabulary));
        assert!(speakable("lance [unk]", &vocabulary));
        assert!(!speakable("lance capcut", &vocabulary));
    }

    #[test]
    fn request_drops_leading_wake_words() {
        assert_eq!(clean_request("mémo ouvre youtube").as_deref(), Some("ouvre youtube"));
        assert_eq!(clean_request("hey memo open netflix").as_deref(), Some("open netflix"));
        assert_eq!(clean_request("hé mémo"), None);
        assert_eq!(clean_request("[unk] mémo ouvre youtube").as_deref(), Some("ouvre youtube"));
        assert_eq!(
            clean_request("un symbole mimo ouvre youtube").as_deref(),
            Some("ouvre youtube")
        );
        assert_eq!(clean_request(""), None);
    }
}
