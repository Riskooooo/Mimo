use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("the engine is not running")]
    NotRunning,
    #[error("invalid shortcut: {0}")]
    InvalidShortcut(String),
    #[error("no voice model for language {0:?}")]
    UnsupportedLanguage(String),
    #[error("invalid color: {0}")]
    InvalidColor(String),
}
