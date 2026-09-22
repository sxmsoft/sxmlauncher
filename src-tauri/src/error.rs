//! Central error type for every fallible backend path.
//!
//! Rules of the codebase:
//! * No `unwrap()` / `expect()` outside of tests and provably-infallible
//!   constructors. Everything returns [`AppResult`].
//! * Errors cross the Tauri IPC boundary as `{ code, message }` so the
//!   TypeScript layer can branch on `code` instead of string matching.
//! * Third-party error types are converted into a *domain* variant that keeps
//!   enough context to be shown to a user without leaking internals.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Stable machine-readable error codes surfaced to the frontend.
pub const CODE_NETWORK: &str = "NETWORK";
pub const CODE_IO: &str = "IO";
pub const CODE_SERIALIZATION: &str = "SERIALIZATION";
pub const CODE_DATABASE: &str = "DATABASE";
pub const CODE_ACCOUNT: &str = "ACCOUNT";
pub const CODE_UNAUTHORIZED: &str = "UNAUTHORIZED";
pub const CODE_INSTANCE_NOT_FOUND: &str = "INSTANCE_NOT_FOUND";
pub const CODE_MOD_RESOLUTION: &str = "MOD_RESOLUTION";
pub const CODE_JAVA: &str = "JAVA";
pub const CODE_TRANSPORT: &str = "TRANSPORT";
pub const CODE_DIRECTORY: &str = "DIRECTORY";
pub const CODE_HASH_MISMATCH: &str = "HASH_MISMATCH";
pub const CODE_CONFIG: &str = "CONFIG";
pub const CODE_UNSUPPORTED: &str = "UNSUPPORTED";
pub const CODE_OTHER: &str = "OTHER";
/// A job was stopped by the user (never a bug, never something to retry).
pub const CODE_CANCELLED: &str = "CANCELLED";

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("network request failed: {0}")]
    Network(String),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(String),

    #[error("account error: {0}")]
    Account(String),

    #[error("sign-in required")]
    Unauthorized,

    #[error("instance not found: {0}")]
    InstanceNotFound(String),

    #[error("mod resolution failed: {0}")]
    ModResolution(String),

    #[error("java runtime error: {0}")]
    Java(String),

    #[error("p2p transport error: {0}")]
    Transport(String),

    #[error("directory/signaling error: {0}")]
    Directory(String),

    #[error("hash mismatch for {file}: expected {expected}, got {actual}")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("not supported yet: {0}")]
    Unsupported(String),

    #[error("cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// Machine-readable discriminator for the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Network(_) => CODE_NETWORK,
            AppError::Io(_) => CODE_IO,
            AppError::Serde(_) => CODE_SERIALIZATION,
            AppError::Database(_) => CODE_DATABASE,
            AppError::Account(_) => CODE_ACCOUNT,
            AppError::Unauthorized => CODE_UNAUTHORIZED,
            AppError::InstanceNotFound(_) => CODE_INSTANCE_NOT_FOUND,
            AppError::ModResolution(_) => CODE_MOD_RESOLUTION,
            AppError::Java(_) => CODE_JAVA,
            AppError::Transport(_) => CODE_TRANSPORT,
            AppError::Directory(_) => CODE_DIRECTORY,
            AppError::HashMismatch { .. } => CODE_HASH_MISMATCH,
            AppError::Config(_) => CODE_CONFIG,
            AppError::Unsupported(_) => CODE_UNSUPPORTED,
            AppError::Cancelled => CODE_CANCELLED,
            AppError::Other(_) => CODE_OTHER,
        }
    }

    /// `true` when retrying the same operation may plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            AppError::Network(_) | AppError::Transport(_) | AppError::Directory(_)
        )
    }

    pub fn other(message: impl Into<String>) -> Self {
        AppError::Other(message.into())
    }

    pub fn config(message: impl Into<String>) -> Self {
        AppError::Config(message.into())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 3)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.serialize_field("retryable", &self.is_retryable())?;
        state.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Helper for the many `map_err` sites that only need a message.
pub trait ResultExt<T> {
    fn network(self, context: &str) -> AppResult<T>;
    fn transport(self, context: &str) -> AppResult<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for Result<T, E> {
    fn network(self, context: &str) -> AppResult<T> {
        self.map_err(|e| AppError::Network(format!("{context}: {e}")))
    }

    fn transport(self, context: &str) -> AppResult<T> {
        self.map_err(|e| AppError::Transport(format!("{context}: {e}")))
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::Database(err.to_string())
    }
}

impl From<redis::RedisError> for AppError {
    fn from(err: redis::RedisError) -> Self {
        AppError::Directory(err.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Network(err.to_string())
    }
}

impl From<keyring::Error> for AppError {
    fn from(err: keyring::Error) -> Self {
        AppError::Account(format!("credential vault: {err}"))
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(err: zip::result::ZipError) -> Self {
        AppError::ModResolution(format!("archive error: {err}"))
    }
}

impl From<url::ParseError> for AppError {
    fn from(err: url::ParseError) -> Self {
        AppError::Config(format!("invalid url: {err}"))
    }
}

impl From<base64::DecodeError> for AppError {
    fn from(err: base64::DecodeError) -> Self {
        AppError::Config(format!("invalid base64 payload: {err}"))
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(err: tokio::task::JoinError) -> Self {
        AppError::Other(format!("background task failed: {err}"))
    }
}

impl From<walkdir::Error> for AppError {
    fn from(err: walkdir::Error) -> Self {
        AppError::Io(std::io::Error::other(err.to_string()))
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::Other(format!("tauri error: {err}"))
    }
}

/// Convenience alias used by Tauri command signatures.
pub type CommandResult<T> = Result<T, AppError>;
