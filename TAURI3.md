# Tauri 3 工作指引（tauri-3 分支）

本文件是 `tauri-3` 分支的入口文档：记录 Tauri 3 的现状、该看哪些文档、怎么编译。
我们对 Tauri 3 从零开始，凡是标注「待验证」的内容都还没有在本仓库实际跑通，
跑通一项就把标注去掉并补上实测结果。

**进度**：
- Windows hello world 已跑通（编译、运行、前端到 Rust 的 IPC、MSI / NSIS 打包）。
- 调试面板 + 本机 HTTP 调试接口（`127.0.0.1:47821`）已在 Windows 实现并验证：CPU / 内存 / GPU、
  设备枚举（cpal / WASAPI）。
- 输入采集 + 重采样到 48 kHz 已实现（2026-10-07 实测）：16 kHz（Mic Device）、44.1 kHz 双声道（Realtek）、
  48 kHz（YUKUI D80）三路同时采集，输出速率均≈48000，回调块恰为原生 10 ms（160 / 441 / 480 帧），
  输出帧恒为 480 整数倍，无丢样；重采样后原生奈奎斯特以上的频谱处于 -160 dB 测量下限（无镜像）。
  三路并发 debug 构建本进程 CPU ≈ 1.7%。
- 降噪（ONNX）已接入（2026-10-07 实测）：`ort` 2.0.0-rc.13（构建时自动下载 onnxruntime 并复制 dll）；
  `purevox_denoise_202609c_ep0012.onnx` 流式推理，实测 **1.83 ms/hop 均值、2.47 ms 最大**（实时余量约 5×），
  采集侧输出速率保持 47.9 kHz；1 kHz 测试音经降噪后电平从 -20 dB 降到约 -110 dB（非语音被抑制），
  证明模型在实际处理而非直通。输入行「降噪」按钮运行时开关。
- **踩坑**：onnxruntime 会话默认为每个核建线程池并**忙等**，在实时音频进程里会占满 CPU 并把推理
  拖慢 50 倍；必须 `with_intra_threads(1)` + `with_inter_threads(1)` + `with_intra_op_spinning(false)`。
  另有基准工具 `cargo run --release --example ort_bench`（不经过音频链路）。
- 输出已实现（2026-10-07 实测）：测试音（1 kHz -20 dBFS）和任意输入源 → 输出设备，工作线程按设备采样率
  重采样 + PI 时钟伺服（±3%）+ 预热/重同步/封顶。VB-Cable 回环（播放到 CABLE Input、从 CABLE Output
  采集）与真实默认输出（EDIFIER）各测 30 s / 16 s：48k 缓冲稳定在目标 40 ms，欠载 0、重同步 0、
  丢弃 0、流错误 0，伺服修正量约 ±300 ppm 内；回环采回频谱峰值 1000 Hz @ -20 dB。
- Linux 未开始。

> 信息收集日期：2026-10-07。Tauri 3 处于 alpha，版本号、API 和文档都可能变化，
> 引用任何结论前先核对下文「版本锚点」是否仍是最新。

---

## 1. 现状

| 项目 | 状态 |
| --- | --- |
| Tauri 3 最新版本 | `3.0.0-alpha.4`（2026-10-01；alpha.0 发布于 2026-09-13） |
| Tauri 2 最新稳定版 | `2.12.1`（2026-09-30） |
| 3.0 里程碑进度 | 约 25%，无截止日期（GitHub milestone #5） |
| 官方态度 | 迁移指南原文：alpha 用于试用新的 webview 运行时模型和反馈问题，**生产应用保持 2.x** |
| 自动迁移 | `tauri migrate` **不支持** 2 到 3 的自动迁移 |
| Rust 最低版本（MSRV） | 1.95 |

### 1.1 Tauri 3 相对 2 的核心变化

- **webview 运行时改为显式选择**：应用直接依赖运行时 crate，并在代码里调用
  `tauri::Builder::runtime(...)` 选择；没选会报 `RuntimeNotConfigured`。
  - `tauri-runtime-wry`：系统 webview（Windows WebView2 / Linux WebKitGTK），包小，Tauri 2 一直用的就是它。
  - `tauri-runtime-cef`：新增的 Chromium Embedded Framework，随应用带一份 Chromium，包大但渲染一致。
- 默认运行时类型为类型擦除的 `tauri::DynRuntime`，`AppHandle` 等类型不再需要写运行时泛型。
- `devtools` / `unstable` 等 feature 改到运行时 crate 上开启。
- Linux：`tauri` 新增 `gtk3` / `gtk4` feature（wry 用 GTK3，CEF 用 GTK4，二者不能同进程）；
  托盘默认改用 ksni（D-Bus StatusNotifierItem），**不再需要 libayatana-appindicator**。
