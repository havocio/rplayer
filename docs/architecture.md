# 架构与设计决策

本文说明 rplayer 的技术选型理由、前后端如何通信、以及音频链路的设计。使用层面请先看 [../README.md](../README.md)。

## 技术选型

| 诉求 | 选型 | 理由 |
|------|------|------|
| 现代美观的界面 | Tauri v2 + 原生 HTML/CSS/JS | 用 CSS 达到接近 Web 的视觉上限；不引前端框架，前端产物极小 |
| 体积小 | 系统 WebView2 + 纯 Rust 解码 | 不捆绑浏览器内核；解码用 `symphonia`（纯 Rust，无 C 依赖）；release 开 `LTO` + `strip` |
| 无损格式覆盖 | `rodio` + `symphonia` | FLAC/WAV/AIFF/ALAC/MP3/AAC/OGG/Opus 开箱即用，支持无缝播放与精确 seek |
| APE / WavPack | 额外 crate，feature 门控 | 按需开启，不强塞进默认体积 |

## 关键决定：音频不经过前端

Webview 里的 `<audio>` 无法播放 APE/WavPack，所以把**解码与声卡输出全部放在 Rust 侧**。前端只负责渲染界面、下发指令，并通过事件接收播放状态。收益：格式全覆盖、可做 gapless、UI 与音频解耦、引擎可脱离 UI 单独测试。

```
┌─────────────────────────────┐        invoke()        ┌──────────────────────────────┐
│  frontend/  (WebView2)      │  ───────────────────▶  │  src-tauri/  (Rust)          │
│  纯 HTML/CSS/JS，深色玻璃 UI │  play/pause/seek/...    │   Tauri commands + State     │
│                             │  ◀───────────────────  │            │                 │
│  进度条 / 封面 / 播放列表    │      player://tick      │            ▼                 │
└─────────────────────────────┘                        │  crates/player-core          │
                                                        │  rodio + symphonia (+ape)    │
                                                        │  cpal → WASAPI → 声卡        │
                                                        └──────────────────────────────┘
```

## 分层

- **`crates/player-core`** — 纯音频引擎，不依赖 Tauri：
  - `player.rs` `PlayerEngine`：持有输出设备（`MixerDeviceSink`）与单个 `rodio::Player`，提供 `play_index/pause/stop/next/prev/seek/set_volume/set_mode/poll_end/snapshot`。全部 `&mut self`，单一所有权，易测试。
  - `formats.rs`：受支持扩展名与目录递归扫描。
  - `metadata.rs` / `track.rs`：用 `lofty` 读取标签、时长、封面，产出可序列化的 `Track`。
  - `state.rs`：`PlayState`/`PlayMode` 与给 UI 的 `Snapshot`。
  - `error.rs`：`thiserror` 的 `EngineError`（对外返回具体错误类型，便于前端分类处理）。
- **`src-tauri`** — Tauri 外壳：
  - 用 `Mutex<Option<PlayerEngine>>` 作为托管状态。`PlayerEngine` 因内含 `cpal::Stream` 在 Windows 上是 `Send` 但**非 `Sync`**，故必须包 `Mutex`（`Mutex<T>: Sync` 当 `T: Send`），满足 Tauri 对管理状态的要求。
  - `#[tauri::command]` 暴露动作；变更类命令直接返回新的 `Snapshot`，UI 同一往返即可刷新。
  - 一个后台线程每 250ms `poll_end()`（驱动曲终自动切换）并 `emit("player://tick", snapshot)`。

## 解码策略

- **流式**（默认，FLAC/WAV/ALAC/MP3/…）：`rodio::Decoder` 经 `symphonia` 边解边播，内存占用低。要支持 seek 必须用
  `Decoder::builder().with_data(r).with_byte_len(len).with_seekable(true).with_gapless(true).build()`——
  直接 `Decoder::new` 默认**不可 seek**（`try_seek` 会报 "Symphonia decoder returned an error"）。
- **整段缓冲**（APE）：用 `ape-decoder` 把 `.ape` 一次性解码为 `Vec<f32>`，再包进 `rodio::buffer::SamplesBuffer`。
  `SamplesBuffer` 自身实现了 `Source` 且**自带 `try_seek`**，所以整段解码后 seek 能力"白送"。代价是大文件首次加载稍慢、占内存，且按 `bits_per_sample` 归一化（8/16/24/32 位）。
- 两种来源在 append 处用一个 `AnySource`（enum，实现 `Iterator + Source`）统一，调用点只认一种类型。

## 前后端约定

- 命令参数名与 Rust 形参一致（`paths/dir/path/tracks/index/secs/v/mode`）；返回 `Result<Snapshot, String>`，失败在前端以 Promise reject 捕获。
- 数据用 `#[serde(rename_all = "camelCase")]`，与 JS 习惯对齐（`positionSecs`/`durationSecs`/`queueLen`/`current`）。
- 枚举 `PlayMode` 序列化为 `"order" | "repeatAll" | "repeatOne" | "shuffle"`。
- 封面走独立命令 `cover(path)`，返回 `{ mime, data: Vec<u8> }`，前端用 `new Blob([new Uint8Array(data)], {type: mime})` 生成 ObjectURL——不把大图塞进列表 JSON。

## 待办 / 已知取舍

- WavPack 尚未接入（需要 `.wv` 容器 reader，`symphonia-codec-wavpack`/`oxideav-wavpack`）。
- APE 整段解码发生在命令线程，超大文件首帧可能短暂阻塞 UI——计划改后台线程。
