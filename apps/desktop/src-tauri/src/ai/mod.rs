//! The local AI (setting `ai_enabled`, off by default). Turning it on
//! downloads the engine and model once, in the background, into
//! `<app data>/ai` (progress: `mimo://ai-status`); after that everything
//! runs on the PC. The engine is started on first use (or when the pill is
//! summoned, to be warm by the time the request comes) and stopped after
//! [`IDLE_STOP`] unused, which frees its memory.

mod download;
mod server;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mimo_core::ai::Message;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use self::server::Server;

const IDLE_STOP: Duration = Duration::from_secs(10 * 60);
const REAP_EVERY: Duration = Duration::from_secs(30);

/// What the settings show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum AiStatus {
    /// Turned off; `installed`: the files are on disk (can be deleted).
    Off { installed: bool, size: u64 },
    Downloading { done: u64, total: u64 },
    Ready,
    Error { message: String },
}

struct State {
    enabled: bool,
    status: AiStatus,
    downloading: bool,
    last_used: Instant,
}

struct Shared {
    app: AppHandle,
    dir: Option<PathBuf>,
    state: Mutex<State>,
    /// Also serializes starting it and the requests (one at a time).
    server: Mutex<Option<Server>>,
    cancel: AtomicBool,
}

pub struct Ai {
    shared: Arc<Shared>,
}

impl Ai {
    pub fn new(app: &AppHandle, enabled: bool) -> Self {
        let dir = app.path().app_data_dir().ok().map(|dir| dir.join("ai"));
        let installed = dir.as_deref().is_some_and(download::installed);
        let status = off_status(installed);
        let shared = Arc::new(Shared {
            app: app.clone(),
            dir,
            state: Mutex::new(State { enabled: false, status, downloading: false, last_used: Instant::now() }),
            server: Mutex::new(None),
            cancel: AtomicBool::new(false),
        });
        let ai = Self { shared };
        ai.start_reaper();
        if enabled {
            ai.set_enabled(true);
        }
        ai
    }

    pub fn status(&self) -> AiStatus {
        self.shared.state.lock().expect("ai mutex poisoned").status.clone()
    }

    /// On and fully downloaded.
    pub fn usable(&self) -> bool {
        let state = self.shared.state.lock().expect("ai mutex poisoned");
        state.enabled && state.status == AiStatus::Ready
    }

    pub fn is_downloading(&self) -> bool {
        self.shared.state.lock().expect("ai mutex poisoned").downloading
    }

    pub fn set_enabled(&self, enabled: bool) {
        let Some(dir) = self.shared.dir.clone() else { return };
        let mut state = self.shared.state.lock().expect("ai mutex poisoned");
        state.enabled = enabled;
        if !enabled {
            self.shared.cancel.store(true, Ordering::SeqCst);
            drop(state);
            self.stop();
            let status = off_status(download::installed(&dir));
            self.set_status(status);
            return;
        }
        if download::installed(&dir) {
            drop(state);
            self.set_status(AiStatus::Ready);
            return;
        }
        if state.downloading {
            return;
        }
        state.downloading = true;
        drop(state);
        self.shared.cancel.store(false, Ordering::SeqCst);
        self.set_status(AiStatus::Downloading { done: 0, total: download::total_size() });

        let shared = self.shared.clone();
        let _ = std::thread::Builder::new().name("mimo-ai-download".into()).spawn(move || {
            let ai = Ai { shared };
            let result = download::install(&dir, &ai.shared.cancel, |done, total| {
                ai.set_status(AiStatus::Downloading { done, total });
            });
            ai.shared.state.lock().expect("ai mutex poisoned").downloading = false;
            let enabled = ai.shared.state.lock().expect("ai mutex poisoned").enabled;
            if matches!(result, Err(download::Failure::Cancelled)) && enabled {
                // Turned off then on again before the download noticed.
                ai.set_enabled(true);
                return;
            }
            let status = match result {
                Ok(()) if enabled => AiStatus::Ready,
                Ok(()) => off_status(true),
                Err(download::Failure::Cancelled) => off_status(download::installed(&dir)),
                Err(download::Failure::Failed(err)) => {
                    eprintln!("[ai] {err}");
                    AiStatus::Error { message: err }
                }
            };
            ai.set_status(status);
        });
    }

    /// Deletes the downloaded files (the AI must be off).
    pub fn delete_files(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
        self.stop();
        if let Some(dir) = &self.shared.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        let enabled = self.shared.state.lock().expect("ai mutex poisoned").enabled;
        if !enabled {
            self.set_status(off_status(false));
        }
    }

