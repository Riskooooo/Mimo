//! "Hey Mimo" voice wake — fully offline, on Vosk with the small French and
//! English models from `resources/vosk` (fetched by
//! `scripts/fetch-voice-models.ps1`). No audio or text ever leaves the PC.
//!
//! While enabled, one background thread owns the microphone:
//! - a grammar-restricted recognizer (only the wake phrase, plus "[unk]"
//!   for everything else) listens continuously — cheap, but too eager;
//! - as soon as it hears the wake word, even mid-sentence (via partial
//!   results), the last few seconds of audio are decoded again to make sure
//!   (see [`WakeCheck`]) — that small grammar snaps everyday speech onto
//!   "hé mémo" far too easily ("et mes mots", "eh mais mon"…);
//! - once confirmed, the audio that follows goes to two recognizers until that
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
use self::vosk::{vocabulary, Model, Recognizer, Vosk, Word};

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

/// Recent audio decoded again to confirm a wake candidate: the wake phrase
/// plus enough of what came before it to tell whether there was a pause.
const WAKE_HISTORY: Duration = Duration::from_millis(2500);

/// Confidence the wake word needs once the whole audio is decoded. Only a
/// safety net: with a grammar this small, Vosk is close to 1.0 even on the
/// false wakes it's meant to catch.
const MIN_WAKE_CONF: f32 = 0.8;
/// Confidence for "hé mémo" after a pause to count without the free-form
/// recognizer hearing the name.
const CLEAR_WAKE_CONF: f32 = 0.9;
/// Silence (seconds) before the wake phrase for it to count as addressing
/// Mimo rather than part of a sentence.
const PAUSE_BEFORE_WAKE: f32 = 0.3;

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
/// What free-form recognition writes for a real "mimo", when confirming a
/// wake. These aren't words people use otherwise…
const WAKE_NAMES_FREE: &[&str] = &["mimo", "immo", "mimos"];
/// …unlike these ("did you get my memo"), which only count right after the
/// greeting or a pause.
const WAKE_LOOKALIKES_FREE: &[&str] = &["mémo", "memo", "mémos", "memos", "némo", "nemo"];
/// Everyday speech that the wake grammar hears as "hé mémo": words that
/// explain the "mémo" on their own ("hey, memory")…
const WAKE_SOUNDALIKE_WORDS_FREE: &[&str] =
    &["mémoire", "mémorise", "mémoriser", "memory", "member", "remember"];
/// …and the start of a sentence ("et mes mots", "eh mais mon"). Only pairs:
/// a real voice's "mimo" comes out as anything ("ennemis", "elle mimant",
/// "animaux", "mille mots", "hey my man"), single short words included.
const WAKE_SOUNDALIKE_PAIRS_FREE: &[(&str, &str)] = &[
    ("et", "mes"),
    ("et", "mon"),
    ("et", "mais"),
    ("et", "même"),
    ("mes", "mots"),
    ("mais", "mon"),
    ("mais", "non"),
    ("me", "more"),
    ("maybe", "more"),
];
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
    let mut checker = WakeChecker {
        wake: Recognizer::with_grammar(model, rate, wake_grammar(language))?.with_word_details(),
        dictation: Recognizer::free(model, rate)?,
    };
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
    let mut recent = AudioHistory::new((WAKE_HISTORY.as_secs_f32() * rate) as usize);

    while current_generation(shared) == generation {
        let Some(chunk) = mic.next(AUDIO_POLL)? else {
            continue;
        };
        let ended = wake.accept(&chunk)?;
        recent.push(chunk);
        let heard = if ended { wake.text() } else { wake.partial() };
        if !mentions_wake_word(&heard) {
            continue;
        }

        wake.reset();
        let checking = Instant::now();
        let check = checker.check(&recent.snapshot());
        let verdict = if check.as_ref().is_some_and(WakeCheck::accepted) { "heard" } else { "rejected" };
        eprintln!("[voice] wake {verdict}: {heard:?} {check:?} in {:?}", checking.elapsed());
        if verdict == "rejected" {
            continue;
        }
        crate::summon(app, "voice");

        let text = listen_for_request(&mic, &mut request, recent.take_last(pre_roll_samples))?;
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

    fn snapshot(&self) -> Vec<Vec<i16>> {
        self.chunks.iter().cloned().collect()
    }

    /// Empties the history, returning about its last `samples` samples.
    fn take_last(&mut self, samples: usize) -> Vec<Vec<i16>> {
        let mut kept = 0;
        let mut taken: Vec<Vec<i16>> = Vec::new();
        while kept < samples {
            let Some(chunk) = self.chunks.pop_back() else { break };
            kept += chunk.len();
            taken.push(chunk);
        }
        self.chunks.clear();
        self.samples = 0;
        taken.reverse();
        taken
    }
}

