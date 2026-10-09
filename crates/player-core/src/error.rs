//! Error types. Per the Rust error-handling guide: libraries expose *specific*
//! typed errors (thiserror) rather than an opaque `anyhow::Error`, so callers can
//! branch on e.g. [`EngineError::UnsupportedFormat`].

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("failed to read/write audio file: {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("audio backend could not be opened: {0}")]
    Backend(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("unsupported or un-enabled format: {0}")]
    UnsupportedFormat(String),

    #[error("metadata could not be read: {0}")]
    Metadata(String),

    #[error("no track is loaded")]
    NoTrack,

    #[error("seek is not available for the current track")]
    Seek(String),
}