- 开发期资源文件不再复制到 target 目录，改资源不会触发整体重编译。
- 官方插件同步发布 `3.0.0-alpha.x`，npm 包走 `next` 标签；2.x 插件不能用于 3.0 应用。

---

## 2. 文档来源（按优先级）

Tauri 3 的官方文档站 `v3.tauri.app` 是 v2 文档的一个分支，只新增或修改了 3 个页面
（迁移指南、webview 运行时、CEF）。其余页面仍是 v2 内容，标题也还写着「Tauri 2.0」。
因此：**v3 专有行为以下表第 1、2 项为准，通用概念（IPC、配置、权限、打包）查 v2 正文即可。**

| # | 文档 | 用途 | 链接 |
| --- | --- | --- | --- |
| 1 | 升级到 Tauri 3.0 Alpha | 2 到 3 的全部破坏性变更与迁移步骤，**首读** | `https://v3.tauri.app/start/migrate/from-tauri-2/` |
| 2 | Webview Runtime | 运行时选择、动态/静态分发、Linux GTK 与托盘 | `https://v3.tauri.app/develop/webview-runtime/` |
| 3 | CEF | 只在决定用 CEF 运行时时才需要 | `https://v3.tauri.app/develop/cef/` |
| 4 | Prerequisites | 各平台系统依赖（v2 内容，Linux 包列表仍含 appindicator，见 3.2 节说明） | `https://v3.tauri.app/start/prerequisites/` |
| 5 | Embedding External Binaries（sidecar） | 随应用打包外部可执行文件；若选择保留 Python 引擎作为 sidecar 时需要 | `https://v3.tauri.app/develop/sidecar/` |
| 6 | 各 crate 的 CHANGELOG | 每个 alpha 的精确变更，比文档更新更快 | tauri 仓库 `crates/*/CHANGELOG.md` |
| 7 | API 参考（docs.rs） | Rust API 精确签名 | `https://docs.rs/tauri/3.0.0-alpha.4/` |
| 8 | 官方示例 | 可直接编译的最小工程 | tauri 仓库 `examples/` |

### 2.1 适合固化（离线、钉版本）的形式

调研结论：**有，而且很好固化**。两种形式都是纯文本、带确定版本：

1. **llms.txt 整合版（单文件）** —— Tauri 官方按 llms.txt 规范生成，整站文档合成一个 markdown 文件：
   - `https://v3.tauri.app/llms-full.txt`（完整版，约 2.6 MB，已确认包含 v3 迁移指南）
   - `https://v3.tauri.app/llms-small.txt`（精简版，约 2.0 MB）
   - 缺点：URL 不带版本，内容随官网更新而变；固化时需要记下下载日期。
2. **tauri-docs 仓库源文件（钉提交）** —— 文档源码是 `.mdx`，按提交 SHA 下载即可永久复现：
   - 仓库 `tauri-apps/tauri-docs`，分支 `v3`
   - raw 地址格式：`https://raw.githubusercontent.com/tauri-apps/tauri-docs/<SHA>/src/content/docs/<路径>.mdx`
   - 三个 v3 专有页面路径：`start/migrate/from-tauri-2.mdx`、`develop/webview-runtime.mdx`、`develop/cef.mdx`

固化时推荐：**v3 专有页面按 SHA 下载源文件（体积小、可复现）+ llms-full.txt 作为全文检索兜底**。
是否把下载内容提交进仓库尚未决定（llms-full.txt 体积较大）。

### 2.2 版本锚点

更新本文时同步刷新这里，保证所有引用可复现：

| 对象 | 版本 / 提交 |
| --- | --- |
| tauri（crate 与 CLI） | `3.0.0-alpha.4`，tag `tauri-v3.0.0-alpha.4`，提交 `a8703ee487c659efbebb27c799752d523a6d09a1` |
| tauri-runtime-wry | `3.0.0-alpha.4` |
| tauri-runtime-cef | `3.0.0-alpha.5` |
| tauri-build / tauri-bundler | `3.0.0-alpha.3` |
| @tauri-apps/cli（npm `next`） | `3.0.0-alpha.4` |
| @tauri-apps/api（npm `next`） | `3.0.0-alpha.2` |
| tauri-docs `v3` 分支 | 提交 `f2e14b4fdba36ac35a61ce309846b53c45ec606e` |

注意：各 crate 版本号**不同步**（如 build 是 alpha.3、runtime-cef 是 alpha.5），
不要假设「全部写同一个版本号」，以 crates.io 实际发布为准。

---

## 3. 编译

### 3.1 Windows 工具链（2026-10-07 已在 Windows 10 22H2 x64 实测跑通）

