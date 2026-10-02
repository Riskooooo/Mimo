use crate::error::CoreError;
use crate::apps::AppCatalog;
use crate::info::Lang;
use crate::intent::{self, Intent};

/// Snapshot of the engine's current state, safe to serialize and send to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineStatus {
    pub running: bool,
}

/// Entry point into system analysis and automation.
///
/// Analyzers and automation modules will register with this engine as they
/// are built; for now it only tracks whether it has been started.
#[derive(Debug, Default)]
pub struct Engine {
    running: bool,
    apps: AppCatalog,
    /// The app's language: what replies are written in.
    language: Lang,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            running: false,
            apps: AppCatalog::default(),
            language: Lang::default(),
        }
    }

    pub fn start(&mut self) -> Result<(), CoreError> {
        self.running = true;
        Ok(())
    }

    /// Decides what to do with a typed or spoken request.
    pub fn interpret(&self, request: &str) -> Result<Intent, CoreError> {
        if !self.running {
            return Err(CoreError::NotRunning);
        }
        Ok(intent::parse_in(request, &self.apps, Some(self.language)))
    }

    /// Replaces the known installed apps; `true` if the list changed.
    pub fn set_installed_apps(&mut self, apps: AppCatalog) -> bool {
        let changed = self.apps != apps;
        self.apps = apps;
        changed
    }

    pub fn language(&self) -> Lang {
        self.language
    }

    pub fn set_language(&mut self, language: Lang) {
        self.language = language;
    }

    pub fn installed_apps(&self) -> &AppCatalog {
        &self.apps
    }

    /// Phrases voice recognition should expect, installed apps included.
    pub fn voice_phrases(&self, language: &str) -> Vec<String> {
        intent::voice_phrases(language, &self.apps)
    }

    pub fn status(&self) -> EngineStatus {
        EngineStatus {
            running: self.running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_stopped() {
        assert_eq!(
            Engine::new().status(),
            EngineStatus { running: false }
        );
    }

    #[test]
    fn start_marks_running() {
        let mut engine = Engine::new();
        engine.start().unwrap();
        assert!(engine.status().running);
    }

    #[test]
    fn interpret_requires_running_engine() {
        assert!(Engine::new().interpret("open youtube").is_err());

        let mut engine = Engine::new();
        engine.start().unwrap();
        assert!(matches!(engine.interpret("open youtube"), Ok(Intent::OpenUrl { .. })));
    }
}
