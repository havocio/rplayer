//! rplayer desktop shell (Tauri v2): wires [`player_core`] to the webview via
//! commands + a playback-progress event stream. Audio never touches the webview —
//! the Rust side owns decode/output, the UI just renders state and issues intents.
//!
//! Two distinct lists are kept:
//! * **library** — every track found under the added folders (used to build the
//!   multi-level folder tree in the UI), and
//! * **queue** — the currently playing *album* (the engine's queue). Order /
//!   repeat / shuffle all operate within this album, matching the "play by album"
//!   model.
//!
//! Both, plus the added source folders and the selected index, persist to
//! `library.json` next to the executable (portable-friendly; falls back to the OS
//! app-data dir if the exe directory isn't writable) so the library and current
//! album survive restarts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use player_core::{EngineError, PlayMode, PlayerEngine, Snapshot, Track};

/// Combined managed state. All fields are interior-mutable `Mutex`es, so `App` is
/// `Send + Sync` even though `PlayerEngine` alone is not `Sync`.
struct App {
    engine: Mutex<Option<PlayerEngine>>,
    library: Mutex<Vec<Track>>,
    sources: Mutex<Vec<PathBuf>>,
}

/// On-disk library snapshot.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Persisted {
    sources: Vec<PathBuf>,
    library: Vec<Track>,
    queue: Vec<Track>,
    index: usize,
    /// Last used volume (0.0..=1.0), restored on startup. Defaults to 1.0 for
    /// older files that predate this field.
    #[serde(default = "default_volume")]
    volume: f32,
}

fn default_volume() -> f32 {
    1.0
}

/// Payload for cover art: raw bytes + MIME, so the frontend can build a Blob URL
/// without base64 inflating the JSON payload.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverPayload {
    mime: String,
    data: Vec<u8>,
}

fn engine_err() -> String {
    "audio engine unavailable (no output device)".to_string()
}

impl App {
    fn persist(&self, app: &AppHandle) {
        let sources = self.sources.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let library = self.library.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let (queue, index, volume) = self
            .engine
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|e| (e.queue().to_vec(), e.index(), e.volume()))
            .unwrap_or((Vec::new(), usize::MAX, 1.0));
        write_persisted(
            app,
            &Persisted {
                sources,
                library,
                queue,
                index,
                volume,
            },
        );
    }

    /// Run a transport action on the engine, then persist and return a snapshot.
    fn run<F>(&self, app: &AppHandle, f: F) -> Result<Snapshot, String>
    where
        F: FnOnce(&mut PlayerEngine) -> Result<(), EngineError>,
    {
        let snap = {
            let mut guard = self.engine.lock().unwrap_or_else(|p| p.into_inner());
            let e = guard.as_mut().ok_or_else(engine_err)?;
            f(e).map_err(|e| e.to_string())?;
            e.snapshot()
        };
        self.persist(app);
        Ok(snap)
    }
}

// ---- persistence ------------------------------------------------------------
//
// The library lives next to the executable (`<exe dir>/library.json`) so the
// portable build is fully self-contained. If that directory isn't writable
// (e.g. a perMachine install under Program Files), we fall back to the OS app
// data dir. Reads check both, preferring the one that already has data.

fn exe_dir() -> Option<PathBuf> {
    let mut p = std::env::current_exe().ok()?;
    p.pop(); // drop the file name -> directory
    Some(p)
}

/// Candidate `library.json` paths, in priority order (portable first).
fn candidate_files(app: &AppHandle) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = exe_dir() {
        v.push(d.join("library.json"));
    }
    if let Ok(d) = app.path().app_data_dir() {
        v.push(d.join("library.json"));
    }
    v
}

fn default_persisted() -> Persisted {
    Persisted {
        sources: Vec::new(),
        library: Vec::new(),
        queue: Vec::new(),
        index: usize::MAX,
        volume: 1.0,
    }
}

fn load_persisted(app: &AppHandle) -> Persisted {
    for path in candidate_files(app) {
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Ok(data) = serde_json::from_str::<Persisted>(&s) {
                return data;
            }
        }
    }
    default_persisted()
}

fn write_persisted(app: &AppHandle, data: &Persisted) {
    let Ok(json) = serde_json::to_string(data) else {
        return;
    };
    for path in candidate_files(app) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&path, &json).is_ok() {
            return; // first writable location wins (exe dir preferred)
        }
    }
}

/// Recursively scan `dir` and build tracks, skipping paths already in `existing`.
fn scan_new(dir: &Path, existing: &HashSet<PathBuf>) -> Vec<Track> {
    player_core::scan_dir(dir, 16)
        .into_iter()
        .filter(|p| !existing.contains(p))
        .map(Track::from_path)
        .collect()
}

/// Clone the current library (all tracks).
fn library_of(state: &App) -> Vec<Track> {
    state.library.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

// ---- read-only commands -----------------------------------------------------

#[tauri::command]
fn cover(path: String) -> Option<CoverPayload> {
    player_core::read_cover(Path::new(&path)).map(|c| CoverPayload {
        mime: c.mime,
        data: c.data,
    })
}

#[tauri::command]
fn snapshot(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |_| Ok(()))
}

/// Every track in the library (frontend builds the folder tree from these paths).
#[tauri::command]
fn get_library(state: State<App>) -> Vec<Track> {
    library_of(&state)
}

