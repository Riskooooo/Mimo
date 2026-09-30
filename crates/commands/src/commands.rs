use std::sync::Mutex;

use mimo_core::{Engine, EngineStatus};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatusDto {
    pub running: bool,
}

impl From<EngineStatus> for EngineStatusDto {
    fn from(status: EngineStatus) -> Self {
        Self {
            running: status.running,
        }
    }
}

#[tauri::command]
pub fn engine_status(engine: State<'_, Mutex<Engine>>) -> EngineStatusDto {
    engine
        .lock()
        .expect("engine mutex poisoned")
        .status()
        .into()
}