| 依赖 | 要求 | 实测版本 |
| --- | --- | --- |
| Rust | rustup，默认工具链 `stable-x86_64-pc-windows-msvc`，>= 1.95 | rustc 1.99.0 |
| MSVC 编译器 | Visual Studio Build Tools 的 VCTools 工作负载（提供 link.exe 与 Windows SDK） | Build Tools 17.14.41 |
| WebView2 运行时 | 运行与开发都需要；**Windows 10 不一定自带**（本机就缺，需手动装） | 154.0.4258.62 |
| Node.js | 不需要（hello world 前端是纯静态 HTML，CLI 用 Cargo 版） | — |

没有 winget 的机器按下面的命令装（管理员 PowerShell，全部静默；Build Tools 约 5~7 GB、十几分钟）：

```powershell
$d = "$env:TEMP"
# 1) MSVC 编译器
Invoke-WebRequest https://aka.ms/vs/17/release/vs_BuildTools.exe -OutFile "$d\vs_BuildTools.exe"
Start-Process "$d\vs_BuildTools.exe" -Wait -ArgumentList '--quiet','--wait','--norestart','--nocache',
  '--add','Microsoft.VisualStudio.Workload.VCTools','--includeRecommended'
# 2) WebView2 运行时（Evergreen Bootstrapper）
Invoke-WebRequest https://go.microsoft.com/fwlink/p/?LinkId=2124703 -OutFile "$d\MicrosoftEdgeWebview2Setup.exe"
Start-Process "$d\MicrosoftEdgeWebview2Setup.exe" -Wait -ArgumentList '/silent','/install'
# 3) Rust（装到 %USERPROFILE%\.cargo，新开终端后 PATH 生效）
Invoke-WebRequest https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe -OutFile "$d\rustup-init.exe"
& "$d\rustup-init.exe" -y --default-toolchain stable --default-host x86_64-pc-windows-msvc --profile minimal
```

判断 WebView2 是否真的装了：看 `C:\Program Files (x86)\Microsoft\EdgeWebView\Application\` 下有没有版本号目录。
注册表 `EdgeUpdate\Clients\{F3017226-...}` 键**存在但 `pv` 为空**表示没装，不能只看键在不在。

### 3.2 Linux 工具链（待验证）

wry 运行时在 Linux 上依赖 WebKitGTK 4.1。官方 prerequisites 的 Debian 列表如下；
其中 `libayatana-appindicator3-dev` 在 Tauri 3 中**已不需要**（托盘默认改用 ksni），
文档页还没更新：

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev librsvg2-dev
```

Fedora / Arch / openSUSE 等发行版的包名见 prerequisites 页对应标签。

### 3.3 安装 Tauri 3 CLI（Windows 已实测）

本仓库**只用 Cargo 版 CLI**（`cargo tauri ...`），不引入 npm 版，仓库里也就没有 package.json / node_modules：

```sh
cargo install tauri-cli --version "^3.0.0-alpha" --locked   # 约 2.5 分钟，装出 cargo-tauri 3.0.0-alpha.4
```

### 3.4 工程布局（hello world，Windows 已实测）

工程放在仓库根 `src-tauri/`：

| 路径 | 作用 |
| --- | --- |
| `src-tauri/Cargo.toml` | 依赖：`tauri` 3.0.0-alpha.4、`tauri-runtime-wry` 3.0.0-alpha.4、`tauri-build` 3.0.0-alpha.3；调试与设备：`sysinfo` 0.39、`cpal` 0.18、`windows` 0.62（仅 Windows）；音频：`rubato` 5（重采样）、`rtrb` 0.4（无锁环）、`rustfft` 6（调试频谱） |
| `src-tauri/Cargo.lock` | 锁定依赖，**提交进仓库**（alpha 期间各 crate 频繁发版，靠它保证可复现） |
| `src-tauri/build.rs` | `tauri_build::build()` |
| `src-tauri/src/main.rs` | 选 wry 运行时，启动调试采样 / HTTP / 设备枚举，注册命令 `debug_snapshot`、`refresh_devices` |
| `src-tauri/src/debug/` | 调试状态唯一数据源 `DebugHub`（`mod.rs`）、系统采样线程（`system.rs`）、Windows GPU（`gpu_win.rs`）、HTTP 接口（`http.rs`） |
| `src-tauri/src/devices.rs` | cpal 设备枚举，刷新单一入口 `spawn_refresh` |
| `src-tauri/src/audio/` | 采集管理 `CaptureManager`（`mod.rs`）、单路采集线程（`capture.rs`）、重采样切 hop（`resampler.rs`）、电平/波形/频谱/速率测量（`meter.rs`） |
| `src-tauri/tauri.conf.json` | 应用配置；`build.frontendDist` 指向 `ui`，无 dev server |
| `src-tauri/capabilities/default.json` | 权限：主窗口 `core:default` |
| `src-tauri/ui/` | 前端（纯静态，`withGlobalTauri` 下用 `window.__TAURI__.core.invoke`）：`index.html` + 调试面板 `debug.js` / `debug.css` |
| `src-tauri/icons/` | 由 `assets/icons/audio_icon_base.png` 经 `cargo tauri icon` 生成，只保留配置引用的 5 个文件 |

