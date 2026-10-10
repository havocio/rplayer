// rplayer frontend — vanilla JS. Talks to the Rust engine via Tauri commands,
// and stays in sync through the `player://tick` event stream (audio never runs
// in the webview). No framework, so the shipped bundle stays tiny.
import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

// ---- element refs -----------------------------------------------------------
const $ = (id) => document.getElementById(id);
const els = {
  playlist: $("playlist"),
  queueCount: $("queue-count"),
  title: $("np-title"),
  artist: $("np-artist"),
  album: $("np-album"),
  tags: $("np-tags"),
  cover: $("cover"),
  seek: $("seek"),
  timeCur: $("time-cur"),
  timeDur: $("time-dur"),
  play: $("btn-play"),
  prev: $("btn-prev"),
  next: $("btn-next"),
  mode: $("btn-mode"),
  modeIcon: $("mode-icon"),
  volume: $("volume"),
  mute: $("btn-mute"),
  addFiles: $("add-files"),
  addFolder: $("add-folder"),
  clear: $("clear-queue"),
};

// ---- state ------------------------------------------------------------------
let snap = {
  state: "stopped",
  mode: "order",
  positionSecs: 0,
  durationSecs: 0,
  volume: 1,
  index: Number.MAX_SAFE_INTEGER,
  queueLen: 0,
  current: null,
};
let queue = []; // mirror of the engine queue for rendering
let scrubbing = false;
let lastTick = { pos: 0, playing: false, at: performance.now() };
let coverUrl = null;
let lastCoverPath = null;
let muted = false;
let lastVolume = 1;

// Unified icon set — all 24×24 inline SVG using `currentColor`, so every control
// matches the theme and scales cleanly (no color-emoji mismatch).
const ICONS = {
  play: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>',
  pause: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 5h4v14H6zm8 0h4v14h-4z"/></svg>',
  order: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M3 6h12v2H3zm0 5h12v2H3zm0 5h7v2H3zm15-4v6l5-3z"/></svg>',
  repeat: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/></svg>',
  repeatOne: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2V9h-1l-2 1v1h1.5v4H13z"/></svg>',
  shuffle: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M10.59 9.17 5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"/></svg>',
  volumeHigh: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/></svg>',
  volumeLow: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M5 9v6h4l5 5V4L9 9H5zm10 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z"/></svg>',
  volumeMute: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3 3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73 4.27 3zM12 4 9.91 6.09 12 8.18V4z"/></svg>',
};

const MODES = [
  { id: "order", svg: ICONS.order, label: "顺序播放" },
  { id: "repeatAll", svg: ICONS.repeat, label: "列表循环" },
  { id: "repeatOne", svg: ICONS.repeatOne, label: "单曲循环" },
  { id: "shuffle", svg: ICONS.shuffle, label: "随机播放" },
];

const AUDIO_EXT = ["flac", "ape", "wav", "wave", "aiff", "aif", "alac", "m4a", "mp3", "aac", "ogg", "opus", "wv"];

