//! rplayer desktop shell (Tauri v2): wires [`player_core`] to the webview via
//! commands + a playback-progress event stream. Audio never touches the webview —
//! the Rust side owns decode/output, the UI just renders state and issues intents.
//!
//! The queue and the folders the user added are persisted to
//! `<app_data_dir>/library.json` so they survive restarts; folders are rescanned
//! recursively on demand.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use player_core::{EngineError, PlayMode, PlayerEngine, Snapshot, Track};

/// Shared engine handle. `PlayerEngine` is `Send` but not `Sync` (it owns a
/// `cpal::Stream`), so we guard it with a `Mutex`. `Mutex<T>` is `Sync` whenever
/// `T: Send`, which is exactly what Tauri managed state requires.
type Engine = Mutex<Option<PlayerEngine>>;

/// The folders the user has added (used by `rescan` and persisted across runs).
type Sources = Mutex<Vec<PathBuf>>;

/// On-disk library snapshot.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Library {
    sources: Vec<PathBuf>,
    tracks: Vec<Track>,
    index: usize,
}

/// Payload for cover art: raw bytes + MIME, so the frontend can build a Blob URL
/// without base64 inflating the JSON payload.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverPayload {
    mime: String,
    data: Vec<u8>,
}

// ---- persistence ------------------------------------------------------------

fn library_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    Some(dir.join("library.json"))
}

fn load_library(app: &AppHandle) -> Library {
    library_file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Library>(&s).ok())
        .unwrap_or(Library {
            sources: Vec::new(),
            tracks: Vec::new(),
            index: usize::MAX,
        })
}

fn save_library(app: &AppHandle, tracks: &[Track], index: usize, sources: &[PathBuf]) {
    let Some(path) = library_file(app) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let lib = Library {
        sources: sources.to_vec(),
        tracks: tracks.to_vec(),
        index,
    };
    if let Ok(json) = serde_json::to_string(&lib) {
        let _ = std::fs::write(path, json);
    }
}

/// Clone the current source list, releasing the lock immediately (never held
/// across the engine lock, to keep a single, cycle-free lock order).
fn sources_snapshot(sources: &State<'_, Sources>) -> Vec<PathBuf> {
    sources.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn engine_err() -> String {
    "audio engine unavailable (no output device)".to_string()
}

// ---- helpers ----------------------------------------------------------------

/// Lock the engine, run a mutating call, persist the library, and return a fresh
/// snapshot so the UI updates in the same round-trip as the action.
fn mutate<F>(
    app: AppHandle,
    engine: State<'_, Engine>,
    sources: State<'_, Sources>,
    f: F,
) -> Result<Snapshot, String>
where
    F: FnOnce(&mut PlayerEngine) -> Result<(), EngineError>,
{
    let srcs = sources_snapshot(&sources);
    let mut guard = engine.lock().unwrap_or_else(|p| p.into_inner());
    let e = guard.as_mut().ok_or_else(engine_err)?;
    f(e).map_err(|e| e.to_string())?;
    let snap = e.snapshot();
    save_library(&app, e.queue(), e.index(), &srcs);
    Ok(snap)
}

/// Recursively scan `dir` (generous depth) and build tracks, skipping paths already
/// present in `existing`.
fn scan_into_queue(dir: &Path, existing: &HashSet<PathBuf>) -> Vec<Track> {
    player_core::scan_dir(dir, 16)
        .into_iter()
        .filter(|p| !existing.contains(p))
        .map(Track::from_path)
        .collect()
}

// ---- read-only / metadata commands -----------------------------------------

/// Cover art bytes for a given track path, if present.
#[tauri::command]
fn cover(path: String) -> Option<CoverPayload> {
    player_core::read_cover(Path::new(&path)).map(|c| CoverPayload {
        mime: c.mime,
        data: c.data,
    })
}

/// Current engine state, on demand (e.g. right after the window is created).
#[tauri::command]
fn snapshot(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |_| Ok(()))
}

/// The full queue (for the UI to render on startup).
#[tauri::command]
fn get_queue(engine: State<Engine>) -> Vec<Track> {
    engine
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|e| e.queue().to_vec())
        .unwrap_or_default()
}

/// The folders the user has added (so the UI can show / rescan them).
#[tauri::command]
fn get_sources(sources: State<Sources>) -> Vec<PathBuf> {
    sources_snapshot(&sources)
}

// ---- library mutation commands (persisted) ---------------------------------

/// Add individual files to the queue (dedup by path). Returns the updated queue.
#[tauri::command]
fn add_files(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    paths: Vec<String>,
) -> Result<Vec<Track>, String> {
    let (q, idx) = {
        let mut guard = engine.lock().unwrap_or_else(|p| p.into_inner());
        let e = guard.as_mut().ok_or_else(engine_err)?;
        let existing: HashSet<PathBuf> = e.queue().iter().map(|t| t.path.clone()).collect();
        let new: Vec<Track> = paths
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| !existing.contains(p))
            .map(Track::from_path)
            .collect();
        e.add_tracks(new);
        (e.queue().to_vec(), e.index())
    };
    let srcs = sources_snapshot(&sources);
    save_library(&app, &q, idx, &srcs);
    Ok(q)
}

