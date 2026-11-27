use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What went wrong, in words a person can act on.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} isn't installed: install it and try again")]
    Missing(&'static str),
    #[error("{tool} failed: {message}")]
    Tool { tool: &'static str, message: String },
    #[error("couldn't read {}: {message}", path.display())]
    Read { path: PathBuf, message: String },
    #[error("couldn't write {}: {message}", path.display())]
    Write { path: PathBuf, message: String },
    #[error("the LLM ({name}) {message}")]
    Llm { name: String, message: String },
    #[error("transcription ({name}) {message}")]
    Transcribe { name: String, message: String },
    #[error("{0}")]
    Setup(String),
    #[error("{0}")]
    Input(String),
}

impl Error {
    pub(crate) fn read(path: impl Into<PathBuf>, e: impl std::fmt::Display) -> Self {
        Self::Read {
            path: path.into(),
            message: e.to_string(),
        }
    }

    pub(crate) fn write(path: impl Into<PathBuf>, e: impl std::fmt::Display) -> Self {
        Self::Write {
            path: path.into(),
            message: e.to_string(),
        }
    }
}
