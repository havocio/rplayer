# rplayer

一个用 **Rust + Tauri v2** 打造的轻量无损音乐播放器，对标 foobar2000 的核心体验。
界面为现代深色玻璃拟态，**音频引擎完全在 Rust 侧**（解码与声卡输出不经过前端），体积小、启动快、格式覆盖广。

> 不想从源码编译？到本仓库右侧的 **Releases** 页下载 Windows 安装包（NSIS `.exe` / MSI），双击安装即可。安装包由 GitHub Actions 在推 `v*` 标签时自动构建发布。

## 特性

- 播放 / 暂停 / 停止 / 上一首 / 下一首，曲终按模式自动切歌
- 拖动进度条精确 seek、音量与静音
- 播放模式：顺序 / 列表循环 / 单曲循环 / 随机
- 原生文件 / 文件夹选择，加入播放队列
- 读取标签与封面（FLAC/Vorbis/ID3/APEv2），显示标题/艺术家/专辑/年份/格式
- 无缝播放（gapless，正确处理 FLAC priming/trailing）
- 纯 Rust 解码，不捆绑浏览器内核（使用系统 WebView2）

## 支持的格式

| 格式 | 状态 | 说明 |
|------|------|------|
| FLAC | ✅ | 纯 Rust `symphonia`，无缝 + seek |
| WAV / AIFF | ✅ | 纯 Rust `symphonia` |
| ALAC / M4A / AAC / MP3 / OGG / Opus | ✅ | `rodio` 默认自带 `symphonia` 解码 |
| APE (Monkey's Audio) | ✅ | 纯 Rust `ape-decoder`（`ape` feature，应用端默认开启） |

## 快速开始

> 当前仅支持 **Windows 10 / 11**（依赖准备与构建流程只覆盖 Windows；Linux/macOS 暂不在范围内）。

前置：**Rust 1.89+**、**Node.js 18+**；Windows 需 **VS Build Tools** 与 **WebView2 运行时**（Win10/11 一般自带）。

```bash
# 开发模式（Vite 热更新 + Tauri 窗口），在仓库根目录执行
npm install
npm run tauri dev

# 本地打包（生成安装包；产物在 target/release/bundle/）
npm run tauri build
```

> 注意：直接 `cargo build --release` 产出的 exe **不含前端界面**（会显示空白/未连接）。
> 要用 `npm run tauri build`，或给 cargo 加上 `--features custom-protocol`。详见下方文档链接。

## 项目结构

```
rplayer/
├─ crates/player-core/   # 音频引擎（无 UI/Tauri 依赖，可独立测试）
├─ src-tauri/            # Tauri 后端：命令 + 进度事件 + 配置/权限/图标
├─ frontend/             # 纯 HTML/CSS/JS 前端（Vite 构建）
├─ scripts/              # 图标生成脚本
└─ docs/                 # 架构与开发文档
```

## 文档

- [架构与设计决策](docs/architecture.md) — 技术选型、前后端通信、音频链路
- [开发指南](docs/development.md) — 环境、构建、测试、打包、低内存构建与常见问题

## 未来计划

- 音乐库：目录增量扫描 + 元数据入库，按艺术家/专辑浏览与搜索
- 歌词（`.lrc`）解析与滚动显示
- 均衡器 / 频谱可视化


## 许可

本项目以 [MIT license](LICENSE-MIT) 授权。

除非你另行声明，任何对本项目的贡献同样以 MIT 许可授权，且不附加任何额外条款。

## 致谢

[rodio](https://github.com/RustAudio/rodio)、[symphonia](https://github.com/pdeljanov/Symphonia)、[cpal](https://github.com/RustAudio/cpal)、[lofty](https://github.com/Serial-ATA/lofty-rs)、[ape-decoder](https://crates.io/crates/ape-decoder)、[Tauri](https://tauri.app)。
