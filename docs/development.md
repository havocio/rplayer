# 开发指南

面向贡献者。功能与使用请先看 [../README.md](../README.md)，设计原理见 [architecture.md](architecture.md)。

> 目标平台：当前**仅 Windows 10 / 11**。代码在架构上未做平台绑定（`cpal`/`rodio` 本身跨平台），但依赖准备、构建脚本与 CI 只覆盖 Windows，暂不维护 Linux/macOS。

## 环境依赖

- **Rust** 1.89+（工具链已在 1.94 验证，MSVC 工具链）
- **Node.js** 18+（构建前端）
- Windows 构建依赖：Visual Studio Build Tools（C/C++ 生成工具 + 链接器）+ WebView2 运行时（Win10/11 一般自带）

## 日常开发

在**仓库根目录**执行（Vite 以 `frontend/` 为工程根，Tauri CLI 自动发现同级的 `src-tauri/`）：

```bash
npm install
npm run tauri dev        # 启动 Vite(localhost:5173) + Tauri 窗口，前后端热重载
```

引擎逻辑可脱离界面单独跑：

```bash
cargo test -p player-core                 # 纯逻辑：扫描/标签/序列化/播放模式
cargo test -p player-core -- --ignored    # 真实声卡：播放/暂停/seek/切歌（需有输出设备）
```

## 打包发布（本机）

> 安装包的原理、离线出包与验证清单见 [packaging.md](packaging.md)。

在**仓库根目录**执行；它会先 `vite build` 生成 `frontend/dist`，再用 `custom-protocol` 把前端内嵌进 exe，并产出安装包：

```bash
npm run tauri build
# 因为是 cargo workspace，产物在【仓库根】target/ 下：
#   target/release/rplayer.exe
#   target/release/bundle/nsis/*.exe   (NSIS 安装向导，推荐)
#   target/release/bundle/msi/*.msi    (MSI)
```

> ⚠️ 常见坑 1：直接 `cargo build --release` 不加 `--features custom-protocol`，产出的 exe **不含前端界面**，双击显示空白 / "未连接"。要用 `npm run tauri build`，或手动：
> ```bash
> npm run build                                            # 生成 frontend/dist
> cargo build --release -p rplayer --features custom-protocol
> ```
> 验证内嵌成功：exe 二进制里能搜到 `assets/index…` 这类前端指纹串。

> ⚠️ 常见坑 2：`tauri build` 打安装包时会**联网从 GitHub 下载 NSIS / WiX 打包工具**（`nsis-*.zip`、`Wix*.zip`）。若出网受限会报 `failed to bundle project: timeout: global`（此时 exe 已编译成功，只是安装包没生成）。三种应对：
>
> 1. 直接交给 CI（`windows-latest` runner 下载很快，见下一节）；
> 2. 挂代理后重试；
> 3. 首次下载成功后 Tauri 会缓存到 `%LOCALAPPDATA%\tauri\`，之后可离线复用。

## 发布到 GitHub Releases（自动出安装包）

安装包由 `.github/workflows/release.yml` 自动构建：推一个 `v*` tag，Actions 会在 `windows-latest` 上用官方 `tauri-apps/tauri-action` 构建（内部完成 `vite build` + `custom-protocol` 内嵌 + NSIS/MSI 打包），并把 **NSIS `.exe`、MSI、以及便携 ZIP** 挂到该 tag 的 **草稿 Release** 上。你到 Releases 页确认无误后点 **Publish release** 即正式发布；用户从 Releases 下载安装包、无需自己编译。

> 便携 ZIP = 内嵌了界面的单个 `rplayer.exe` + 文档，解压即用、免安装（配置写入 AppData/注册表，属非破坏式）。

首次接入步骤（在本机执行）：

```bash
# 0) 一次性认证（三选一，见下方「GitHub 认证」）

# 1) 关联你在 GitHub 上建好的空仓库（不要勾选 README）
git remote add origin https://github.com/<你的用户名>/<仓库名>.git

# 2) 首推 main 分支
git branch -M main
git push -u origin main

