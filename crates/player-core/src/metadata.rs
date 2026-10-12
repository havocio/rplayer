//! Tag / property probing via [`lofty`] (pure Rust, supports FLAC/Vorbis/ID3/APE tags).

use std::borrow::Cow;
use std::path::Path;

use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;

use crate::error::EngineError;

/// Raw, non-allocation-heavy metadata snapshot for one file.
#[derive(Clone, Debug, Default)]
pub struct RawMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub duration_secs: f64,
    /// ReplayGain values (dB / linear peak), read from the file's tags if present.
    pub rg_track_gain_db: Option<f32>,
    pub rg_album_gain_db: Option<f32>,
    pub rg_track_peak: Option<f32>,
    pub rg_album_peak: Option<f32>,
}

/// A cover image extracted from the file's tags.
#[derive(Clone, Debug)]
pub struct Cover {
    /// Encoded image bytes (JPEG / PNG — whatever the tag stored).
    pub data: Vec<u8>,
    /// MIME type, e.g. `image/jpeg`.
    pub mime: String,
}

fn opt_str<'a>(value: Option<Cow<'a, str>>) -> Option<String> {
    value.map(|c| c.into_owned())
}

/// Read common tags + duration. Fails only if the file cannot be probed at all.
pub fn read(path: &Path) -> Result<RawMeta, EngineError> {
    let tagged = Probe::open(path)
        .and_then(|p| p.read())
        .map_err(|e| EngineError::Metadata(format!("{}: {e}", path.display())))?;

    let props = tagged.properties();
    let duration_secs = props.duration().as_secs_f64();

    let mut meta = RawMeta {
        duration_secs,
        ..Default::default()
    };

    if let Some(tag) = tagged.primary_tag() {
        meta.title = opt_str(tag.title());
        meta.artist = opt_str(tag.artist());
        meta.album = opt_str(tag.album());
        meta.genre = opt_str(tag.genre());
        meta.track_number = tag.track();
        meta.rg_track_gain_db = tag.get_string(ItemKey::ReplayGainTrackGain).and_then(parse_num);
        meta.rg_album_gain_db = tag.get_string(ItemKey::ReplayGainAlbumGain).and_then(parse_num);
        meta.rg_track_peak = tag.get_string(ItemKey::ReplayGainTrackPeak).and_then(parse_num);
        meta.rg_album_peak = tag.get_string(ItemKey::ReplayGainAlbumPeak).and_then(parse_num);
    }

    Ok(meta)
}

/// Parse the leading numeric token of a ReplayGain value, e.g. `"-10.43 dB"` -> -10.43,
/// `"0.98881"` -> 0.98881. Returns `None` for empty/garbage tags.
fn parse_num(s: &str) -> Option<f32> {
    s.split_whitespace().next()?.parse::<f32>().ok()
}

/// Extract the first cover image, if present. Kept separate from [`read`] so we
/// don't allocate large image buffers while scanning a whole folder.
pub fn read_cover(path: &Path) -> Option<Cover> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag()?;
    let pic = tag.pictures().first()?;
    let mime = match pic.mime_type() {
        Some(MimeType::Png) => "image/png".to_string(),
        Some(MimeType::Jpeg) => "image/jpeg".to_string(),
        Some(MimeType::Gif) => "image/gif".to_string(),
        Some(MimeType::Bmp) => "image/bmp".to_string(),
        Some(MimeType::Tiff) => "image/tiff".to_string(),
        _ => "application/octet-stream".to_string(),
    };
    Some(Cover {
        data: pic.data().to_vec(),
        mime,
    })
}
