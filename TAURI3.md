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
- CI 与发版流水线已建立（2026-10-07，见 §3.7）：`tests.yml`（门禁）/ `warm-cache.yml`（唯一缓存写入者）/
  `release.yml`（tag → 门禁 → 打包 → `assert_bundle -Smoke` 安装冒烟（7 模型布局）→ gh release）；
  门禁与断言脚本集中在 `tools/automation/`，本机与 CI 跑同一份 `check.ps1`。
- **网络（手机 ⇄ 电脑）已实现**（2026-10-08，DESIGN.md §4.1）：axum WebSocket 服务（端口
  59123，`/ws` + `/health`）。`remote_mic` 输入行（手机麦克风→电脑，**不需要虚拟声卡驱动**，
  进引擎全链可降噪）+ `remote_speaker` 输出行（电脑→手机扬声器）+ 远程输入（手机输入法打字
  `text` 与手机实体键当全尺寸键盘 `key`，**两件事分开**，开关默认关闭）。协议见 DESIGN.md §4.1。
  - **Opus 零编译链**：不用 `opus` crate（其 `-sys` 要 cmake 编 C 源码），也不用已废弃 5 年的
    `audiopus`；改为运行时 `LoadLibraryExW` + `GetProcAddress` 加载仓库里**预编译 x64**
    `server/opus.dll`（`libopus 1.3.1`，AGENTS §2.5 保留的遗留产物，正好复用）。实测
    10 ms 帧 ≈ 29 kbps、解码每包恰好 480 样本。**坑**：`opus_encoder_get_size` /
    `opus_decoder_get_size` 是兼容桩**恒返回 0**，一律用 `*_create`（状态由 libopus 内部分配）。
  - **坑**：出站节拍必须用绝对时刻（`next += 10ms` 再 `sleep_until(next)`）。最初用
    `tokio::time::interval` 实测只产 **64 包/s**（应 100），改绝对时刻后回 100 —— 与
    §3.3 里列工作线程那个 sleep 过冲是同一个病。
  - **坑**：连接收尾若 `writer_task.abort()`，已排队的 `err`（如「协议版本不符」）会被吞掉；
    要 `drop(out_tx)` 后等写任务自然结束。
  - 回环测试 `src-tauri/src/net/loopback.rs`（10 项，真实 axum 服务 + 真实 WS 客户端），
    节拍诊断 `net/diag.rs`。Android 客户端用系统自带 `MediaCodec`（`audio/opus`），
    **无 JNI / 无 NDK**；明文 `ws://` + `usesCleartextTraffic`。
- Android 客户端已能构建并运行（2026-10-08）：`./gradlew assembleDebug`（JDK17 + SDK 34）出
  `app-debug.apk`，`adb install` + 启动成功。修掉了一批从未编译过的 Kotlin 错误：
  `MediaCodec.queueInputBuffer` 少传 `flags`、`AudioRecord.read(float[])` 少传 `readMode`、
  `MediaCodecInfo` 上不存在的 `isFormatSupported`（应取 `getCapabilitiesForType(MIME)`）、
  `View.layoutHeight`（应为 `height`）、okio `ByteString.of(ByteArray)` 已废弃（改用 `toByteString()`）、
  以及非公开 API `AudioRecord.Builder.setPerformanceMode`（SDK 里根本没有，删除）。
- UI 字符串统一管理已落地（2026-10-07）：`i18n.js` 单字典 + `T()` 占位符 + `__pvTpl()` Rust 模板表
  + `i18n_lint` 门禁（规则见 AGENTS.md §4）。
- **Linux 已跑通并出包**（2026-10-08）：本机 Ubuntu 24.04（CI 用 `ubuntu-latest`）`cargo build` /
  `clippy -D warnings` / `test`（40 项全过）/ `cargo tauri build` 均通过，出 deb / rpm / AppImage；
  `tools/automation/check.ps1` 改为跨平台（`$HOME/.cargo/bin`）。Windows 专有功能在其它平台给
  **明确「不可用」**而不静默：全局热键（`RegisterHotKey`）、AEC 远端 WASAPI 回环（改填输入设备作参考）。
  Opus 改为运行时动态加载：Windows 载随包 `opus.dll`，Linux `dlopen` 系统 `libopus.so.0`
  （缺库时只有网络音频降级，其余正常）。
