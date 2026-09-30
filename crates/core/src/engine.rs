use crate::error::CoreError;

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
}

impl Engine {
    pub fn new() -> Self {
        Self { running: false }
    }

    pub fn start(&mut self) -> Result<(), CoreError> {
        self.running = true;
        Ok(())
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
}