    /// Stops the engine and any download (quitting, erasing memory).
    pub fn shutdown(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
        self.stop();
    }

    fn stop(&self) {
        *self.shared.server.lock().expect("ai server mutex poisoned") = None;
    }

    /// Starts the engine in the background if it's usable and not running,
    /// so it's loaded by the time a request needs it.
    pub fn warm_up(&self) {
        if !self.usable() {
            return;
        }
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let ai = Ai { shared };
            let mut server = ai.shared.server.lock().expect("ai server mutex poisoned");
            if let Err(err) = ai.ensure_started(&mut server) {
                eprintln!("[ai] {err}");
            }
        });
    }

    /// One request to the model (starting the engine if needed). Blocking.
    pub fn chat(&self, messages: &[Message], schema: Option<Value>, max_tokens: u32, timeout: Duration) -> Result<String, String> {
        if !self.usable() {
            return Err("the local AI is off or not downloaded".to_string());
        }
        let mut server = self.shared.server.lock().expect("ai server mutex poisoned");
        self.ensure_started(&mut server)?;
        let result = server.as_ref().expect("server just started").chat(messages, schema, max_tokens, timeout);
        self.shared.state.lock().expect("ai mutex poisoned").last_used = Instant::now();
        result
    }

    fn ensure_started(&self, server: &mut Option<Server>) -> Result<(), String> {
        if server.as_mut().is_some_and(Server::alive) {
            return Ok(());
        }
        let dir = self.shared.dir.as_ref().ok_or("no app data folder")?;
        *server = Some(Server::start(dir)?);
        self.shared.state.lock().expect("ai mutex poisoned").last_used = Instant::now();
        Ok(())
    }

    fn set_status(&self, status: AiStatus) {
        let mut state = self.shared.state.lock().expect("ai mutex poisoned");
        if state.status == status {
            return;
        }
        state.status = status.clone();
        drop(state);
        let _ = self.shared.app.emit("mimo://ai-status", status);
    }

    /// Stops the engine once it has gone unused for a while.
    fn start_reaper(&self) {
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new().name("mimo-ai-idle".into()).spawn(move || loop {
            std::thread::sleep(REAP_EVERY);
            let idle = shared.state.lock().expect("ai mutex poisoned").last_used.elapsed() >= IDLE_STOP;
            if idle {
                // Not while a request holds it.
                if let Ok(mut server) = shared.server.try_lock() {
                    if server.take().is_some() {
                        eprintln!("[ai] engine stopped (unused)");
                    }
                }
            }
        });
    }
}

fn off_status(installed: bool) -> AiStatus {
    AiStatus::Off { installed, size: download::total_size() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimo_core::ai::{parse_understanding, recap_prompt, understanding_prompt, understanding_schema};
    use mimo_core::Lang;

    /// Downloads the AI into the real app data folder (resumable), starts
    /// the engine and asks a few things: `cargo test -p desktop
    /// ai_end_to_end -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ai_end_to_end() {
        let dir = PathBuf::from(std::env::var("APPDATA").unwrap()).join("com.fabian.mimo").join("ai");
        let cancel = AtomicBool::new(false);
        let mut last = 0;
        let result = download::install(&dir, &cancel, |done, total| {
            let percent = done * 100 / total;
            if percent != last {
                last = percent;
                println!("download {percent} % ({} / {} MB)", done / 1_000_000, total / 1_000_000);
            }
        });
        assert!(result.is_ok(), "download failed");
        assert!(download::installed(&dir));

        let server = Server::start(&dir).expect("engine");
        let timeout = Duration::from_secs(120);
        for request in [
            "j'entends rien là",
            "c'est beaucoup trop fort",
            "je capte pas ce que dit la vidéo, c'est trop bas",
            "mets-moi un truc chill sur youtube",
            "faut que je pense à appeler mamie ce soir à 19h",
            "c'est quoi la photosynthèse ?",
            "donne-moi une idée de repas rapide",
            "il fait trop sombre sur mon écran",
            "j'ai sommeil",
            "lance moi un truc pour écouter de la musique",
        ] {
            let output = server
                .chat(&understanding_prompt(request, Lang::Fr, "lundi 05/10/2026, 14:00"), Some(understanding_schema()), 300, timeout)
                .expect("chat");
            println!("{request:?} → {:?}", parse_understanding(&output));
        }
        let recap = server
            .chat(&recap_prompt("Aujourd'hui : 2 h 30 d'activité, surtout sur Discord (2 h 03) et VS Code (27 min).", Lang::Fr), None, 200, timeout)
            .expect("recap");
        println!("recap → {recap}");
    }
}
