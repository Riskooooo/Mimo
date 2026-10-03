//! Minimal runtime binding to `libvosk.dll`. It is loaded with `libloading`
//! instead of being linked, so Mimo builds and starts even when the voice
//! assets haven't been downloaded yet — voice wake then just reports what's
//! missing.

use std::collections::HashSet;
use std::ffi::{c_char, c_int, c_short, c_void, CStr, CString};
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;

use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};
use serde::Deserialize;

pub const MISSING_ASSETS: &str =
    "Voice files are missing. Run `npm run setup:voice` in apps/desktop, then toggle this again.";

type ModelNew = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type RecognizerNew = unsafe extern "C" fn(*mut c_void, f32) -> *mut c_void;
type RecognizerNewGrm = unsafe extern "C" fn(*mut c_void, f32, *const c_char) -> *mut c_void;
type AcceptWaveformS = unsafe extern "C" fn(*mut c_void, *const c_short, c_int) -> c_int;
type ResultFn = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type SetLogLevel = unsafe extern "C" fn(c_int);
type SetWords = unsafe extern "C" fn(*mut c_void, c_int);

/// The loaded library. Function pointers stay valid as long as `_lib` lives,
/// and every model/recognizer keeps an `Arc` to it.
pub struct Vosk {
    model_new: ModelNew,
    model_free: Free,
    recognizer_new: RecognizerNew,
    recognizer_new_grm: RecognizerNewGrm,
    recognizer_free: Free,
    recognizer_reset: Free,
    recognizer_set_words: SetWords,
    accept_waveform_s: AcceptWaveformS,
    result: ResultFn,
    partial_result: ResultFn,
    final_result: ResultFn,
    _lib: Library,
}

impl Vosk {
    pub fn load(lib_dir: &Path) -> Result<Arc<Self>, String> {
        let path = lib_dir.join("libvosk.dll");
        if !path.exists() {
            return Err(MISSING_ASSETS.to_string());
        }

        // libvosk depends on MinGW runtime DLLs shipped next to it; let the
        // loader look in its own folder for them.
        let lib = unsafe {
            Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .map_err(|err| format!("Couldn't load the speech engine: {err}"))?;

        let set_log_level: SetLogLevel = symbol(&lib, "vosk_set_log_level")?;
        // Kaldi is chatty on stderr; only keep errors.
        unsafe { set_log_level(-1) };

        Ok(Arc::new(Self {
            model_new: symbol(&lib, "vosk_model_new")?,
            model_free: symbol(&lib, "vosk_model_free")?,
            recognizer_new: symbol(&lib, "vosk_recognizer_new")?,
            recognizer_new_grm: symbol(&lib, "vosk_recognizer_new_grm")?,
            recognizer_free: symbol(&lib, "vosk_recognizer_free")?,
            recognizer_reset: symbol(&lib, "vosk_recognizer_reset")?,
            recognizer_set_words: symbol(&lib, "vosk_recognizer_set_words")?,
            accept_waveform_s: symbol(&lib, "vosk_recognizer_accept_waveform_s")?,
            result: symbol(&lib, "vosk_recognizer_result")?,
            partial_result: symbol(&lib, "vosk_recognizer_partial_result")?,
            final_result: symbol(&lib, "vosk_recognizer_final_result")?,
            _lib: lib,
        }))
    }
}

fn symbol<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
    let name = format!("{name}\0");
    unsafe { lib.get::<T>(name.as_bytes()) }
        .map(|sym| *sym)
        .map_err(|err| format!("Speech engine is missing {name}: {err}"))
}

/// A loaded acoustic + language model (a few hundred MB in memory for the
/// small models, so only the selected language is ever loaded).
pub struct Model {
    vosk: Arc<Vosk>,
    ptr: *mut c_void,
}

impl Model {
    pub fn load(vosk: &Arc<Vosk>, dir: &Path) -> Result<Self, String> {
        if !dir.join("am").exists() {
            return Err(MISSING_ASSETS.to_string());
        }
        let path = CString::new(narrow_path(dir)).map_err(|_| "Invalid voice model path.".to_string())?;
        let ptr = unsafe { (vosk.model_new)(path.as_ptr()) };
        if ptr.is_null() {
            return Err(format!("Couldn't load the voice model in {}.", dir.display()));
        }
        Ok(Self { vosk: vosk.clone(), ptr })
    }
}