- Linux 设备面与虚拟驱动（2026-10-08）：ALSA 枚举 108 条 → **精简到 18 条**（丢插件、同卡按名去重、
  优先 `plughw`；`devices::simplify`）。顶栏新增「驱动」页，按平台切（外壳 `ui/drivers.js`）：
  Linux = 虚拟驱动（`audio/virtual_mic.rs`，照 legacy 建 `purevox_out`+monitor+`purevox_mic`，创建/移除），
  Windows = VB-CABLE 下载/教程/检测（`ui/drivers_windows.js`）。AEC 远端在 Linux 走 `parec` 监听 monitor
  （`audio/loopback_linux.rs`），候选由 `list_loopback_targets` 给出。

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

> **不需要 cmake / Ninja**：网络功能的 Opus 走运行时加载预编译 `opus.dll`（见 §1 进度），
> `tools/automation/versions.env` 也不含 cmake 项。Android 端同理用系统 `MediaCodec`，无 NDK。

### 3.2 Linux 工具链（2026-10-08 本机 Ubuntu 24.04 已实测跑通）

wry 运行时在 Linux 上依赖 WebKitGTK 4.1；`cpal` 依赖 ALSA；打 rpm 需要 `rpmbuild`。
其中 `libayatana-appindicator3-dev` 在 Tauri 3 中**已不需要**（托盘默认改用 ksni），
官方 prerequisites 文档页还没更新：

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev librsvg2-dev libgtk-3-dev libasound2-dev pkg-config rpm
```

CI 用 `ubuntu-latest`（Ubuntu 24.04，glibc 2.39）——`onnxruntime` 预编译包依赖较新的 glibc
（22.04 链接会报 `__isoc23_strtol` 未定义），因此 Linux 安装包要求 **glibc ≥ 2.39**（Ubuntu 24.04+ / 较新发行版）。
Fedora / Arch / openSUSE 等发行版的包名见 prerequisites 页对应标签。

实测（Ubuntu 24.04）：`cargo build` / `clippy -D warnings` / `test`（40 项全过）/
`cargo tauri build` 均通过，出 `deb (69 MB)` / `rpm (57 MB)` / `AppImage (141 MB)`。
运行期还需系统 `libopus0`（网络音频用，见 §1 与 AGENTS §1.2；缺失时降级、不崩）。

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

### 3.7 CI 与发版（2026-10-08 起，多平台：Windows + Linux + Android）

纪律（规则正文见 AGENTS.md §2 第 7 条）：

- **触发纪律：没有任何分支推送 / PR 触发**（日常提交零成本）。`tests.yml`（只测试）只
  `workflow_dispatch`；`release.yml`（打包发版）tag `v*` 或手动；`warm-cache.yml`（唯一缓存写入者）只手动。
- **门禁唯一实现** `tools/automation/check.ps1`（**跨平台 PowerShell**，Windows 与 Linux 跑同一份）：
  `fmt`（rustfmt --check，缩进见 `src-tauri/rustfmt.toml`）/ `clippy`（`--all-targets -D warnings`）/
  `build` / `test` / `ui`（`tsc -p ui/jsconfig.json --noEmit` + `i18n_lint.js`）。本机跑全量或 `-Gate <项>`。
- **工作流** `.github/workflows/`：
  - `tests.yml`：手动 → Rust 门禁矩阵 `windows-latest` + `ubuntu-latest`；`ui` 门禁在 `ubuntu-latest`。
  - `warm-cache.yml`：**全仓库唯一缓存写入者**，手动；矩阵 Windows+Linux **冷构建**（不恢复 target 缓存）
    debug 三门禁 + `cargo tauri build`，再 save 四个桶（`continue-on-error`，同键已存在即视为成功）。
  - `release.yml`：tag `v*` 或手动 → 三平台并行：
    - **Windows**：门禁 → `cargo tauri build` → `assert_bundle.ps1 -Smoke` → MSI/NSIS。
    - **Linux**（`ubuntu-latest`）：装 WebKitGTK/ALSA/rpm 依赖 → 门禁 → `cargo tauri build` →
      `assert_bundle_linux.sh --smoke` → `test_packages.sh`（deb→ubuntu / rpm→fedora 容器安装验证）→
      deb/rpm/AppImage。
    - **Android**：JDK17 + `install_android_sdk.sh` + `./gradlew assembleDebug` → debug APK。
    - tag 时 `release_notes.ps1`（上一 tag 区间）+ `gh release create` 附带全部产物；手动只打包、不建 release。
  - **没有 Lite 变体**（Lite 已取消；遗留资产已收进只读快照 `legacy-v2026.09.30.1944/`）。
- **缓存纪律**：`tests.yml` / `release.yml` 全部 `actions/cache/restore`（无 save）；键与版本唯一来源 =
  `tools/automation/versions.env`（`CACHE_GEN` / `RUST_TOOLCHAIN` / `TCLI_VER` / `TS_VER` / `NODE_VER` /
  `LINUX_RUNNER` / `JDK_VER` / `ANDROID_PLATFORM` / `ANDROID_BUILD_TOOLS`），工作流只组合
  `hashFiles('src-tauri/Cargo.lock')`。四个桶按 `purevox-<RUNNER_OS>-…`：
  `…-cargo-…`（registry+git）、`…-target-dbg-…`、`…-target-rel-…`、`…-tauri-cli-<ver>`
  （`cargo-tauri`，未命中才 `cargo install tauri-cli --version 钉死`）。
  为什么集中写：GitHub 缓存按触发 ref 分域，各工作流各写各的只会堆出没人回读的重复条目（tag 上尤甚）。
- **产物断言**：
  - Windows `tools/automation/assert_bundle.ps1 -Smoke`：安装包 ≥ 40 MB → NSIS `/S /D=` 静默安装 →
    校验 `models\*.onnx` 7/7 → 启动并轮询 `/debug` 就绪 → 杀进程清残留。
  - Linux `tools/automation/assert_bundle_linux.sh [--smoke]`：deb ≥ 40 MB → `dpkg-deb -x` 解包 →
    校验可执行文件与 `*.onnx` 7/7 → rpm / AppImage 存在性 → `--smoke` 时 xvfb 下启动轮询 `/debug`。
  - 跨发行版 `tools/automation/test_packages.sh`（环境模拟）：容器里**真实安装**——deb→`ubuntu:24.04`
    （apt）、rpm→`fedora:latest`（dnf）、AppImage→本机 `--appimage-extract`；断言可执行文件存在、
    模型 7/7、`ldd` 无缺失。容器运行时用 `docker`（GitHub runner 自带）；本机 `DOCKER=podman`。
- **UI 字符串门禁** `tools/automation/i18n_lint.js`（`-Gate ui` 内）：规则见 AGENTS.md §4。

---

## 4. 与 PureVox 相关的待决问题

以下问题需要先讨论再动手，决定后写回本文并删掉对应条目：

（暂无）

已决（写回 §1/§3）：
- 用 **Tauri 3 alpha**（`3.0.0-alpha.4`）+ **wry**（系统 WebView2，包小）。
- 音频引擎**用 Rust 原生重写**（设备 I/O + onnxruntime Rust 绑定），不挂 Python sidecar。
- **Linux 打包依赖**：wry 需 WebKitGTK 4.1、cpal 需 ALSA、rpm 需 rpmbuild，托盘不再需 appindicator
  （见 §3.2）；Linux 出包用 `ubuntu-latest`（24.04），**要求 glibc ≥ 2.39**（onnxruntime 预编译包所致）。
- **多平台 CI + 无自动触发**：测试手动、打包 tag 触发，三平台（Windows / Linux / Android），
  无 Lite 变体（见 §3.7）。

已决定：Tk 桌面 UI 与全部旧 Python 代码已从主线删除（2026-10-07），只保留归档快照。
