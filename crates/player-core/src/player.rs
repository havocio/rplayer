//! The playback engine: owns the OS audio sink, a single `rodio::Player`, and the queue.
//!
//! Everything here is `&mut self` (a single owner). The desktop layer keeps it in a
//! `Mutex<PlayerEngine>` and drives `poll_end()` on a timer for gapless-ish
//! auto-advance. Keeping the engine UI-agnostic makes it trivially testable.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rodio::source::SeekError;
use rodio::{
    ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate, Source,
};

#[cfg(feature = "ape")]
use rodio::buffer::SamplesBuffer;

use crate::error::EngineError;
use crate::state::{PlayMode, PlayState, Snapshot};
use crate::track::Track;
use crate::Result;

/// Format-pluggable lossless player engine.
pub struct PlayerEngine {
    /// Kept alive for the whole engine lifetime — dropping it tears down the
    /// `cpal` output stream and audio stops.
    _sink: MixerDeviceSink,
    player: Player,
    queue: Vec<Track>,
    index: usize,
    mode: PlayMode,
    volume: f32,
    state: PlayState,
    shuffle_order: Vec<usize>,
    shuffle_pos: usize,
}

impl PlayerEngine {
    /// Open the default output device and build a player wired to its mixer.
    pub fn new() -> Result<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| EngineError::Backend(e.to_string()))?;
        // Silence rodio's "stream dropped" stderr log; we manage the sink lifetime.
        sink.log_on_drop(false);

        let player = Player::connect_new(sink.mixer());
        player.set_volume(1.0);

        Ok(Self {
            _sink: sink,
            player,
            queue: Vec::new(),
            index: usize::MAX,
            mode: PlayMode::Order,
            volume: 1.0,
            state: PlayState::Stopped,
            shuffle_order: Vec::new(),
            shuffle_pos: 0,
        })
    }

    // ---- decoding -----------------------------------------------------------

    /// Open a decoder for `path`. `rodio` (symphonia) auto-probes FLAC / WAV /
    /// MP3 / AAC / OGG / ALAC. APE / WavPack are rejected here with a clear
    /// message unless enabled via cargo features (roadmap).
    ///
    /// We build with `byte_len` + `seekable` + `gapless` so the UI can scrub and
    /// so FLAC priming/trailing padding is handled correctly (gapless playback).
    fn open_decoder(&self, path: &Path) -> Result<Decoder<BufReader<File>>> {
        let file = File::open(path).map_err(|source| EngineError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let byte_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let reader = BufReader::new(file);

        Decoder::builder()
            .with_data(reader)
            .with_byte_len(byte_len)
            .with_seekable(true)
            .with_gapless(true)
            .build()
            .map_err(|e| {
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext == "ape" || ext == "wv" {
                    EngineError::UnsupportedFormat(format!(
                        "{ext} requires the `{}` feature (extra decoder crate)",
                        if ext == "ape" { "ape" } else { "wavpack" }
                    ))
                } else {
                    EngineError::Decode(e.to_string())
                }
            })
    }

    // ---- transport ----------------------------------------------------------

    /// Load and start playing the queue entry at `i`.
    pub fn play_index(&mut self, i: usize) -> Result<()> {
        let path = self
            .queue
            .get(i)
            .map(|t| t.path.clone())
            .ok_or(EngineError::NoTrack)?;
        let source = self.build_source(&path)?;

        self.player.clear();
        self.player.append(source);
        self.player.set_volume(self.volume);
        self.player.play();

        self.index = i;
        self.state = PlayState::Playing;
        Ok(())
    }

    /// Pick the right source for the format: stream FLAC/WAV/etc. via symphonia,
    /// or (when the `ape` feature is on) fully decode APE into an in-memory buffer.
    fn build_source(&self, path: &Path) -> Result<AnySource> {
        #[cfg(feature = "ape")]
        if has_ext(path, "ape") {
            return Ok(AnySource::Buffer(self.decode_ape(path)?));
        }
        Ok(AnySource::Stream(self.open_decoder(path)?))
    }

    /// Resume, or start from the top of the queue if nothing is selected yet.
    pub fn play(&mut self) -> Result<()> {
        if self.index == usize::MAX {
            if self.queue.is_empty() {
                return Err(EngineError::NoTrack);
            }
            let start = self.start_index();
            return self.play_index(start);
        }
        self.player.play();
        self.state = PlayState::Playing;
        Ok(())
    }

    pub fn pause(&mut self) {
        if self.state == PlayState::Playing {
            self.player.pause();
            self.state = PlayState::Paused;
        }
    }

    pub fn stop(&mut self) {
        self.player.clear();
        self.state = PlayState::Stopped;
    }

    pub fn toggle(&mut self) -> Result<()> {
        if self.state == PlayState::Playing {
            self.pause();
            Ok(())
        } else {
            self.play()
        }
    }

    /// Advance to the next track (manual).
    // A transport verb ("next track"), not an iterator advance — the name reads
    // best for callers even though it collides with `Iterator::next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<()> {
        self.advance(false)
    }

    /// Rewind to the previous track, or restart if we're a few seconds in.
    pub fn prev(&mut self) -> Result<()> {
        if self.queue.is_empty() {
            return Err(EngineError::NoTrack);
        }
        if self.position_secs() > 3.0 && self.index != usize::MAX {
            return self.seek(0.0);
        }
        let i = match self.mode {
            PlayMode::Shuffle => self.shuffle_prev(),
            _ => {
                if self.index == usize::MAX || self.index == 0 {
                    if self.mode == PlayMode::RepeatAll {
                        self.queue.len() - 1
                    } else {
                        0
                    }
                } else {
                    self.index - 1
                }
            }
        };
        self.play_index(i)
    }

    /// Seek to `secs` within the current track (works for the symphonia formats).
    pub fn seek(&mut self, secs: f64) -> Result<()> {
        if self.index == usize::MAX {
            return Err(EngineError::NoTrack);
        }
        let target = Duration::from_secs_f64(secs.max(0.0));
        self.player
            .try_seek(target)
            .map_err(|e| EngineError::Seek(e.to_string()))
    }

    /// Called on a timer by the host. If the current track ended on its own,
    /// advance according to the play mode.
    pub fn poll_end(&mut self) {
        if self.state == PlayState::Playing && self.player.empty() {
            let _ = self.advance(true);
        }
    }

    fn advance(&mut self, auto: bool) -> Result<()> {
        if self.queue.is_empty() {
            return Err(EngineError::NoTrack);
        }
        let last = self.queue.len() - 1;
        let idx = self.index.min(last);

        if auto && self.mode == PlayMode::RepeatOne {
            return self.play_index(idx);
        }

        let next = match self.mode {
            PlayMode::Shuffle => Some(self.shuffle_next()),
            PlayMode::RepeatAll => Some((idx + 1) % self.queue.len()),
            // Order: stop past the end. RepeatOne: manual next just advances
            // (auto-repeat is handled above).
            PlayMode::Order | PlayMode::RepeatOne => {
                if idx < last {
                    Some(idx + 1)
                } else {
                    None
                }
            }
        };

        match next {
            Some(i) => self.play_index(i),
            None => {
                self.stop();
                Ok(())
            }
        }
    }

    // ---- queue / settings ---------------------------------------------------

    pub fn set_queue(&mut self, tracks: Vec<Track>) {
        self.queue = tracks;
        self.index = usize::MAX;
        self.shuffle_order.clear();
        self.shuffle_pos = 0;
    }

    pub fn add_tracks(&mut self, tracks: Vec<Track>) {
        if self.queue.is_empty() {
            self.set_queue(tracks);
        } else {
            self.queue.extend(tracks);
        }
    }

    pub fn clear_queue(&mut self) {
        self.stop();
        self.queue.clear();
        self.index = usize::MAX;
        self.shuffle_order.clear();
        self.shuffle_pos = 0;
    }

    pub fn queue(&self) -> &[Track] {
        &self.queue
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn set_mode(&mut self, mode: PlayMode) {
        self.mode = mode;
        if mode == PlayMode::Shuffle {
            let mut order: Vec<usize> = (0..self.queue.len()).collect();
            shuffle(&mut order);
            self.shuffle_order = order;
            self.shuffle_pos = 0;
        }
    }

    pub fn mode(&self) -> PlayMode {
        self.mode
    }

    pub fn set_volume(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.volume = v;
        self.player.set_volume(v);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn position_secs(&self) -> f64 {
        self.player.get_pos().as_secs_f64()
    }

    /// Serializable view of the whole engine for the UI.
    pub fn snapshot(&self) -> Snapshot {
        let current = self.queue.get(self.index).cloned();
        Snapshot {
            state: self.state,
            mode: self.mode,
            position_secs: self.position_secs(),
            duration_secs: current.as_ref().map(|t| t.duration_secs).unwrap_or(0.0),
            volume: self.volume,
            index: self.index,
            queue_len: self.queue.len(),
            current,
        }
    }

    // ---- shuffle helpers ----------------------------------------------------

    fn start_index(&self) -> usize {
        match self.mode {
            PlayMode::Shuffle => {
                if self.shuffle_order.is_empty() {
                    0
                } else {
                    *self.shuffle_order.first().unwrap_or(&0)
                }
            }
            _ => 0,
        }
    }

    fn shuffle_next(&mut self) -> usize {
        self.ensure_shuffle();
        if self.shuffle_pos >= self.shuffle_order.len() {
            shuffle(&mut self.shuffle_order);
            self.shuffle_pos = 0;
        }
        let i = self.shuffle_order[self.shuffle_pos];
        self.shuffle_pos += 1;
        i
    }

    fn shuffle_prev(&mut self) -> usize {
        self.ensure_shuffle();
        if self.shuffle_pos == 0 {
            0
        } else {
            self.shuffle_pos -= 1;
            self.shuffle_order[self.shuffle_pos]
        }
    }

    fn ensure_shuffle(&mut self) {
        let n = self.queue.len();
        if self.shuffle_order.len() != n {
            let mut order: Vec<usize> = (0..n).collect();
            shuffle(&mut order);
            self.shuffle_order = order;
            self.shuffle_pos = 0;
        }
    }

    /// Decode an entire Monkey's Audio (`.ape`) file into an in-memory f32 buffer.
    /// Only compiled with the `ape` feature. Whole-track decode is the simplest
    /// correct integration and buys us instant, in-memory seeking via `SamplesBuffer`.
    #[cfg(feature = "ape")]
    fn decode_ape(&self, path: &Path) -> Result<SamplesBuffer> {
        use std::num::{NonZeroU16, NonZeroU32};

        use ape_decoder::ApeDecoder;

        let file = File::open(path).map_err(|source| EngineError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut decoder = ApeDecoder::new(BufReader::new(file))
            .map_err(|e| EngineError::Decode(format!("ape header: {e}")))?;
        let info = decoder.info().clone();
        let pcm = decoder
            .decode_all()
            .map_err(|e| EngineError::Decode(format!("ape decode: {e}")))?;

        let samples = pcm_to_f32(&pcm, info.bits_per_sample);
        let channels = ChannelCount::new(if info.channels == 0 {
            1
        } else {
            info.channels
        })
        .unwrap_or(NonZeroU16::MIN);
        let rate = SampleRate::new(if info.sample_rate == 0 {
            44_100
        } else {
            info.sample_rate
        })
        .unwrap_or(NonZeroU32::new(44_100).unwrap());

        Ok(SamplesBuffer::new(channels, rate, samples))
    }
}

/// Uniform source type so one append site can play streamed (symphonia) or
/// fully-decoded (APE buffer) audio.
enum AnySource {
    Stream(Decoder<BufReader<File>>),
    #[cfg(feature = "ape")]
    Buffer(SamplesBuffer),
}

impl Iterator for AnySource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        match self {
            AnySource::Stream(s) => s.next(),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            AnySource::Stream(s) => s.size_hint(),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.size_hint(),
        }
    }
}