/// Recursively scan a folder, append new tracks, remember the folder. Returns queue.
#[tauri::command]
fn add_folder(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    dir: String,
) -> Result<Vec<Track>, String> {
    let dirp = PathBuf::from(&dir);
    // Snapshot existing paths, then scan WITHOUT holding the engine lock (a big
    // folder scan shouldn't stall the progress ticker).
    let existing: HashSet<PathBuf> = engine
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|e| e.queue().iter().map(|t| t.path.clone()).collect())
        .unwrap_or_default();
    let new = scan_into_queue(&dirp, &existing);

    let (q, idx) = {
        let mut guard = engine.lock().unwrap_or_else(|p| p.into_inner());
        let e = guard.as_mut().ok_or_else(engine_err)?;
        e.add_tracks(new);
        (e.queue().to_vec(), e.index())
    };

    let srcs = {
        let mut s = sources.lock().unwrap_or_else(|p| p.into_inner());
        if !s.contains(&dirp) {
            s.push(dirp);
        }
        s.clone()
    };
    save_library(&app, &q, idx, &srcs);
    Ok(q)
}

/// Re-walk every saved folder (recursively) and rebuild the queue, preserving the
/// currently selected track when it still exists. Returns the updated queue.
#[tauri::command]
fn rescan(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Vec<Track>, String> {
    let srcs = sources_snapshot(&sources);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut tracks: Vec<Track> = Vec::new();
    for dir in &srcs {
        for p in player_core::scan_dir(dir, 16) {
            if seen.insert(p.clone()) {
                tracks.push(Track::from_path(p));
            }
        }
    }

    let (q, idx) = {
        let mut guard = engine.lock().unwrap_or_else(|p| p.into_inner());
        let e = guard.as_mut().ok_or_else(engine_err)?;
        let prev = e.queue().get(e.index()).map(|t| t.path.clone());
        e.set_queue(tracks);
        if let Some(prev) = prev {
            if let Some(i) = e.queue().iter().position(|t| t.path == prev) {
                e.set_index(i);
            }
        }
        (e.queue().to_vec(), e.index())
    };
    save_library(&app, &q, idx, &srcs);
    Ok(q)
}

// ---- transport commands -----------------------------------------------------

#[tauri::command]
fn set_queue(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    tracks: Vec<Track>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, move |e| {
        e.set_queue(tracks);
        Ok(())
    })
}

#[tauri::command]
fn clear_queue(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    let snap = {
        let mut guard = engine.lock().unwrap_or_else(|p| p.into_inner());
        let e = guard.as_mut().ok_or_else(engine_err)?;
        e.clear_queue();
        e.snapshot()
    };
    {
        let mut s = sources.lock().unwrap_or_else(|p| p.into_inner());
        s.clear();
    }
    save_library(&app, &[], usize::MAX, &[]);
    Ok(snap)
}

#[tauri::command]
fn play_index(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    index: usize,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, move |e| e.play_index(index))
}

#[tauri::command]
fn play(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |e| e.play())
}

#[tauri::command]
fn pause(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |e| {
        e.pause();
        Ok(())
    })
}

#[tauri::command]
fn toggle(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |e| e.toggle())
}

#[tauri::command]
fn stop(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |e| {
        e.stop();
        Ok(())
    })
}

#[tauri::command]
fn next(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |e| e.next())
}

#[tauri::command]
fn prev(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, |e| e.prev())
}

#[tauri::command]
fn seek(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    secs: f64,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, move |e| e.seek(secs))
}

#[tauri::command]
fn set_volume(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    v: f32,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, move |e| {
        e.set_volume(v);
        Ok(())
    })
}

#[tauri::command]
fn set_mode(
    app: AppHandle,
    engine: State<Engine>,
    sources: State<Sources>,
    mode: PlayMode,
) -> Result<Snapshot, String> {
    mutate(app, engine, sources, move |e| {
        e.set_mode(mode);
        Ok(())
    })
}

/// Periodically advance the engine and push a state snapshot to the webview so the
/// progress bar + transport buttons stay live without the UI polling commands.
fn spawn_ticker(handle: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let snap = {
            let state = handle.state::<Engine>();
            let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
            match guard.as_mut() {
                Some(engine) => {
                    engine.poll_end();
                    engine.snapshot()
                }
                None => Snapshot::empty(),
            }
        };
        let _ = handle.emit("player://tick", snap);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let lib = load_library(app.handle());
            let engine = match PlayerEngine::new() {
                Ok(mut e) => {
                    if !lib.tracks.is_empty() {
                        e.set_queue(lib.tracks.clone());
                        e.set_index(lib.index);
                    }
                    Some(e)
                }
                Err(err) => {
                    eprintln!("[rplayer] audio init failed: {err}");
                    None
                }
            };
            app.manage(Mutex::new(engine));
            app.manage(Mutex::new(lib.sources));
            spawn_ticker(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cover,
            snapshot,
            get_queue,
            get_sources,
            add_files,
            add_folder,
            rescan,
            set_queue,
            clear_queue,
            play_index,
            play,
            pause,
            toggle,
            stop,
            next,
            prev,
            seek,
            set_volume,
            set_mode
        ])
        .run(tauri::generate_context!())
        .expect("error while running rplayer");
}