/// libvosk opens files with narrow (ANSI code page) paths, so a folder
/// with accents — the install folder under C:\Users\Hélène — wouldn't
/// load. Such paths are passed in their 8.3 short form, which is ASCII.
pub(crate) fn narrow_path(dir: &Path) -> String {
    let full = dir.to_string_lossy().to_string();
    if full.is_ascii() {
        return full;
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut buffer = vec![0u16; 1024];
    let len = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len > buffer.len() {
        return full;
    }
    String::from_utf16_lossy(&buffer[..len])
}

#[link(name = "kernel32")]
extern "system" {
    fn GetShortPathNameW(long_path: *const u16, short_path: *mut u16, len: u32) -> u32;
}

impl Drop for Model {
    fn drop(&mut self) {
        // Recognizers hold their own reference to the model inside Vosk, so
        // freeing it here is safe even if one outlives this handle.
        unsafe { (self.vosk.model_free)(self.ptr) };
    }
}

/// The words a model can recognize, read from the symbol table in its
/// `graph/Gr.fst` (OpenFST binary header). Vosk silently drops unknown
/// words from a grammar phrase — "lance capcut" would become just "lance" —
/// so phrases are checked against this first. `None` if the file isn't in
/// the expected format (then nothing gets filtered).
pub fn vocabulary(model_dir: &Path) -> Option<HashSet<String>> {
    const FST_MAGIC: i32 = 2_125_659_606;
    const SYMBOL_TABLE_MAGIC: i32 = 2_125_658_996;
    const HAS_INPUT_SYMBOLS: i32 = 1;

    let mut r = BufReader::new(File::open(model_dir.join("graph").join("Gr.fst")).ok()?);
    let i32_ = |r: &mut BufReader<File>| -> Option<i32> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b).ok()?;
        Some(i32::from_le_bytes(b))
    };
    let i64_ = |r: &mut BufReader<File>| -> Option<i64> {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).ok()?;
        Some(i64::from_le_bytes(b))
    };
    let string = |r: &mut BufReader<File>, len: i32| -> Option<String> {
        let mut b = vec![0u8; usize::try_from(len).ok()?];
        r.read_exact(&mut b).ok()?;
        String::from_utf8(b).ok()
    };

    // Header: magic, fst type, arc type, version, flags, properties,
    // start state, state count, arc count — then the input symbol table.
    if i32_(&mut r)? != FST_MAGIC {
        return None;
    }
    for _ in 0..2 {
        let len = i32_(&mut r)?;
        string(&mut r, len)?;
    }
    let _version = i32_(&mut r)?;
    let flags = i32_(&mut r)?;
    for _ in 0..4 {
        i64_(&mut r)?;
    }
    if flags & HAS_INPUT_SYMBOLS == 0 || i32_(&mut r)? != SYMBOL_TABLE_MAGIC {
        return None;
    }
    let name_len = i32_(&mut r)?;
    string(&mut r, name_len)?;
    let _available_key = i64_(&mut r)?;
    let size = usize::try_from(i64_(&mut r)?).ok()?;

    let mut words = HashSet::with_capacity(size);
    for _ in 0..size {
        let len = i32_(&mut r)?;
        words.insert(string(&mut r, len)?);
        i64_(&mut r)?; // symbol id
    }
    Some(words)
}

pub struct Recognizer {
    vosk: Arc<Vosk>,
    ptr: *mut c_void,
}

#[derive(Deserialize, Default)]
struct Output {
    #[serde(default)]
    text: String,
    #[serde(default)]
    partial: String,
    #[serde(default)]
    result: Vec<Word>,
}

/// One recognized word with its timing (seconds since the recognizer was
/// last reset) and confidence (0–1), from [`Recognizer::finish_words`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Word {
    pub word: String,
    pub start: f32,
    pub end: f32,
    pub conf: f32,
}

impl Recognizer {
    /// Free-form recognition of anything the model knows.
    pub fn free(model: &Model, sample_rate: f32) -> Result<Self, String> {
        let ptr = unsafe { (model.vosk.recognizer_new)(model.ptr, sample_rate) };
        Self::wrap(model, ptr)
    }

    /// Recognition restricted to the given phrases (plus `"[unk]"` for
    /// anything else) — cheaper, and far less prone to wrong guesses.
    pub fn with_grammar<S: AsRef<str>>(model: &Model, sample_rate: f32, phrases: &[S]) -> Result<Self, String> {
        let phrases: Vec<&str> = phrases.iter().map(AsRef::as_ref).collect();
        let json = serde_json::to_string(&phrases).expect("phrases serialize");
        let json = CString::new(json).map_err(|_| "Invalid grammar.".to_string())?;
        let ptr = unsafe { (model.vosk.recognizer_new_grm)(model.ptr, sample_rate, json.as_ptr()) };
        Self::wrap(model, ptr)
    }

    fn wrap(model: &Model, ptr: *mut c_void) -> Result<Self, String> {
        if ptr.is_null() {
            return Err("Couldn't start speech recognition.".to_string());
        }
        Ok(Self { vosk: model.vosk.clone(), ptr })
    }

    /// Feeds mono 16-bit samples; `true` once an utterance just ended
    /// (its text is then available from [`Recognizer::text`]).
    pub fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        let len = c_int::try_from(samples.len()).map_err(|_| "Audio chunk too large.".to_string())?;
        match unsafe { (self.vosk.accept_waveform_s)(self.ptr, samples.as_ptr(), len) } {
            -1 => Err("Speech recognition failed.".to_string()),
            ended => Ok(ended == 1),
        }
    }

    /// Text of the utterance that just ended.
    pub fn text(&mut self) -> String {
        self.read(self.vosk.result).text
    }

    /// Best guess so far for the utterance in progress.
    pub fn partial(&mut self) -> String {
        self.read(self.vosk.partial_result).partial
    }

    /// Ends the utterance in progress now and returns its text.
    pub fn finish(&mut self) -> String {
        self.read(self.vosk.final_result).text
    }

    /// Ends the utterance in progress now and returns its words, with
    /// timings and confidences. Needs [`Recognizer::with_word_details`].
    pub fn finish_words(&mut self) -> Vec<Word> {
        self.read(self.vosk.final_result).result
    }

    /// Makes results carry per-word timings and confidences.
    pub fn with_word_details(self) -> Self {
        unsafe { (self.vosk.recognizer_set_words)(self.ptr, 1) };
        self
    }

    pub fn reset(&mut self) {
        unsafe { (self.vosk.recognizer_reset)(self.ptr) };
    }

    fn read(&mut self, getter: ResultFn) -> Output {
        // The returned string is owned by the recognizer and only valid
        // until the next call, so it's parsed immediately.
        let raw = unsafe { getter(self.ptr) };
        if raw.is_null() {
            return Output::default();
        }
        let json = unsafe { CStr::from_ptr(raw) }.to_string_lossy();
        serde_json::from_str(&json).unwrap_or_default()
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        unsafe { (self.vosk.recognizer_free)(self.ptr) };
    }
}