# 3) 打版本 tag 触发自动打包发布（tag 与 src-tauri/tauri.conf.json 的 version 对应）
git tag v0.1.0
git push origin v0.1.0
```

发版前记得同步版本号：`package.json`、`Cargo.toml`（workspace 与两个成员）、`src-tauri/tauri.conf.json` 三处的 `version`。

在 GitHub 上给工作流放行权限：`Settings → Actions → General → Workflow permissions` 选 **Read and write contents**（等价于本 workflow 顶部的 `permissions: contents: write`）。

### GitHub 认证（"认证失败"排查）

GitHub 从 2021 年起**不再接受账号密码**推送，报 `Authentication failed` / `支持密码验证` 相关错误时，任选其一：

- **GitHub CLI（最省事）**：装 `gh` → `gh auth login` → 选 GitHub.com / HTTPS / Login with a web browser。之后 `git push` 自动带上凭据。
- **HTTPS + Personal Access Token**：GitHub `Settings → Developer settings → Personal access tokens` 生成一个（Fine-grained 需给目标仓库 **Contents: Read and write**；或 classic 勾 `repo` scope）。推送时用户名填你的 GitHub 用户名、**密码处粘贴 token**。可用 `git config credential.helper manager` 让 Windows 记住。
- **SSH**：`ssh-keygen -t ed25519` → 把公钥加到 GitHub `Settings → SSH and GPG keys` → remote 改用 `git@github.com:<用户>/<仓库>.git`。

如果你是把仓库建在组织里或用了 2FA，token 必须显式授予该仓库的写权限，否则推 tag / 建 Release 会 403。


## 低内存 / 页面文件不足

rplayer 依赖图较大（Tauri 会拉入庞大的 `windows` / `windows-sys`）。在内存或页面文件紧张的机器上，默认并行 + 全量 debuginfo 可能让 rustc 崩溃，表现为**在无关 crate（如 `aho-corasick`、`symphonia-*`）里冒出 `undeclared Vec/Box`、`cannot find Option` 之类的假错误**——根因是 `os error 1455（页面文件太小）` 导致元数据被写坏，而非源码或本项目的问题。

降内存重编即可绕过：

```bash
# 开发构建（关调试信息、限并行）
CARGO_PROFILE_DEV_DEBUG=0 cargo build -p rplayer -j 2

# 发布构建（thin LTO、限并行）
CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_LTO=thin \
  cargo build --release -p rplayer --features custom-protocol -j 1
```

也可适当调大系统页面文件，或减小 `-j`。

## 图标

应用图标由脚本生成（青绿→深青渐变 + 金色播放键），输出到 `src-tauri/icons/`：

```bash
python scripts/make_icons.py    # 需要 Pillow
```

改配色：编辑脚本顶部的 `TEAL` / `TEAL_DARK` / `GOLD` 常量后重跑。若用 Tauri 标准流程，也可 `npm run tauri icon path/to/1024.png` 重新生成全套图标。

## 体积参考

- release 裸 exe（strip 后）约 **5.7 MB**；正式 `tauri build` 会内嵌约 6KB 的前端并产出更小的安装包。
- 使用系统 WebView2，不捆绑浏览器内核，这是体积的主要优势。
- release profile：`LTO` + `strip` + `codegen-units=1` + `opt-level="s"`。若要进一步瘦身，可关掉 `rodio` 默认不需要的解码特性，只保留 FLAC/WAV。

## 代码规范

- 格式化：`cargo fmt`（`rustfmt.toml`；`imports_granularity`/`group_imports` 需 nightly）。
- 静态检查：`cargo clippy --all-features -- -D warnings`（`clippy.toml`，MSRV 1.89）。
- 引擎库用 `thiserror` 暴露具体错误；应用外壳用 `anyhow`/`String` 向 UI 传递错误。
- 库代码避免 `unwrap()`；`unsafe` 目前为零（纯 Rust 解码栈）。

## 提交前自检

```bash
cargo clippy -p player-core --all-features -- -D warnings
cargo clippy -p rplayer -- -D warnings
cargo test -p player-core
npm run build
```