/// Decodes a wake candidate's recent audio again, from scratch and to the
/// end, rather than trusting a partial guess.
struct WakeChecker {
    /// Same grammar as the always-on recognizer, with word timings and
    /// confidences.
    wake: Recognizer,
    /// Free-form, so it isn't forced to pick "mémo" for anything close.
    dictation: Recognizer,
}

impl WakeChecker {
    fn check(&mut self, audio: &[Vec<i16>]) -> Option<WakeCheck> {
        self.wake.reset();
        for chunk in audio {
            // Its words are all read back at once below.
            self.wake.accept(chunk).ok()?;
        }
        let words = self.wake.finish_words();
        // Free-form decoding costs ~10× more: skip it when the answer is
        // already no. It hears all the audio, though — given only the wake
        // phrase, without what led to it, it guesses wildly.
        let (wake, _) = wake_phrase(&words)?;
        if words[wake].conf < MIN_WAKE_CONF {
            return WakeCheck::from_decoding(&words, "");
        }

        self.dictation.reset();
        let mut dictated = String::new();
        for chunk in audio {
            if self.dictation.accept(chunk).ok()? {
                dictated.push(' ');
                dictated.push_str(&self.dictation.text());
            }
        }
        dictated.push(' ');
        dictated.push_str(&self.dictation.finish());
        eprintln!("[voice] wake check heard: {:?}", dictated.trim());
        WakeCheck::from_decoding(&words, &dictated)
    }
}

/// Indices of the last wake word and of where its phrase starts (the
/// greeting, if any).
fn wake_phrase(words: &[Word]) -> Option<(usize, usize)> {
    let wake = words.iter().rposition(|w| WAKE_WORDS.contains(&w.word.as_str()))?;
    let greeting = wake.checked_sub(1).filter(|&g| GREETINGS.contains(&words[g].word.as_str()));
    Some((wake, greeting.unwrap_or(wake)))
}

/// Evidence that a wake candidate really is someone calling Mimo.
#[derive(Debug, Clone, PartialEq)]
struct WakeCheck {
    /// The wake word's confidence after decoding the whole audio.
    confidence: f32,
    /// Said as "hé mémo", not a bare "mémo".
    greeting: bool,
    /// The wake phrase came after a pause, not in the middle of speech.
    pause_before: bool,
    /// The free-form recognizer heard "mimo" too…
    named: bool,
    /// …or a real word that sounds like it ("memo")…
    lookalike: bool,
    /// It heard everyday words that sound like the wake phrase instead.
    soundalike: bool,
}