impl Source for AnySource {
    fn current_span_len(&self) -> Option<usize> {
        match self {
            AnySource::Stream(s) => s.current_span_len(),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.current_span_len(),
        }
    }

    fn channels(&self) -> ChannelCount {
        match self {
            AnySource::Stream(s) => s.channels(),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.channels(),
        }
    }

    fn sample_rate(&self) -> SampleRate {
        match self {
            AnySource::Stream(s) => s.sample_rate(),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            AnySource::Stream(s) => s.total_duration(),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.total_duration(),
        }
    }

    fn try_seek(&mut self, pos: Duration) -> std::result::Result<(), SeekError> {
        match self {
            AnySource::Stream(s) => s.try_seek(pos),
            #[cfg(feature = "ape")]
            AnySource::Buffer(s) => s.try_seek(pos),
        }
    }
}

/// True if `path`'s extension equals `want` (case-insensitive).
#[cfg(feature = "ape")]
fn has_ext(path: &Path, want: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(want))
        .unwrap_or(false)
}

/// Convert interleaved little-endian PCM bytes to normalized `f32` samples in
/// `[-1.0, 1.0]`. Handles the bit depths Monkey's Audio produces (8/16/24/32).
#[cfg(feature = "ape")]
fn pcm_to_f32(bytes: &[u8], bits: u16) -> Vec<f32> {
    let width = (bits.div_ceil(8) as usize).max(1);
    let mut out = Vec::with_capacity(bytes.len() / width);
    match bits {
        8 => {
            for &b in bytes {
                out.push((b as f32 - 128.0) / 128.0);
            }
        }
        24 => {
            for c in bytes.chunks_exact(3) {
                let v = (c[0] as u32) | ((c[1] as u32) << 8) | ((c[2] as u32) << 16);
                let s = ((v << 8) as i32) >> 8; // sign-extend 24-bit
                out.push(s as f32 / 8_388_608.0);
            }
        }
        32 => {
            for c in bytes.chunks_exact(4) {
                let s = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                out.push(s as f32 / 2_147_483_648.0);
            }
        }
        // Default / 16-bit (APE's most common depth).
        _ => {
            for c in bytes.chunks_exact(2) {
                let s = i16::from_le_bytes([c[0], c[1]]);
                out.push(s as f32 / 32_768.0);
            }
        }
    }
    out
}

/// Tiny dependency-free Fisher–Yates using a time-seeded xorshift PRNG.
/// (Keeps the tree small; a stronger RNG is unnecessary for track order.)
fn shuffle(v: &mut [usize]) {
    if v.len() < 2 {
        return;
    }
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut state = XorShift::new(seed);
    for i in (1..v.len()).rev() {
        let j = (state.next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x2545_F491_4F6C_DD1D } else { seed })
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}