// ---- helpers ----------------------------------------------------------------
const fmt = (s) => {
  if (!isFinite(s) || s < 0) s = 0;
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${sec.toString().padStart(2, "0")}`;
};

const setFill = (input) => {
  const min = +input.min || 0;
  const max = +input.max || 100;
  const pct = ((+input.value - min) / (max - min)) * 100;
  input.style.setProperty("--fill", `${pct}%`);
};

// ---- rendering --------------------------------------------------------------
function renderNowPlaying() {
  const t = snap.current;
  els.title.textContent = t?.title || "未选择曲目";
  els.artist.textContent = t?.artist || "—";
  els.album.textContent = t?.album || "—";
  document.title = t ? `${t.title} · rplayer` : "rplayer";

  const tags = [];
  if (t?.year) tags.push(`<span class="tag">${t.year}</span>`);
  if (t?.genre) tags.push(`<span class="tag">${t.genre}</span>`);
  if (t) {
    const ext = (t.path.split(".").pop() || "").toUpperCase();
    if (["FLAC", "APE", "WAV", "AIFF", "ALAC", "WV"].includes(ext)) {
      tags.push(`<span class="tag lossless">${ext} · LOSSLESS</span>`);
    } else {
      tags.push(`<span class="tag">${ext}</span>`);
    }
  }
  els.tags.innerHTML = tags.join("");

  els.timeDur.textContent = fmt(snap.durationSecs);
  els.seek.max = Math.max(1, Math.round(snap.durationSecs * 10)) / 10 || 1;
  loadCover(t?.path || null);
}

function renderPlaylist() {
  els.queueCount.textContent = `${queue.length} 首`;
  els.playlist.innerHTML = queue
    .map((t, i) => {
      const active = i === snap.index;
      const playing = active && snap.state === "playing";
      return `<li class="${active ? "active" : ""} ${playing ? "playing" : ""}" data-i="${i}">
        <span class="idx">${i + 1}</span>
        <span class="name"><span class="t">${escapeHtml(t.title)}</span><span class="a">${escapeHtml(t.artist)}</span></span>
        <span class="dur">${fmt(t.durationSecs)}</span>
      </li>`;
    })
    .join("");
}

function renderTransport() {
  els.play.innerHTML = snap.state === "playing" ? ICONS.pause : ICONS.play;
  const m = MODES.find((x) => x.id === snap.mode) || MODES[0];
  els.modeIcon.innerHTML = m.svg;
  els.mode.title = m.label;
  els.mode.classList.toggle("on", snap.mode !== "order");
  els.volume.value = Math.round(snap.volume * 100);
  setFill(els.volume);
  const v = snap.volume;
  const isMute = muted || v === 0;
  els.mute.innerHTML = isMute ? ICONS.volumeMute : v < 0.5 ? ICONS.volumeLow : ICONS.volumeHigh;
  els.mute.classList.toggle("on", isMute);
}

function highlightRow() {
  els.playlist.querySelectorAll("li").forEach((li) => {
    const i = +li.dataset.i;
    li.classList.toggle("active", i === snap.index);
    li.classList.toggle("playing", i === snap.index && snap.state === "playing");
  });
}

function escapeHtml(s) {
  return (s || "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

// ---- cover art --------------------------------------------------------------
async function loadCover(path) {
  if (!path) {
    resetCover();
    return;
  }
  if (path === lastCoverPath) return;
  lastCoverPath = path;
  try {
    const c = await invoke("cover", { path });
    if (!c || !c.data || !c.data.length) {
      resetCover();
      return;
    }
    const blob = new Blob([new Uint8Array(c.data)], { type: c.mime });
    if (coverUrl) URL.revokeObjectURL(coverUrl);
    coverUrl = URL.createObjectURL(blob);
    els.cover.innerHTML = `<img src="${coverUrl}" alt="cover" />`;
    els.cover.dataset.empty = "false";
  } catch (e) {
    resetCover();
  }
}

function resetCover() {
  if (coverUrl) {
    URL.revokeObjectURL(coverUrl);
    coverUrl = null;
  }
  lastCoverPath = null;
  els.cover.innerHTML = `<div class="cover-fallback">♪</div>`;
  els.cover.dataset.empty = "true";
}

// ---- snapshot application ---------------------------------------------------
function applySnapshot(next) {
  if (!next) return;
  snap = next;
  lastTick = { pos: snap.positionSecs, playing: snap.state === "playing", at: performance.now() };
  renderNowPlaying();
  renderTransport();
  highlightRow();
}

// smooth progress between 250ms ticks
function animate() {
  if (!scrubbing) {
    let pos = lastTick.pos;
    if (lastTick.playing) pos = lastTick.pos + (performance.now() - lastTick.at) / 1000;
    if (snap.durationSecs > 0) pos = Math.min(pos, snap.durationSecs);
    els.seek.value = pos;
    els.timeCur.textContent = fmt(pos);
    setFill(els.seek);
  }
  requestAnimationFrame(animate);
}

// ---- actions ----------------------------------------------------------------
const act = (fn) => async (...args) => {
  try {
    applySnapshot(await fn(...args));
  } catch (e) {
    console.error(e);
  }
};

async function addFiles() {
  const picked = await open({ multiple: true, directory: false, filters: [{ name: "音频", extensions: AUDIO_EXT }] });
  if (!picked) return;
  const paths = Array.isArray(picked) ? picked : [picked];
  const tracks = await invoke("load_paths", { paths });
  await act(invoke)("add_tracks", { tracks });
  queue = queue.concat(tracks);
  renderPlaylist();
  if (snap.index === Number.MAX_SAFE_INTEGER && queue.length) await act(invoke)("play_index", { index: 0 });
}

async function addFolder() {
  const dir = await open({ directory: true, multiple: false });
  if (!dir) return;
  const tracks = await invoke("scan_dir", { dir });
  if (!tracks.length) return;
  await act(invoke)("add_tracks", { tracks });
  queue = queue.concat(tracks);
  renderPlaylist();
  if (snap.index === Number.MAX_SAFE_INTEGER) await act(invoke)("play_index", { index: 0 });
}

async function clearQueue() {
  await act(invoke)("clear_queue");
  queue = [];
  renderPlaylist();
}

function cycleMode() {
  const cur = MODES.findIndex((m) => m.id === snap.mode);
  const next = MODES[(cur + 1) % MODES.length];
  act(invoke)("set_mode", { mode: next.id })();
}

function toggleMute() {
  if (snap.volume > 0) {
    lastVolume = snap.volume;
    muted = true;
    act(invoke)("set_volume", { v: 0 })();
  } else {
    muted = false;
    act(invoke)("set_volume", { v: lastVolume || 1 })();
  }
}

// ---- wiring -----------------------------------------------------------------
els.addFiles.addEventListener("click", () => addFiles().catch(console.error));
els.addFolder.addEventListener("click", () => addFolder().catch(console.error));
els.clear.addEventListener("click", () => clearQueue().catch(console.error));
els.play.addEventListener("click", () => act(invoke)("toggle")());
els.prev.addEventListener("click", () => act(invoke)("prev")());
els.next.addEventListener("click", () => act(invoke)("next")());
els.mode.addEventListener("click", cycleMode);
els.mute.addEventListener("click", toggleMute);

els.volume.addEventListener("input", (e) => {
  const v = +e.target.value / 100;
  muted = v === 0;
  els.mute.innerHTML = v === 0 ? ICONS.volumeMute : v < 0.5 ? ICONS.volumeLow : ICONS.volumeHigh;
  els.mute.classList.toggle("on", v === 0);
  setFill(e.target);
});
els.volume.addEventListener("change", (e) => act(invoke)("set_volume", { v: +e.target.value / 100 })());

els.seek.addEventListener("input", (e) => {
  scrubbing = true;
  els.timeCur.textContent = fmt(+e.target.value);
  setFill(e.target);
});
els.seek.addEventListener("change", (e) => {
  const to = +e.target.value;
  scrubbing = false;
  act(invoke)("seek", { secs: to })();
});

els.playlist.addEventListener("click", (e) => {
  const li = e.target.closest("li");
  if (!li) return;
  act(invoke)("play_index", { index: +li.dataset.i })();
});

window.addEventListener("keydown", (e) => {
  if (["INPUT", "TEXTAREA"].includes(document.activeElement?.tagName)) return;
  if (e.code === "Space") {
    e.preventDefault();
    act(invoke)("toggle")();
  } else if (e.code === "ArrowRight" && e.ctrlKey) {
    act(invoke)("next")();
  } else if (e.code === "ArrowLeft" && e.ctrlKey) {
    act(invoke)("prev")();
  }
});

// ---- boot -------------------------------------------------------------------
listen("player://tick", (ev) => applySnapshot(ev.payload)).catch(console.error);

(async function boot() {
  renderTransport(); // paint icons immediately (play/pause, mode, volume) even before first snapshot
  requestAnimationFrame(animate);
  try {
    applySnapshot(await invoke("snapshot"));
  } catch (e) {
    console.error("snapshot failed:", e);
  }
})();
