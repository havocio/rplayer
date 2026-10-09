//! rplayer desktop shell (Tauri v2): wires [`player_core`] to the webview via
//! commands + a playback-progress event stream. Audio never touches the webview —
//! the Rust side owns decode/output, the UI just renders state and issues intents.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use player_core::{EngineError, PlayMode, PlayerEngine, Snapshot, Track};

/// Shared engine handle. `PlayerEngine` is `Send` but not `Sync` (it owns a
/// `cpal::Stream`), so we guard it with a `Mutex`. `Mutex<T>` is `Sync` whenever
/// `T: Send`, which is exactly what Tauri managed state requires.
type Engine = Mutex<Option<PlayerEngine>>;

/// Payload for cover art: raw bytes + MIME, so the frontend can build a Blob URL
/// without base64 inflating the JSON payload.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverPayload {
    mime: String,
    data: Vec<u8>,
}

/// Lock the engine, run a mutating call, and return a fresh snapshot so the UI can
/// update in the same round-trip as the action (no waiting for the next tick).
fn mutate<F>(state: State<'_, Engine>, f: F) -> Result<Snapshot, String>
where
    F: FnOnce(&mut PlayerEngine) -> Result<(), EngineError>,
{
    let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
    let engine = guard
        .as_mut()
        .ok_or_else(|| "audio engine unavailable (no output device)".to_string())?;
    f(engine).map_err(|e| e.to_string())?;
    Ok(engine.snapshot())
}

// ---- read-only / metadata commands -----------------------------------------

/// Turn a list of file paths into track metadata (title/artist/album/duration).
#[tauri::command]
fn load_paths(paths: Vec<String>) -> Vec<Track> {
    paths.into_iter().map(Track::from_path).collect()
}

/// Recursively scan a folder for supported audio files.
#[tauri::command]
fn scan_dir(dir: String) -> Vec<Track> {
    player_core::scan_dir(std::path::Path::new(&dir), 6)
        .into_iter()
        .map(Track::from_path)
        .collect()
}

/// Cover art bytes for a given track path, if present.
#[tauri::command]
fn cover(path: String) -> Option<CoverPayload> {
    player_core::read_cover(std::path::Path::new(&path)).map(|c| CoverPayload {
        mime: c.mime,
        data: c.data,
    })
}

/// Current engine state, on demand (e.g. right after the window is created).
#[tauri::command]
fn snapshot(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |_| Ok(()))
}

// ---- transport commands -----------------------------------------------------

#[tauri::command]
fn set_queue(tracks: Vec<Track>, state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, move |e| {
        e.set_queue(tracks);
        Ok(())
    })
}

#[tauri::command]
fn add_tracks(tracks: Vec<Track>, state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, move |e| {
        e.add_tracks(tracks);
        Ok(())
    })
}

#[tauri::command]
fn clear_queue(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| {
        e.clear_queue();
        Ok(())
    })
}

#[tauri::command]
fn play_index(index: usize, state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, move |e| e.play_index(index))
}

#[tauri::command]
fn play(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| e.play())
}

#[tauri::command]
fn pause(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| {
        e.pause();
        Ok(())
    })
}

#[tauri::command]
fn toggle(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| e.toggle())
}

#[tauri::command]
fn stop(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| {
        e.stop();
        Ok(())
    })
}

#[tauri::command]
fn next(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| e.next())
}

#[tauri::command]
fn prev(state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, |e| e.prev())
}

#[tauri::command]
fn seek(secs: f64, state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, move |e| e.seek(secs))
}

#[tauri::command]
fn set_volume(v: f32, state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, move |e| {
        e.set_volume(v);
        Ok(())
    })
}

#[tauri::command]
fn set_mode(mode: PlayMode, state: State<'_, Engine>) -> Result<Snapshot, String> {
    mutate(state, move |e| {
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
        // Emit is best-effort; ignore send failures (e.g. window closed).
        let _ = handle.emit("player://tick", snap);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let engine = match PlayerEngine::new() {
                Ok(e) => Some(e),
                Err(err) => {
                    eprintln!("[rplayer] audio init failed: {err}");
                    None
                }
            };
            app.manage(Mutex::new(engine));
            spawn_ticker(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_paths,
            scan_dir,
            cover,
            snapshot,
            set_queue,
            add_tracks,
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
