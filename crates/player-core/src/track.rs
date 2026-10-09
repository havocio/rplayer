//! A single playable item plus its (cached) tag metadata.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::metadata;

/// One entry in the queue.
///
/// `camelCase` serde is used so the JSON keys map cleanly to JS idioms on the
/// webview side. Cover art is intentionally **not** embedded here — it would bloat
/// the playlist payload. Fetch it separately via [`crate::read_cover`] and hand the
/// frontend a `data:` / asset URL (see the Tauri layer).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Absolute path to the audio file.
    pub path: PathBuf,
    /// Display title (falls back to the file stem when the tag is empty).
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Track length in seconds (0.0 if the probe could not determine it).
    pub duration_secs: f64,
    pub track_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}

impl Track {
    /// Build a [`Track`] from a file path, reading tags via [`lofty`].
    ///
    /// Metadata is best-effort: if the probe fails we still return a usable
    /// track whose title is the file stem, rather than turning a broken tag into
    /// a hard error that would drop the song from the playlist.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let meta = metadata::read(&path);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        match meta {
            Ok(m) => Self {
                title: m.title.unwrap_or_else(|| stem.clone()),
                artist: m.artist.unwrap_or_else(|| "Unknown artist".into()),
                album: m.album.unwrap_or_default(),
                duration_secs: m.duration_secs,
                track_number: m.track_number,
                year: m.year,
                genre: m.genre,
                path,
            },
            Err(_) => Self {
                title: stem,
                artist: "Unknown artist".into(),
                album: String::new(),
                duration_secs: 0.0,
                track_number: None,
                year: None,
                genre: None,
                path,
            },
        }
    }

    /// Convenience: build a track and surface a genuine I/O error (missing file).
    pub fn try_from_path(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let p = path.as_ref();
        std::fs::metadata(p).map_err(|source| EngineError::Io {
            path: p.to_path_buf(),
            source,
        })?;
        Ok(Self::from_path(p))
    }
}