impl WakeCheck {
    /// `None` if the full decoding doesn't hold a wake word at all.
    fn from_decoding(words: &[Word], dictated: &str) -> Option<Self> {
        let (i, phrase_start) = wake_phrase(words)?;
        let greeting = phrase_start < i;
        let start = words[phrase_start].start;
        let pause_before = match phrase_start.checked_sub(1) {
            Some(prev) => start - words[prev].end >= PAUSE_BEFORE_WAKE,
            // Nothing decoded before it: a pause, if the audio reaches back
            // far enough to tell.
            None => start >= PAUSE_BEFORE_WAKE,
        };
        let dictated: Vec<&str> = dictated.split_whitespace().collect();
        Some(Self {
            confidence: words[i].conf,
            greeting,
            pause_before,
            named: dictated.iter().any(|word| WAKE_NAMES_FREE.contains(word)),
            lookalike: dictated.iter().any(|word| WAKE_LOOKALIKES_FREE.contains(word)),
            soundalike: dictated.iter().any(|word| WAKE_SOUNDALIKE_WORDS_FREE.contains(word))
                || dictated
                    .windows(2)
                    .any(|pair| WAKE_SOUNDALIKE_PAIRS_FREE.contains(&(pair[0], pair[1]))),
        })
    }

    /// Either the free-form recognizer, which isn't limited to the wake
    /// phrase, heard the name too ("memo" — a real word — only counts with
    /// the greeting or a pause before it), or "hé mémo" was said clearly
    /// after a pause and the free-form recognizer didn't hear everyday
    /// words instead ("eh mais mon pote" after a silence).
    fn accepted(&self) -> bool {
        let addressed = self.greeting && self.pause_before && self.confidence >= CLEAR_WAKE_CONF;
        self.confidence >= MIN_WAKE_CONF
            && (self.named
                || (self.lookalike && (self.greeting || self.pause_before))
                || (addressed && !self.soundalike))
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

/// Worth checking with [`WakeChecker`]: the greeting is often heard as
/// "[unk]", so any wake word is a candidate.
fn mentions_wake_word(text: &str) -> bool {
    text.split_whitespace().any(|word| WAKE_WORDS.contains(&word))
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
    fn any_wake_word_is_a_candidate() {
        assert!(mentions_wake_word("hé mémo"));
        assert!(mentions_wake_word("[unk] mémo [unk]"));
        assert!(mentions_wake_word("hey memo"));
        assert!(!mentions_wake_word("[unk] hé"));
        assert!(!mentions_wake_word(""));
    }

    fn word(word: &str, start: f32, end: f32, conf: f32) -> Word {
        Word { word: word.to_string(), start, end, conf }
    }

    // The decodings below are real ones, of synthesized speech (see
    // `replay_wake`).

    #[test]
    fn wake_heard_as_mimo_is_accepted() {
        // "Bon je vais regarder ça… Hé Mimo, quelle heure est-il ?": the
        // greeting comes out as "[unk]", glued to the wake word.
        let words = [word("[unk]", 3.52, 4.74, 1.0), word("mémo", 4.77, 5.25, 1.0)];
        let check = WakeCheck::from_decoding(&words, "à symbole copyright mimo quelle heure est-il").unwrap();
        assert!(!check.greeting && !check.pause_before && check.named);
        assert!(check.accepted());
    }

    #[test]
    fn wake_heard_as_memo_needs_greeting_or_pause() {
        // "OK let me check that… Hey Mimo, what time is it?"
        let words = [word("[unk]", 0.1, 1.2, 1.0), word("hey", 2.0, 2.2, 1.0), word("memo", 2.2, 2.6, 1.0)];
        let check = WakeCheck::from_decoding(&words, "okay let me check that hey memo what time is it").unwrap();
        assert!(check.greeting && check.pause_before && check.lookalike);
        assert!(check.accepted());
        // "Did you get my memo about the raid tonight?"
        let words = [word("[unk]", 0.1, 0.8, 1.0), word("memo", 0.8, 1.1, 1.0), word("[unk]", 1.1, 2.4, 1.0)];
        let check = WakeCheck::from_decoding(&words, "did you get my memo about the raid tonight").unwrap();
        assert!(!check.greeting && !check.pause_before && check.lookalike);
        assert!(!check.accepted());
    }

    #[test]
    fn clear_call_after_a_pause_is_accepted_however_dictation_spells_it() {
        // How free-form recognition heard real "hé Mimo" calls by the dev.
        let words = [word("[unk]", 0.0, 0.6, 1.0), word("hey", 1.2, 1.4, 1.0), word("mémo", 1.4, 1.8, 0.96)];
        for heard in ["ennemis", "elle mimant", "animaux", "hum hum", "mille mots", "hey my man"] {
            let check = WakeCheck::from_decoding(&words, heard).unwrap();
            assert!(!check.named && !check.lookalike && !check.soundalike, "{heard}");
            assert!(check.accepted(), "{heard}");
        }
        // Not without the pause, nor with a hesitant wake word.
        let words = [word("[unk]", 0.0, 1.15, 1.0), word("hey", 1.2, 1.4, 1.0), word("mémo", 1.4, 1.8, 0.96)];
        assert!(!WakeCheck::from_decoding(&words, "elle mimant").unwrap().accepted());
        let words = [word("[unk]", 0.0, 0.6, 1.0), word("hey", 1.2, 1.4, 1.0), word("mémo", 1.4, 1.8, 0.85)];
        assert!(!WakeCheck::from_decoding(&words, "elle mimant").unwrap().accepted());
        // A sentence starting after a silence with words that sound alike.
        let words = [word("hé", 1.0, 1.2, 1.0), word("mémo", 1.2, 1.6, 1.0)];
        for heard in ["et mais mon pote", "et mes mots ne suffisent pas", "hey maybe more people"] {
            let check = WakeCheck::from_decoding(&words, heard).unwrap();
            assert!(check.soundalike && !check.accepted(), "{heard}");
        }
    }

    #[test]
    fn everyday_speech_is_rejected() {
        // "Franchement, et mes mots ne suffisent pas…", which used to wake Mimo.
        let words = [
            word("[unk]", 0.12, 0.9, 1.0),
            word("hé", 1.02, 1.14, 0.49),
            word("mémo", 1.14, 1.5, 1.0),
            word("[unk]", 2.15, 5.01, 1.0),
        ];
        let check = WakeCheck::from_decoding(&words, "franchement et mes mots ne suffisent pas").unwrap();
        assert!(!check.accepted());
        // "Yeah I told him, hey, memory is the problem": greeting and pause,
        // but the free-form recognizer heard "memory".
        let words = [word("[unk]", 0.1, 1.0, 1.0), word("hey", 1.4, 1.6, 1.0), word("memo", 1.6, 1.9, 1.0)];
        let check = WakeCheck::from_decoding(&words, "yeah i told him hey memory is the problem").unwrap();
        assert!(check.greeting && check.pause_before && check.soundalike);
        assert!(!check.accepted());
    }

    #[test]
    fn unsure_or_vanished_wake_is_rejected() {
        let words = [word("hé", 1.0, 1.2, 0.5), word("mémo", 1.2, 1.6, 0.4)];
        assert!(!WakeCheck::from_decoding(&words, "et mimo").unwrap().accepted());
        assert!(WakeCheck::from_decoding(&words, "et mimo").unwrap().named);
        // The partial guess said "mémo", the full decoding doesn't.
        assert_eq!(WakeCheck::from_decoding(&[word("[unk]", 0.0, 2.0, 1.0)], "et mes mots"), None);
        // Phrase right at the start of the audio: can't tell there was a pause.
        let words = [word("hé", 0.05, 0.1, 1.0), word("mémo", 0.1, 0.5, 1.0)];
        assert!(!WakeCheck::from_decoding(&words, "").unwrap().pause_before);
    }

    /// Replays 16 kHz mono 16-bit WAV files through the wake detection, as
    /// the live loop does, to tune the thresholds on real audio:
    /// `MIMO_WAKE_WAVS=<dir> cargo test --release -p desktop replay_wake -- --ignored --nocapture`
    /// (`MIMO_WAKE_LANG=en` for the English model, `MIMO_WAKE_DEBUG=1` to
    /// also print the raw decodings). Unlike live, the audio after a wake
    /// isn't consumed by a request, so a few extra candidates follow it.
    #[test]
    #[ignore]
    fn replay_wake() {
        let dir = PathBuf::from(std::env::var("MIMO_WAKE_WAVS").expect("set MIMO_WAKE_WAVS"));
        let language = std::env::var("MIMO_WAKE_LANG").unwrap_or_else(|_| "fr".to_string());
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/vosk");
        let vosk = Vosk::load(&assets.join("lib")).unwrap();
        let model = Model::load(&vosk, &assets.join("models").join(&language)).unwrap();
        let rate = 16_000.0;

        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
            .collect();
        paths.sort();
        // Shared, as in the live loop (the first check warms it up).
        let mut checker = WakeChecker {
            wake: Recognizer::with_grammar(&model, rate, wake_grammar(&language)).unwrap().with_word_details(),
            dictation: Recognizer::free(&model, rate).unwrap(),
        };
        for path in paths {
            let samples = read_wav(&path);
            let mut wake = Recognizer::with_grammar(&model, rate, wake_grammar(&language)).unwrap();
            let mut recent = AudioHistory::new((WAKE_HISTORY.as_secs_f32() * rate) as usize);
            let mut verdicts = Vec::new();
            for chunk in samples.chunks(4000) {
                let ended = wake.accept(chunk).unwrap();
                recent.push(chunk.to_vec());
                let heard = if ended { wake.text() } else { wake.partial() };
                if !mentions_wake_word(&heard) {
                    continue;
                }
                wake.reset();
                let checking = Instant::now();
                let check = checker.check(&recent.snapshot());
                let accepted = check.as_ref().is_some_and(WakeCheck::accepted);
                let verdict = if accepted { "WAKE" } else { "reject" };
                verdicts.push(format!("{verdict} {heard:?} {check:?} in {:?}", checking.elapsed()));
                if accepted {
                    recent.take_last(0);
                }
            }
            let name = path.file_name().unwrap().to_string_lossy();
            if std::env::var("MIMO_WAKE_DEBUG").is_ok() {
                // What each recognizer makes of the whole file.
                let mut free = Recognizer::free(&model, rate).unwrap();
                let mut grammar =
                    Recognizer::with_grammar(&model, rate, wake_grammar(&language)).unwrap().with_word_details();
                for chunk in samples.chunks(4000) {
                    if free.accept(chunk).unwrap() {
                        println!("{name}: free {:?}", free.text());
                    }
                    grammar.accept(chunk).unwrap();
                }
                println!("{name}: free {:?}", free.finish());
                println!("{name}: grammar {:?}", grammar.finish_words());
            }
            if verdicts.is_empty() {
                println!("{name}: no candidate");
            }
            for verdict in verdicts {
                println!("{name}: {verdict}");
            }
        }
    }

    /// Loads the French model from a copy in a folder with accents (like an
    /// install under C:/Users/Hélène), as Vosk would see it once installed.
    #[test]
    #[ignore = "copies a voice model (~66 MB)"]
    fn loads_a_model_from_an_accented_folder() {
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &to.join(entry.file_name()));
                } else {
                    std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
                }
            }
        }
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/vosk");
        let accented = std::env::temp_dir().join("mimo-test-Hélène").join("modèle");
        copy(&assets.join("models/fr"), &accented);
        let vosk = Vosk::load(&assets.join("lib")).unwrap();
        println!("short path: {}", vosk::narrow_path(&accented));
        let loaded = Model::load(&vosk, &accented);
        let _ = std::fs::remove_dir_all(accented.parent().unwrap());
        assert!(loaded.is_ok(), "{:?}", loaded.err());
    }

    fn read_wav(path: &Path) -> Vec<i16> {
        let bytes = std::fs::read(path).unwrap();
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let id = &bytes[at..at + 4];
            let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            if id == b"data" {
                let data = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
                return data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
            }
            at += 8 + len + (len & 1);
        }
        panic!("no data chunk in {}", path.display());
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
