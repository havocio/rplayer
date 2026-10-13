//! Playback state, queue play-mode, and the UI snapshot the webview renders from.

use serde::{Deserialize, Serialize};

use crate::Track;

/// Transport state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayState {
    Stopped,
    Playing,
    Paused,
}

/// How the queue advances when a track ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayMode {
    /// Play to the end of the list, then stop.
    #[default]
    Order,
    /// Wrap around forever.
    RepeatAll,
    /// Replay the current track.
    RepeatOne,
    /// Random order (reshuffles the remaining queue as it drains).
    Shuffle,
}

/// Loudness-normalization mode (ReplayGain). `Off` keeps output bit-identical to
/// the source (times only the user volume); `Album`/`Track` apply the file's
/// stored gain so recordings play at a consistent loudness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplayGainMode {
    #[default]
    Off,
    Album,
    Track,
}

/// A cheap, serializable view of the engine for the UI. The desktop layer polls
/// [`crate::PlayerEngine::snapshot`] on a timer and emits it as an event.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub state: PlayState,
    pub mode: PlayMode,
    /// Elapsed time within the current track, seconds.
    pub position_secs: f64,
    /// Total length of the current track, seconds (0.0 if unknown).
    pub duration_secs: f64,
    /// Linear user volume in `0.0..=1.0`.
    pub volume: f32,
    /// Index of the current track in the queue (`usize::MAX` when none).
    pub index: usize,
    pub queue_len: usize,
    pub current: Option<Track>,
    pub replay_gain_mode: ReplayGainMode,
    pub preamp_db: f32,
    pub clip_guard: bool,
}

impl Snapshot {
    /// A neutral snapshot, used as a fallback before the engine is ready.
    pub fn empty() -> Self {
        Self {
            state: PlayState::Stopped,
            mode: PlayMode::Order,
            position_secs: 0.0,
            duration_secs: 0.0,
            volume: 1.0,
            index: usize::MAX,
            queue_len: 0,
            current: None,
            replay_gain_mode: ReplayGainMode::Off,
            preamp_db: 0.0,
            clip_guard: true,
        }
    }
}