Tauri 3 与 2 在骨架上唯一的差别：`main` 里必须 `.runtime(tauri_runtime_wry::Wry::default())`，
且 Cargo.toml 直接依赖 `tauri-runtime-wry`。

### 3.5 常用命令（在 `src-tauri/` 下执行，Windows 已实测）

| 命令 | 结果 |
| --- | --- |
| `cargo build` | 首次约 1 分钟，产出 `target/debug/purevox.exe` |
| `cargo tauri dev` | 编译并运行开发版 |
| `cargo tauri build` | release 版 + 两个安装包：`target/release/bundle/msi/PureVox_0.1.0_x64_en-US.msi`（2.9 MB）与 `target/release/bundle/nsis/PureVox_0.1.0_x64-setup.exe`（2.0 MB）；裸 exe 8.4 MB |
| `cargo tauri icon <png>` | 从一张方形 PNG 生成全套图标 |

`cargo tauri build` 首次会从 GitHub 下载 WiX 与 NSIS 工具（tauri-apps/binary-releases），需要能访问 GitHub。

图标：`icons/` 由 `cargo tauri icon <源图.png>` 从一张 1024 方形源图生成（PureVox 源图为深蓝渐变圆角方块 +
白色「PureVox」字样），窗口 / 托盘 / 安装包共用同一套；源图改设计后重跑该命令即可。

### 3.6 自动化验证的坑

- **WebView2 远程调试环境变量无效**：wry 自己设置了浏览器参数，`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
  会被覆盖，9222 之类的调试端口不会打开。
- **不要用模拟键鼠测界面**：Windows 会拦截后台进程抢焦点，`SendKeys` 会打到当前前台窗口（别的程序）里。
- 运行状态一律用 HTTP 调试接口验证：`curl.exe -s http://127.0.0.1:47821/debug`。
  PowerShell 的 `Invoke-WebRequest` 会按错误字符集解码中文，用 `curl.exe` 并先设
  `[Console]::OutputEncoding=[Text.Encoding]::UTF8`。
- 界面交互用 Windows UI Automation（`UIAutomationClient`）：按名称找到按钮后调 InvokePattern，
  不需要焦点。脚本里的中文控件名要用码点拼（如 `[string]::new([char[]]@(0x5237,0x65B0))` 即「刷新」），
  直接写中文字面量会被控制台代码页弄乱，查找失败。
- 有音频流时 `/debug` 响应含波形与频谱数组（数百 KB），Windows PowerShell 5 的 `ConvertFrom-Json`
  解析失败且只回显原文；改用 `.venv\Scripts\python.exe` 跑脚本解析。PowerShell 向原生程序传参会吞掉双引号，
  Python 代码写进文件再执行，不要用 `python -c "..."`。
- 截图用 `PrintWindow(hwnd, hdc, 2)`（`PW_RENDERFULLCONTENT`，能截到 WebView2 内容），
  不用 `CopyFromScreen`（截的是屏幕上最前面的窗口）。注意：
  - 从智能体 shell 启动的程序窗口是**最小化**的，先 `ShowWindow(hwnd, 4)`（`SW_SHOWNOACTIVATE`，不抢焦点）再截。
  - `Process.MainWindowHandle` 可能指向 26×26 的辅助窗口，要用 `EnumWindows` 按标题 `PureVox` 找主窗口。

---

## 4. 与 PureVox 相关的待决问题

以下问题需要先讨论再动手，决定后写回本文并删掉对应条目：

1. **Linux 打包依赖变化**：wry 需要 WebKitGTK 4.1；托盘不再需要 appindicator。
   旧的 deb Depends / rpm Requires 清单见 `legacy-v2026.09.30.1944/` 内的打包脚本。

已决（写回 §1/§3）：
- 用 **Tauri 3 alpha**（`3.0.0-alpha.4`）+ **wry**（系统 WebView2，包小）。
- 音频引擎**用 Rust 原生重写**（设备 I/O + onnxruntime Rust 绑定），不挂 Python sidecar。

已决定：Tk 桌面 UI 与全部旧 Python 代码已从主线删除（2026-10-07），只保留归档快照。
