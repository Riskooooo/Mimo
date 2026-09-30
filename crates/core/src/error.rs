use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("the engine is not running")]
    NotRunning,
}
