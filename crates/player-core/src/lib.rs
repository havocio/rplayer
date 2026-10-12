//! `player-core` — a small, format-pluggable lossless audio playback engine.
//!
//! Design goals (mirroring the project brief):
//! * modern-sounding app, but keep the binary small → pure-Rust decode
//!   (`symphonia` behind `rodio`, no C dependencies for the default formats),
//! * correctness over cleverness → explicit `Result`, no `unwrap()` in library code,
//! * UI-agnostic → this crate knows nothing about Tauri; the desktop layer drives it.
//!
//! The public surface is [`PlayerEngine`] (owns the audio device + a single [`rodio::Player`])
//! plus [`Track`] / [`Snapshot`] value types that are cheap to serialize to the webview.

mod error;
mod formats;
mod metadata;
mod player;
mod state;
mod track;

pub use error::EngineError;
pub use formats::{is_supported, scan_dir, supported_extensions, SUPPORTED_EXTS};
pub use metadata::{read_cover, Cover, RawMeta};
pub use player::PlayerEngine;
pub use state::{PlayMode, PlayState, ReplayGainMode, Snapshot};
pub use track::Track;

/// Crate result alias.
pub type Result<T, E = EngineError> = std::result::Result<T, E>;
