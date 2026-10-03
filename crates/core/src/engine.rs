use crate::error::CoreError;
use crate::apps::AppCatalog;
use crate::custom::{self, CustomAction, CustomCommand};
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
    /// The user's own commands, checked before everything else.
    custom: Vec<CustomCommand>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            running: false,
            apps: AppCatalog::default(),
            language: Lang::default(),
            custom: Vec::new(),
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
        if let Some(command) = custom::find(&self.custom, request) {
            return Ok(match &command.action {
                // Handled like the request it stands for (never another
                // custom command, so no loops).
                CustomAction::Request { text } => intent::parse_in(text, &self.apps, Some(self.language)),
                action => Intent::Custom { action: action.clone(), lang: self.language },
            });
        }
        Ok(intent::parse_in(request, &self.apps, Some(self.language)))
    }

    pub fn set_custom_commands(&mut self, commands: Vec<CustomCommand>) {
        self.custom = commands;
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
        let mut phrases = intent::voice_phrases(language, &self.apps);
        phrases.extend(self.custom.iter().map(custom::voice_phrase).filter(|p| !p.is_empty()));
        phrases
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
