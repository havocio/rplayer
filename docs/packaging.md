# 构建 Windows 安装包

本文讲清楚 rplayer 的**安装包到底是怎么被"打"出来的**：NSIS / MSI 的原理、Tauri 的自动流水线、相关配置、离线打包、便携 ZIP，以及产物验证。发版流程见 [development.md](development.md)，设计背景见 [architecture.md](architecture.md)。

## 一、三种分发形态

`npm run tauri build`（或 CI）一次会产出多种 Windows 分发物，都在**仓库根** `target/release/bundle/` 下（因为本项目是 cargo workspace，产物落在 workspace 根的 `target/`，不是 `src-tauri/target/`）：

| 形态 | 路径 | 用途 |
|------|------|------|
| NSIS 安装向导 | `bundle/nsis/rplayer_<版本>_x64-setup.exe` | 面向普通用户，双击安装，带卸载器（推荐） |
| MSI | `bundle/msi/rplayer_<版本>_x64_en-US.msi` | 面向企业分发 / `msiexec` 静默部署 |
| 便携 ZIP | 见第六节（CI 额外生成） | 解压即用、免安装 |
| 裸 exe | `release/rplayer.exe` | 内嵌了界面的可执行文件，上面几种都由它派生 |

## 二、NSIS 安装程序本质

NSIS（Nullsoft Scriptable Install System）是 Windows 老牌安装程序生成器。工作方式类似编译器，只是"源码"是一段描述**装什么、装到哪、建哪些快捷方式、写哪些注册表**的 `.nsi` 脚本：

```
installer.nsi  ──makensis──►  rplayer_x.x.x_x64-setup.exe
```

产出的 exe 内嵌了：待安装文件 + NSIS 运行时 + 卸载逻辑。双击时它把文件解到 `InstallDir`，创建开始菜单/桌面快捷方式、写入 `HKCU\...\Uninstall\rplayer` 注册项并生成 `Uninstall.exe`。

## 三、Tauri 的自动流水线

你只敲 `npm run tauri build`，背后 Tauri 大致做 5 步：

1. **编译** `rplayer.exe`（cargo release；`custom-protocol` 特性把 `frontend/dist` 内嵌进 exe —— 这一步决定了 exe 有没有界面）。
2. **取 NSIS**：从 GitHub `tauri-apps/binary-releases` 下载 `nsis-3.x.zip`（含 `makensis.exe` + 插件 + 语言包），解压缓存到 `%LOCALAPPDATA%\tauri\`，下次离线复用。
3. **生成 `.nsi`**：把 `tauri.conf.json` 的 `productName / version / identifier / icon / bundle.windows.nsis.*` 填进内置模板。
4. **喂文件**：`rplayer.exe` + WebView2 Loader（按 `webviewInstallMode`）+ 需要外置的资源。
5. **执行打包**：`makensis /DVERSION=... installer.nsi` → `bundle/nsis/*-setup.exe`；MSI 走 WiX（`candle`/`light`，下载 `Wix*.zip`）。

结论：**NSIS/WiX 由 Tauri 自行下载并调用，你不用手动装它们**；但也因此第 2 步需要能访问 GitHub（见第五节离线方案）。

## 四、相关配置（`src-tauri/tauri.conf.json`）

```json
{
  "productName": "rplayer",
  "version": "0.1.0",
  "identifier": "app.rplayer.desktop",
  "build": { "frontendDist": "../frontend/dist" },
  "bundle": {
    "active": true,
    "targets": ["nsis", "msi"],
    "windows": {
      "nsis": {
        "installMode": "currentUser",            // currentUser | perMachine
        "languages": ["English", "SimpChinese"], // 生成语言选择页
        "displayLanguageSelector": true
      }
    }
  }
}
```

- `productName / version / identifier` 直接决定安装程序标题、文件名、卸载表键、装到哪个目录。
- `bundle.targets`：`"all"` 会按当前平台产出全部可用格式；这里显式写 `["nsis","msi"]` 只出 Windows 两类。
- NSIS 还可加：`template`（自定义 `.nsi` 模板路径）、`headerImage` / `sidebarImage`、`installerIcon`、`startMenuFolder`、`customLanguageFiles`。要完全掌控安装流程时，写一份自己的 `installer.nsi` 并把 `template` 指过去。

## 五、本地打包失败 / 离线出包

常见报错：

```
Info Verifying NSIS package
Downloading https://github.com/tauri-apps/binary-releases/.../nsis-3.11.zip
failed to bundle project: `timeout: global`
```

含义：**exe 已编译成功**，只是打安装包时下载 NSIS 工具超时（网络/代理/镜像受限）。三种解法：

1. **交给 CI**——GitHub Actions 的 `windows-latest` runner 下载很快，发版走 [development.md 的发布流程](development.md) 最省心。
2. **挂代理 / 换网络**后重跑 `npm run tauri build`。
3. **自备 NSIS**：`winget install NSIS.NSIS`（或 https://nsis.sourceforge.io 下载），再把已解压的 NSIS 放进 Tauri 的缓存目录 `%LOCALAPPDATA%\tauri\NSIS\`，使其命中缓存离线复用。

> 内存不足的机器（页面文件太小 `os error 1455`）另见 [development.md 的"低内存"一节](development.md)。

## 六、便携 ZIP（免安装）

原理：Tauri 在 release+`custom-protocol` 下产出的 `rplayer.exe` 已内嵌全部前端，本身即可独立运行；便携包只是把它和文档打包。CI 中的做法（`release.yml`，PowerShell）：

```powershell
$version = "<tag>" -replace '^v',''
Copy-Item target/release/rplayer.exe portable/
Copy-Item README.md,LICENSE-MIT,CHANGELOG.md portable/
Compress-Archive -Path portable/* -DestinationPath rplayer_${version}_windows_x64_portable.zip -Force
```

要点：便携包里的 exe **必须是 `custom-protocol` 构建**，否则双击同样是"未连接"空白。rplayer 的配置写 `AppData`/注册表（`currentUser` 安装），属非破坏式，因此单 exe 便携可用。

## 七、验证安装包能用的清单

在干净的 Windows 10/11（或 VM）上：

1. 双击 `-setup.exe`：出现语言选择 / 安装向导 → 选目录 → 完成。
2. 确认界面已内嵌（非"未连接"）：

   ```powershell
   Select-String -Path "$env:LOCALAPPDATA\Programs\rplayer\rplayer.exe" -Pattern "assets/index" -SimpleMatch
   ```

   命中即表示前端打进去了。
3. 启动后：添加 FLAC 文件夹 → 能播放、能拖进度 seek、能自动切歌、显示封面。
4. "设置 → 应用"里能看到 `rplayer`，卸载后快捷方式与安装目录被清干净。
5. 静默（企业分发）：安装 `rplayer_...-setup.exe /S`；卸载 `Uninstall.exe /S`。

## 八、代码签名（可选）

未签名安装包会触发 SmartScreen「未知发布者」。要消除需代码签名证书（OV/EV，或 Azure Trusted Signing），用 `signtool` 签 exe 与安装包；CI 里把证书放入 secrets 后在打包后加一步签名。开源初期一般不签，让用户「更多信息 → 仍要运行」即可。

## 九、和 GitHub Release 的衔接

以上产物由 `.github/workflows/release.yml` 在推 `v*` tag 时自动构建，并以**草稿 Release** 上传（NSIS/MSI 由 `tauri-apps/tauri-action` 上传，便携 ZIP 由 `softprops/action-gh-release` 追加）。发版与认证细节见 [development.md](development.md)。
