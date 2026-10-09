# GitHub 发布说明 / Changelog

遵循 Keep a Changelog 与 SemVer。安装包的下载入口在 Releases 页面。

## v0.1.0

首个可安装版本（仅 Windows 10 / 11）。

新增：

- 无损播放：FLAC / WAV / AIFF，及 ALAC / MP3 / AAC / OGG / Opus（纯 Rust `rodio` + `symphonia`）
- Monkey's Audio (APE) 支持（`ape-decoder`，应用端默认开启）
- 精确进度条 seek、音量与静音、顺序 / 列表循环 / 单曲循环 / 随机四种模式
- 原生文件 / 文件夹选择，加入队列后曲终自动切换
- 标签与封面读取（FLAC / Vorbis / ID3 / APEv2）
- gapless 无缝播放
- 现代深色玻璃拟态界面（Tauri v2 + 原生 HTML/CSS/JS，前端产物 gzip 约 6KB）

规划中：

- WavPack (.wv) 解码支持
- 音乐库（目录增量扫描 + SQLite 入库，按艺术家 / 专辑浏览与搜索）
- 歌词（`.lrc`）滚动显示
- 均衡器 / 频谱可视化
- 自定义标题栏、系统托盘与媒体键快捷键
