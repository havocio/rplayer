//! Which file extensions the player treats as audio, and folder scanning.
//!
//! The default set is what pure-Rust `symphonia` (behind `rodio`) decodes without
//! any C dependency. `ape` / `wv` (Monkey's Audio, WavPack) are **not** in the
//! default set — they need extra crates and are gated behind cargo features
//! (see `PlayerEngine`'s `open_decoder` and the crate `ape`/`wavpack` features).

use std::path::{Path, PathBuf};

/// Extensions handled out of the box.
pub const SUPPORTED_EXTS: &[&str] = &[
    "flac", "wav", "wave", "aiff", "aif", "aifc", "alac", "m4a", "mp3", "aac", "ogg", "oga",
    "opus",
];

/// True if `path` has a supported extension (case-insensitive).
///
/// APE and WavPack are only "supported" when their cargo features are enabled,
/// so the folder scanner and UI match exactly what the engine can decode.
pub fn is_supported(path: &Path) -> bool {
    let Some(ext) = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
    else {
        return false;
    };
    if SUPPORTED_EXTS.contains(&ext.as_str()) {
        return true;
    }
    #[cfg(feature = "ape")]
    if ext == "ape" {
        return true;
    }
    #[cfg(feature = "wavpack")]
    if ext == "wv" {
        return true;
    }
    let _ = &ext;
    false
}

/// A stable, cloned copy of the supported-extension list.
pub fn supported_extensions() -> Vec<&'static str> {
    SUPPORTED_EXTS.to_vec()
}

/// Recursively collect supported audio files under `dir`.
///
/// Returns paths sorted alphabetically for a deterministic initial order.
/// Symlink loops are avoided by tracking visited canonical directories up to a
/// modest depth.
pub fn scan_dir(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    scan_into(dir, 0, max_depth, &mut out);
    out.sort();
    out
}

fn scan_into(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // unreadable dir: skip silently, best-effort scan
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            scan_into(&path, depth + 1, max_depth, out);
        } else if file_type.is_file() && is_supported(&path) {
            out.push(path);
        }
    }
}