/// The folders the user has added (tree roots).
#[tauri::command]
fn get_sources(state: State<App>) -> Vec<PathBuf> {
    state.sources.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// The currently loaded album (engine queue).
#[tauri::command]
fn get_queue(state: State<App>) -> Vec<Track> {
    state
        .engine
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|e| e.queue().to_vec())
        .unwrap_or_default()
}

// ---- library mutation commands (update the full library) --------------------

#[tauri::command]
fn add_files(
    app: AppHandle,
    state: State<App>,
    paths: Vec<String>,
) -> Result<Vec<Track>, String> {
    {
        let mut lib = state.library.lock().unwrap_or_else(|p| p.into_inner());
        let mut existing: HashSet<PathBuf> = lib.iter().map(|t| t.path.clone()).collect();
        for p in paths {
            let pb = PathBuf::from(&p);
            if existing.insert(pb.clone()) {
                lib.push(Track::from_path(pb));
            }
        }
    }
    state.persist(&app);
    Ok(library_of(&state))
}

#[tauri::command]
fn add_folder(app: AppHandle, state: State<App>, dir: String) -> Result<Vec<Track>, String> {
    let dirp = PathBuf::from(&dir);
    {
        let mut lib = state.library.lock().unwrap_or_else(|p| p.into_inner());
        let existing: HashSet<PathBuf> = lib.iter().map(|t| t.path.clone()).collect();
        lib.extend(scan_new(&dirp, &existing));
    }
    {
        let mut s = state.sources.lock().unwrap_or_else(|p| p.into_inner());
        if !s.contains(&dirp) {
            s.push(dirp);
        }
    }
    state.persist(&app);
    Ok(library_of(&state))
}

/// Re-walk every saved folder (recursively) and rebuild the library.
#[tauri::command]
fn rescan(app: AppHandle, state: State<App>) -> Result<Vec<Track>, String> {
    let srcs = state.sources.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut lib: Vec<Track> = Vec::new();
    for dir in &srcs {
        for p in player_core::scan_dir(dir, 16) {
            if seen.insert(p.clone()) {
                lib.push(Track::from_path(p));
            }
        }
    }
    *state.library.lock().unwrap_or_else(|p| p.into_inner()) = lib;
    state.persist(&app);
    Ok(library_of(&state))
}

#[tauri::command]
fn clear_library(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    let snap = {
        let mut guard = state.engine.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_mut() {
            Some(e) => {
                e.clear_queue();
                e.snapshot()
            }
            None => Snapshot::empty(),
        }
    };
    *state.library.lock().unwrap_or_else(|p| p.into_inner()) = Vec::new();
    *state.sources.lock().unwrap_or_else(|p| p.into_inner()) = Vec::new();
    state.persist(&app);
    Ok(snap)
}

// ---- album playback (sets the engine queue to one album) --------------------

/// Load an album's tracks as the queue and start at `index`. Order / repeat /
/// shuffle then stay within this album.
#[tauri::command]
fn play_album(
    app: AppHandle,
    state: State<App>,
    tracks: Vec<Track>,
    index: usize,
) -> Result<Snapshot, String> {
    state.run(&app, move |e| {
        e.set_queue(tracks);
        e.play_index(index)
    })
}

// ---- transport commands (operate within the current album) ------------------

#[tauri::command]
fn play_index(
    app: AppHandle,
    state: State<App>,
    index: usize,
) -> Result<Snapshot, String> {
    state.run(&app, move |e| e.play_index(index))
}

#[tauri::command]
fn play(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |e| e.play())
}

#[tauri::command]
fn pause(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |e| {
        e.pause();
        Ok(())
    })
}

#[tauri::command]
fn toggle(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |e| e.toggle())
}

#[tauri::command]
fn stop(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |e| {
        e.stop();
        Ok(())
    })
}

#[tauri::command]
fn next(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |e| e.next())
}

#[tauri::command]
fn prev(app: AppHandle, state: State<App>) -> Result<Snapshot, String> {
    state.run(&app, |e| e.prev())
}

#[tauri::command]
fn seek(app: AppHandle, state: State<App>, secs: f64) -> Result<Snapshot, String> {
    state.run(&app, move |e| e.seek(secs))
}

#[tauri::command]
fn set_volume(app: AppHandle, state: State<App>, v: f32) -> Result<Snapshot, String> {
    state.run(&app, move |e| {
        e.set_volume(v);
        Ok(())
    })
}

#[tauri::command]
fn set_mode(app: AppHandle, state: State<App>, mode: PlayMode) -> Result<Snapshot, String> {
    state.run(&app, move |e| {
        e.set_mode(mode);
        Ok(())
    })
}

/// Periodically advance the engine and push a state snapshot to the webview.
fn spawn_ticker(handle: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let snap = {
            let state = handle.state::<App>();
            let mut guard = state.engine.lock().unwrap_or_else(|p| p.into_inner());
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
            let data = load_persisted(app.handle());
            let engine = match PlayerEngine::new() {
                Ok(mut e) => {
                    e.set_volume(data.volume);
                    if !data.queue.is_empty() {
                        e.set_queue(data.queue.clone());
                        e.set_index(data.index);
                    }
                    Some(e)
                }
                Err(err) => {
                    eprintln!("[rplayer] audio init failed: {err}");
                    None
                }
            };
            app.manage(App {
                engine: Mutex::new(engine),
                library: Mutex::new(data.library),
                sources: Mutex::new(data.sources),
            });
            spawn_ticker(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cover,
            snapshot,
            get_library,
            get_sources,
            get_queue,
            add_files,
            add_folder,
            rescan,
            clear_library,
            play_album,
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
