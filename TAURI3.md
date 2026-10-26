# Tauri 3 工作指引（tauri-3 分支）

本文件是 `tauri-3` 分支的入口文档：记录 Tauri 3 的现状、该看哪些文档、怎么编译。
我们对 Tauri 3 从零开始，凡是标注「待验证」的内容都还没有在本仓库实际跑通，
跑通一项就把标注去掉并补上实测结果。

**进度**：
- Windows hello world 已跑通（编译、运行、前端到 Rust 的 IPC、NSIS 打包 —— Windows 只出 exe，
  不出 MSI，见 `src-tauri/tauri.windows.conf.json`）。
- 调试面板 + 本机 HTTP 调试接口（`127.0.0.1:47821`）已在 Windows 实现并验证：CPU / 内存 / GPU、
  设备枚举（cpal / WASAPI）。
- 输入采集 + 重采样到 48 kHz 已实现（2026-10-07 实测）：16 kHz（Mic Device）、44.1 kHz 双声道（Realtek）、
  48 kHz（YUKUI D80）三路同时采集，输出速率均≈48000，回调块恰为原生 10 ms（160 / 441 / 480 帧），
  输出帧恒为 480 整数倍，无丢样；重采样后原生奈奎斯特以上的频谱处于 -160 dB 测量下限（无镜像）。
  三路并发 debug 构建本进程 CPU ≈ 1.7%。
- 降噪（ONNX）已接入（2026-10-07 实测）：`ort` 2.0.0-rc.13（构建时自动下载 onnxruntime 并复制 dll）；
  `purevox_denoise_202609b_ep0046.onnx` 流式推理，实测 **1.83 ms/hop 均值、2.47 ms 最大**（实时余量约 5×），
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
  门禁与断言脚本集中在 `tools/automation/`，本机与 CI 跑同一份 `check.py`。
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
  `app-debug.apk`，`adb install` + 启动成功。**发版走 `assembleRelease` + 签名**（凭据走环境变量、
  keystore 在仓库外，见 `android/README.md`；CI 缺 secret 直接失败，不发未签名包）。修掉了一批从未编译过的 Kotlin 错误：
  `MediaCodec.queueInputBuffer` 少传 `flags`、`AudioRecord.read(float[])` 少传 `readMode`、
  `MediaCodecInfo` 上不存在的 `isFormatSupported`（应取 `getCapabilitiesForType(MIME)`）、
  `View.layoutHeight`（应为 `height`）、okio `ByteString.of(ByteArray)` 已废弃（改用 `toByteString()`）、
  以及非公开 API `AudioRecord.Builder.setPerformanceMode`（SDK 里根本没有，删除）。
- UI 字符串统一管理已落地（2026-10-07）：`i18n.js` 单字典 + `T()` 占位符 + `__pvTpl()` Rust 模板表
  + `i18n_lint` 门禁（规则见 AGENTS.md §4）。
- **Linux 已跑通并出包**（2026-10-08）：本机 Ubuntu 24.04（CI 用 `ubuntu-24.04`）`cargo build` /
  `clippy -D warnings` / `test`（40 项全过）/ `cargo tauri build` 均通过，出 deb / rpm / AppImage；
  `tools/automation/check.py` 跨平台（Windows / Linux / macOS 同一份）。Windows 专有功能在其它平台给
  **明确「不可用」**而不静默：全局热键（`RegisterHotKey`）、AEC 远端 WASAPI 回环（改填输入设备作参考）。
  Opus 改为运行时动态加载：Windows 载随包 `opus.dll`，Linux `dlopen` 系统 `libopus.so.0`
  （缺库时只有网络音频降级，其余正常）。
- Linux 设备面与虚拟驱动（2026-10-08）：ALSA 枚举 108 条 → **精简到 18 条**（丢插件、同卡按名去重、
  优先 `plughw`；`devices::simplify`）。顶栏新增「驱动」页，按平台切（外壳 `ui/drivers.js`）：
  Linux = 虚拟驱动（`audio/virtual_mic.rs`，照 legacy 建 `purevox_out`+monitor+`purevox_mic`，创建/移除），
  Windows = VB-CABLE 下载/教程/检测（`ui/drivers_windows.js`）。AEC 远端在 Linux 走 `parec` 监听 monitor
  （`audio/loopback_linux.rs`），候选由 `list_loopback_targets` 给出。
- **Tauri 依赖升到 `3.0.0-alpha.5`**（2026-10-14 上游发布）：`tauri` / `tauri-runtime-wry`
  alpha.4 → alpha.5、`tauri-build` alpha.3 → alpha.4（`tauri-codegen` / `tauri-macros` /
  `tauri-utils` 随之到 alpha.4）；CLI 钉 `TCLI_VER=3.0.0-alpha.5`，同时把 `CACHE_GEN` 提到 **2**
  （缓存键里含着 `TCLI_VER` 与代数，旧桶整体作废、必须重暖）。各 crate 版本号**不同步**，
  以 crates.io 实际发布为准（见 §2 版本锚点）。改了 `Cargo.toml` 之后，`src-tauri/Cargo.lock`
  由下一次 `cargo build` / `cargo update` 自动重新解析并生成，别手改锁文件（本条升级已用
  `cargo update -p tauri --precise 3.0.0-alpha.5` 落地并提交：只动 tauri 族 + `swift-rs`，
  其余依赖原地不动，共 25 增 17 删）。

> 信息收集日期：2026-10-07。Tauri 3 处于 alpha，版本号、API 和文档都可能变化，
> 引用任何结论前先核对下文「版本锚点」是否仍是最新。

---

## 0. 2026-10-12 收尾：输入/输出补全 + 更新 + 界面重做

**输入/输出其实后端早就有了，缺的是界面**——这也是「感觉功能没做完」的真正原因：
`soundpad` / `player` / `loopback` / `remote_mic` / `remote_speaker` 的后端都在跑，
但界面上一个控件都没有（或给错了选项）。本轮补齐界面并重做整套样式。

- **音效板 / 播放器界面**（`ui/media.js` 新增）：音效网格（每格：播放键、名字、热键徽标、
  删除、±10 dB 滑杆、**热键录制框**）+ 播放器走带（曲目、进度条、播放/暂停/暂停态、
  音量、单曲循环徽标）。两者是**处理行**，全部控件由 `media.js` 独占，
  `columns.js` 不再画通用参数控件（否则同一件事两个入口）。
- **每个音效一个全局热键**：`hotkey.rs` 从「单个热键」改成「一组绑定、**一个线程**」
  （`RegisterHotKey` 的 id 按序号分配 + HashMap 派发）；热键**跟着计划走**
  （`pads` 参数的 `hotkey` 字段），由 `plan::pad_hotkeys` 统一收口、`apply_plan` 时重注册。
- **`loopback` 输入行**：原来错列了**输入设备**列表（后端会把输入设备 ID 当渲染端点去打，
  必然打不开）；改成与 AEC 同一套回环目标下拉，并补上「音量(dB)」参数控件。
  老配置缺 `loopback:` 前缀的，界面按 `list_loopback_targets` 保守补前缀并落盘。
- **手机面板从调试面板迁出**为顶栏常驻面板（服务开关 / 远程输入开关 / 本机地址 / 怎么用），
  调试面板只留只读状态行——这就是「放在调试里很奇怪」的来源。
- **更新检查与更新**（`src/update/`，新增）：GitHub Releases → 查 → 下载 → 静默安装，
  顶栏「更新」面板 + 调试接口 `update` 段。详见 DESIGN.md §4.2 与 AGENTS.md §1.2。
- **版本号改为「年.月.日」**（如 `26.10.8`）：tag 仍是完整时间戳 `v2026.10.08.1430`，
  更新比较用 tag（`version::STAMP`），同日发两次也分得清。
- **界面整体重做**（`ui/debug.css`）：去掉所有写死的高度/行数，改 flex 纵向自适应；
  统一控件外观（按钮状态、滑杆、下拉、复选框、焦点环）、卡片化、悬停/焦点反馈、
  细滚动条、`prefers-reduced-motion` 降级。**规则写在 CSS 文件头，改样式前先读**。

**本轮踩到的坑**（都在仓库里有对应注释/测试）：

1. **symphonia 的 feature 分「容器」与「解码器」两类**。原来只开了
   `mp3/aac/flac/vorbis/pcm` —— `pcm` 只是 PCM **解码器**，**没有 WAV 容器读取器**，
   于是 `media_import` 对任何 wav 都报「无法识别的音频格式：end of stream」。
   要 `wav`（容器）+ `ogg`（vorbis 的容器）+ `isomp4`（m4a / AAC-in-MP4）才算齐。
2. **离线重采样要冲尾**。`ToHops` 只出整 hop，文件导入会丢掉 sinc 重采样器的输出延迟
   （44.1k→48k 实测短 **480 样本 = 整整 1 hop**）。文件导入改用 `Converter` + 末尾喂静音
   凑够精确长度再截断；**实时链路继续用 `ToHops`**（要 10 ms 网格与流式节奏）。
3. **`media::import` 按内容哈希幂等**，测试留下的旧文件会让下一次跑直接走「已存在」分支
   而跳过解码——改完解码逻辑跑测试前先清 `~/.purevox/{soundpad,music}`，否则会看到
   「改了没生效」的假象。
4. **前端资源是编译期内嵌的**：`frontendDist` 指向 `ui/`，改完 JS/CSS **必须 `cargo build`**
   才会生效，直接重启旧 exe 会让你以为改动没起作用。
5. **`i18n_lint` 的 W1 孤儿键曾漏报纯 ASCII 的 msgid**：占位符模板
   （如 `'{state} {pos} / {dur}'`）不含中文，被 `CJK.test()` 过滤掉、永远进不了
   `usedKeys`，于是恒报孤儿。已修（先登记 `T()` 的 msgid 再按是否含中文决定查不查 E1/E2）。
6. **Windows 静默自更新必须先退进程**：NSIS 安装器开头有 `CheckIfAppIsRunning`，
   正在运行的 exe 会被拦下弹框。用**脱离的 cmd**（`ping -n 5 127.0.0.1 >nul & 安装器 /S
   /UPDATE /NS /R`）先歇几秒等本进程退出，再由安装器接手并按 `/R` 重启应用。
7. **「本机是哪种包」由二进制自己回答，别去问 dpkg/rpm**（2026-10-18）：bundler 打包时把
   `__TAURI_BUNDLE_TYPE_VAR_{APP,DEB,RPM,MSI,NSS}` 打进二进制，`tauri::utils::platform::bundle_type()`
   读它即可（`tauri::utils` = `pub use tauri_utils as utils`，`platform` 与 `config::BundleType`
   都是公开的）。本仓库产物实测：deb 里是 `VAR_DEB`、AppImage 里是 `VAR_APP`。
   **每个包各自被改写过**，所以同一份源码打三种 Linux 包会得到三个不同的二进制 ——
   这正是「一个包只认自己那种更新产物」的依据（`update/install_linux.rs`）。
   没打标的二进制（`cargo run`）返回 `None`，Linux 侧据此落 `unknown`、不下载任何包。

---

## 1. 现状

| 项目 | 状态 |
| --- | --- |
| Tauri 3 最新版本 | `3.0.0-alpha.5`（2026-10-14；alpha.0 发布于 2026-09-13） |
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
| 7 | API 参考（docs.rs） | Rust API 精确签名 | `https://docs.rs/tauri/3.0.0-alpha.5/` |
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
| tauri（crate 与 CLI） | `3.0.0-alpha.5`，tag `tauri-v3.0.0-alpha.5`，提交 `1991873f5e2baa68773f21465f61baa93077498f` |
| tauri-runtime-wry | `3.0.0-alpha.5` |
| tauri-runtime-cef | `3.0.0-alpha.6` |
| tauri-build / tauri-bundler | `3.0.0-alpha.4` |
| @tauri-apps/cli（npm `next`） | `3.0.0-alpha.5` |
| @tauri-apps/api（npm `next`） | `3.0.0-alpha.3` |
| tauri-docs `v3` 分支 | 提交 `f2e14b4fdba36ac35a61ce309846b53c45ec606e` |

注意：各 crate 版本号**不同步**（如 build 是 alpha.4、runtime-cef 是 alpha.6），
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

CI 用 `ubuntu-24.04`（Ubuntu 24.04，glibc 2.39）——`onnxruntime` 预编译包依赖较新的 glibc
（22.04 链接会报 `__isoc23_strtol` 未定义），因此 Linux 安装包要求 **glibc ≥ 2.39**（Ubuntu 24.04+ / 较新发行版）。
Fedora / Arch / openSUSE 等发行版的包名见 prerequisites 页对应标签。

实测（Ubuntu 24.04）：`cargo build` / `clippy -D warnings` / `test`（40 项全过）/
`cargo tauri build` 均通过，出 `deb (69 MB)` / `rpm (57 MB)` / `AppImage (141 MB)`。
运行期还需系统 `libopus0`（网络音频用，见 §1 与 AGENTS §1.2；缺失时降级、不崩）。

### 3.3 安装 Tauri 3 CLI（Windows 已实测）

本仓库**只用 Cargo 版 CLI**（`cargo tauri ...`），不引入 npm 版，仓库里也就没有 package.json / node_modules：

```sh
cargo install tauri-cli --version "^3.0.0-alpha" --locked   # 约 2.5 分钟，装出 cargo-tauri 3.0.0-alpha.5
```

### 3.4 工程布局（hello world，Windows 已实测）

工程放在仓库根 `src-tauri/`：

| 路径 | 作用 |
| --- | --- |
| `src-tauri/Cargo.toml` | 依赖：`tauri` 3.0.0-alpha.5、`tauri-runtime-wry` 3.0.0-alpha.5、`tauri-build` 3.0.0-alpha.4；调试与设备：`sysinfo` 0.39、`cpal` 0.18、`windows` 0.62（仅 Windows）；音频：`rubato` 5（重采样）、`rtrb` 0.4（无锁环）、`rustfft` 6（调试频谱） |
| `src-tauri/Cargo.lock` | 锁定依赖，**提交进仓库**（alpha 期间各 crate 频繁发版，靠它保证可复现） |
| `src-tauri/build.rs` | `tauri_build::build()` |
| `src-tauri/src/main.rs` | 选 wry 运行时，启动调试采样 / HTTP / 设备枚举，注册命令 `debug_snapshot`、`refresh_devices` |
| `src-tauri/src/debug/` | 调试状态唯一数据源 `DebugHub`（`mod.rs`）、系统采样线程（`system.rs`）、Windows GPU（`gpu_win.rs`）、HTTP 接口（`http.rs`） |
| `src-tauri/src/devices.rs` | cpal 设备枚举，刷新单一入口 `spawn_refresh` |
| `src-tauri/src/audio/` | 采集管理 `CaptureManager`（`mod.rs`）、单路采集线程（`capture.rs`）、重采样切 hop（`resampler.rs`）、电平/波形/频谱/速率测量（`meter.rs`） |
| `src-tauri/tauri.conf.json` | 应用配置；`build.frontendDist` 指向 `ui`，无 dev server |
| `src-tauri/capabilities/default.json` | 权限：主窗口 `core:default` |
| `src-tauri/ui/` | 前端（纯静态，`withGlobalTauri` 下用 `window.__TAURI__.core.invoke`）：`index.html` + 调试面板 `debug.js` / `debug.css` |
| `src-tauri/icons/` | 由 `tools/automation/gen_icons.py` 生成（跨平台同一条命令），见 3.5.1 |

Tauri 3 与 2 在骨架上唯一的差别：`main` 里必须 `.runtime(tauri_runtime_wry::Wry::default())`，
且 Cargo.toml 直接依赖 `tauri-runtime-wry`。

### 3.5 常用命令（在 `src-tauri/` 下执行，Windows 已实测）

| 命令 | 结果 |
| --- | --- |
| `cargo build` | 首次约 1 分钟，产出 `target/debug/purevox.exe` |
| `cargo tauri dev` | 编译并运行开发版 |
| `cargo tauri build` | release 版 + 安装包：`target/release/bundle/nsis/PureVox_<版本>_x64-setup.exe`；裸 exe 8.4 MB。**Windows 只出这一个 exe**（`bundle.targets=["nsis"]`），不再出 MSI |

`cargo tauri build` 首次会从 GitHub 下载 WiX 与 NSIS 工具（tauri-apps/binary-releases），需要能访问 GitHub。

图标见下面 3.5.1。

### 3.5.1 图标生成（**不参与 CI**）

图标是**一次性生成、产物入库**的静态资源：CI 里不重新生成（AGENTS §2.7 的门禁与打包都不碰
它），改设计时开发者本地跑一次脚本，把产物提交上去即可。理由是它需要 Pillow 这个宿主能力，
而打包根本不需要图像库——放进 CI 只会多一个「CI 能跑、本地跑不了」的分叉点。

```text
python tools/automation/gen_icons.py            # 全量（Windows / Linux / macOS 同一条命令）
python tools/automation/gen_icons.py -Compare   # 只比对形状，不写文件
python tools/automation/gen_icons.py -Skip Android,Ios
python tools/automation/gen_icons.py -Out /tmp/x # 试跑：输出到 /tmp/x，一个字节都不碰仓库
```

依赖：`pip install pillow`。仅本机一次性使用，CI 不需要（产物已入库）。

**只有这一份实现。** 此前是 `gen_icons.ps1`（GDI+）与 `gen_icons.sh`（ImageMagick）两份，
共用同一套盒子定义，但两个图像库的渐变抖动、抗锯齿、降采样滤波器都不同，**实测同尺寸
像素差 50~90%** —— 两台机器生成的图标长得不一样，「一个功能只有一条实现路径」就是句空话。
现在同一条命令在三个平台跑出**逐字节相同**的产物。

顺带一个好处：Pillow 的渐变是逐行连续插值，而 `LinearGradientBrush` 默认**开抖动**（往渐变里
掺噪点防条带）。所以同一张 1024px 的 macOS 图标，GDI+ 版 49 KB、Pillow 版 9 KB —— 更小
且没有抖动噪点。

`-Out` 必须**完全隔离**：桌面图标与 Android 图标一起搬过去，一个字节都不碰仓库。之前
Android 那段写死了仓库路径，于是「试跑到临时目录」反而把仓库里的 `mipmap-*` 覆盖了 ——
生成脚本半隔离比不隔离更危险。

**Android 的图标不进 `src-tauri/icons/`**：那里曾放 `cargo tauri icon` 的产物，现已删除
（见 AGENTS §2.5）。脚本直接写到 `android/app/src/main/res/mipmap-*`。

**P 是四个直角盒子，不是字体**

```text
上横  x 120..336  y  54..107      左竖  x 120..173  y  54..488
右竖  x 338..391  y 109..270      中横  x 120..336  y 272..325
描边 = 每个盒子向外扩 30px（方形外扩）    含描边包围盒 332×495
```

坐标是 512 画布上的绝对像素，逐像素量自旧源图。**改设计就是改这四个盒子加描边宽度。**

早期版本走 ark-pixel 像素字体渲染（legacy `tools/gen_lite_tray_icon.py` 的托盘黄 P），现已
弃用，三个理由：

1. 字体只在**只读快照**里（AGENTS §2.3 / §2.5），现役树不该依赖它；
2. PIL / GDI+ / FreeType 的**字形度量各不相同**（同一参数实测差 8%），同一份代码在不同平台
   会画出不同的 P —— 对「跨平台同款图标」是硬伤；
3. P 本身是直角几何图形，矢量描述比「先渲染成字形再缩放」更准也更省。

**相对旧源图的两处修正**：P 改为居中（旧图垂直偏下约 15px）；描边末端不再被画布切掉（旧图
画布 512 高而 P 含描边高 495、底边在 y=518，底部 7px 是平的）。所以与旧图的形状 IoU 约 90%，
差异集中在边界 1px（旧的字体扩散带抗锯齿，新的矢量是实边），肉眼不可见。

**圆边为什么要超采样**：直接按目标尺寸画圆，圆周就是一圈台阶（mdpi 48px 上尤其明显）。
两个脚本都是 **4× 超采样画完再降采样**，圆边是多像素灰阶过渡，观感是平滑弧线。

**产出**

| 位置 | 内容 |
| --- | --- |
| `src-tauri/icons/audio_icon_base.png` | 源图（512 透明底 P） |
| `src-tauri/icons/{32x32,64x64,128x128,128x128@2x,icon.png}` + `icon.ico`（16/24/32/48/64/128/256） | 打包用（`tauri.conf.json` 的 `bundle.icon`） |
| `src-tauri/icons/Square{30,44,71,89,107,142,150,284,310}x…Logo.png` + `StoreLogo.png` | Windows Store |
| `src-tauri/icons/icon.icns`（icp4…ic10） | macOS（配置引用，但本项目不打包 macOS） |
| `src-tauri/icons/ios/AppIcon.appiconset/` | iOS 的 20 个槽位 + `Contents.json`（见下） |
| `src-tauri/icons/tray_running.png` / `tray_stopped.png` | 托盘 64px。**运行态 = 蓝渐变底 + 浅蓝 P；停止态 = 暗灰底 + 暗蓝 P** —— 整套一起压暗才有「熄灭」的观感，只压底色的话 P 仍是亮蓝，在暗灰底上比运行时还跳，一眼看不出停了（AGENTS §1.2 托盘）。P 高占 72%：64px 下再小就看不清 |
| `android/app/src/main/res/mipmap-*/` | 五档密度 × `ic_launcher` / `_round`（传统，API < 26）/ `_foreground` / `_monochrome` |
| `android/app/src/main/res/mipmap-anydpi-v26/{ic_launcher,ic_launcher_round}.xml` | 自适应图标声明（API 26+）：背景 + 前景 + **`<monochrome>`**（Android 13+ 主题图标）；由脚本写出，勿手改 |
| `android/app/src/main/res/drawable/ic_launcher_bg.xml` | 自适应图标背景层（深蓝竖向渐变，满幅无圆角） |

**Android 自适应图标为什么不是 `cargo tauri icon` 那套**：它从透明底源图取不到主色，
`ic_launcher_background` 被填成 `#FFFFFF`；而前景层是带圆角底的整图，四角透明——系统一裁就露出
白底，图标变成「白底 + 一个圆角深蓝方块」。这里改成标准三层：背景是**同一条渐变**的矢量，
前景只有安全区里的 P（108dp 画布、72dp 安全区），裁切处两边同色同位置，接缝看不出来；
第三层 `monochrome` 是**单色剪影**（描边与本体同色，只有 alpha 有意义），供 Android 13+
主题图标整体着色 —— 少了它，开了主题图标的手机会把我们的图标画成一个灰底方块。
`ic_launcher_round.xml` 与方形指向同一套层（API 26+ 圆形设备走它，而非退回传统 PNG）。

#### iOS：必须是 appiconset，不是裸 PNG

`icons/ios/AppIcon.appiconset/` 下是 20 个槽位 + `Contents.json`。**没有这个索引文件，Xcode 根本
不认这套图标**——早先这里放的是 `cargo tauri icon` 留下的裸 PNG，形状对但结构不对，等于废图。

同一物理尺寸在不同 idiom 下是**同一个文件**（20x20@2x 的 iPhone 与 iPad 都是 40px），只有 iPad
独有的槽位才另起 `~ipad` 文件。旧图里那几个 `AppIcon-{20x20,29x29,40x40}@2x-1.png` 是
`cargo tauri icon` 自己的 iPad 命名 —— 既不是 Xcode 约定，内容又和各自的 `@2x` **完全相同**
（同一尺寸而已），属重复文件，已随结构重建一并消失。

本项目**不做 iOS**（CI 只有 Windows / Linux / Android），所以这套图标目前只是备着；真要做 iOS，
`Contents.json` 已经是标准结构，直接可用。

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

**Linux 桌面会话里做无人值守验证**（`tools/automation/ui.py` 的会话子命令：`start` / `take` /
`verify-update` / `stop`）：

- 两条前置，缺一条 `ui.py list` 就是空的（不是没控件，是 a11y 桥没起来）：装 `python3-pyatspi`；
  打开 GTK 的 a11y 总开关 `gsettings set org.gnome.desktop.interface toolkit-accessibility true`
  （重装/精简安装后默认是 false）。
- **会话总线上要有 `org.kde.StatusNotifierWatcher`**：GNOME 上这个角色由 appindicator 扩展提供，
  扩展没启用（`gnome-extensions info ubuntu-appindicators@ubuntu.com` 显示 INACTIVE）时总线上
  根本没有这个名字，而纯 Rust 的 `ksni` 把它当**硬错误** —— ≤26.10.18 的应用会**启动即 panic**
  （`failed to register to the StatusNotifierWatcher`），用户连界面都看不到。`main.rs` 现已只把
  「托盘不可用 + 原因」写进调试面板并继续跑；要复现/验证这个场景用 `ui.py start --private-bus`
  （另起一条没有 watcher 的私有总线：老二进制在那里 panic，新二进制正常起）。总线真缺 watcher 时
  `ui.py start` 会自动顶一个替身（`ui.py watcher`，只实现 ksni 用到的
  `RegisterStatusNotifierItem` + `IsStatusNotifierHostRegistered`）。
- **AT-SPI 跨不过总线**：`--private-bus` 起的实例，`ui.py click` / `verify-update` 找不到它的控件
  （工具会提示换实例）。
- 更新分流一条链验证（包已装好之后）：`ui.py start` → `ui.py verify-update deb`
  （断言 `install_kind` == `deb` 且产物名以 `.deb` 结尾；跨格式兜底会让 deb 用户白下 140 MB 还装不上）
  → `ui.py take '.update'` 看细节 → `ui.py stop`。开发构建用 `verify-update none`
  （应在「没有 CI 时间戳」处停下并给原因，或 `unsupported`）。
- **应用内「下载更新」在本机会失败**（报网络原因，不静默）：GitHub 的资产 CDN 在这条网络上基本
  不可用（`gh release download` 15 分钟拿不到 76 MB 的包）。要手工取资产走 `gh-proxy.com`
  （实测 ~370 KB/s），并用 GitHub API 自带的 digest 核对：
  `gh api repos/<owner>/<repo>/releases/tags/<tag> --jq '.assets[] | "\(.name) \(.digest)"'`。

### 3.7 CI 与发版（2026-10-08 起，多平台：Windows + Linux + Android）

纪律（规则正文见 AGENTS.md §2 第 7 条）：

- **触发纪律：没有任何分支推送 / PR 触发**（日常提交零成本）。`tests.yml`（只测试）只
  `workflow_dispatch`；`release.yml`（打包发版）tag `v*` 或手动；`warm-cache.yml`（唯一缓存写入者）只手动。
- **门禁唯一实现** `tools/automation/check.py`（**Python，跨平台唯一**，Windows / Linux / macOS 跑同一份）：
  `fmt`（rustfmt --check，缩进见 `src-tauri/rustfmt.toml`）/ `clippy`（`--all-targets -D warnings`）/
  `build` / `test` / `ui`（`tsc -p ui/jsconfig.json --noEmit` + `i18n_lint.js`）/
  `version`（版本号推导，见 `version.py`）。本机跑全量或 `-Gate <项>`。
  **为什么从 PowerShell 换成 Python**：门禁要跨三平台跑，而 pwsh 只有 Windows / Linux 装得像样
  （CI runner 自带，本机还得自己单独装 —— 本机为跑门禁装 Microsoft 源还撞上 GPG key 签名
  失败）；Python 三边都是系统自带，冷启动 ~20 ms（pwsh ~330 ms），且只用标准库、不用
  `pip install`。两个 workflow 里的 `shell: pwsh` 也随之取消。同理 `version.py` /
  `gen_icons.py` 也都是**单份 Python**，取代原先的 `.ps1` + `.sh` 双份。
- **工作流** `.github/workflows/`：
  - `tests.yml`：手动 → Rust 门禁矩阵 `windows-latest` + `ubuntu-24.04`；`ui` 门禁在 `ubuntu-24.04`。
  - `warm-cache.yml`：**全仓库唯一缓存写入者**，手动；矩阵 Windows+Linux **冷构建**（不恢复 target 缓存）
    debug 三门禁 + `cargo tauri build`，再 save 四个桶（`continue-on-error`，同键已存在即视为成功）。
  - `release.yml`：tag `v*` 或手动 → 三平台并行：
    - **Windows**：门禁 → `cargo tauri build` → `assert_bundle.py --smoke` → NSIS exe（只此一个）。
    - **Linux**（`ubuntu-24.04`）：装 WebKitGTK/ALSA/rpm 依赖 → 门禁 → `cargo tauri build` →
      `assert_bundle.py --smoke` → `test_packages.py`（deb→ubuntu / rpm→fedora 容器安装验证）→
      deb/rpm/AppImage。
    - **Android**：JDK17 + `install_android_sdk.py` + 从 `gh secret` 还原签名 keystore →
      `./gradlew assembleRelease` → **签名 release APK**（`PV_REQUIRE_SIGNING=1`，缺 secret 当场失败）。
    - tag 时 `release_notes.py`（上一 tag 区间）+ `gh release create` 附带全部产物；手动只打包、不建 release。
  - **没有 Lite 变体**（Lite 已取消；遗留资产已收进只读快照 `legacy-v2026.09.30.1944/`）。
- **缓存纪律**：`tests.yml` / `release.yml` 全部 `actions/cache/restore`（无 save）；键与版本唯一来源 =
  `tools/automation/versions.env`（`CACHE_GEN` / `RUST_TOOLCHAIN` / `TCLI_VER` / `TS_VER` / `NODE_VER` /
  `LINUX_RUNNER` / `JDK_VER` / `ANDROID_PLATFORM` / `ANDROID_BUILD_TOOLS`），工作流只组合
  `hashFiles('src-tauri/Cargo.lock')`。四个桶按 `purevox-<RUNNER_OS>-…`：
  `…-cargo-…`（registry+git）、`…-target-dbg-…`、`…-target-rel-…`、`…-tauri-cli-<ver>`
  （`cargo-tauri`，未命中才 `cargo install tauri-cli --version 钉死`）。
  为什么集中写：GitHub 缓存按触发 ref 分域，各工作流各写各的只会堆出没人回读的重复条目（tag 上尤甚）。
- **产物断言**（`assert_bundle.py` 是入口，平台差异拆到 `_windows.py` / `_linux.py`，
  见 AGENTS §4）：
  - Windows：安装包 ≥ 40 MB → NSIS `/S /D=` 静默安装 → 校验 `models\*.onnx` 7/7 →
    启动并轮询 `/debug` 就绪 → 杀进程清残留。
  - Linux：deb ≥ 40 MB → `dpkg-deb -x` 解包 → 校验可执行文件与 `*.onnx` 7/7 →
    rpm 必需 / AppImage best-effort → `--smoke` 时 xvfb 下启动轮询 `/debug`。
  - 跨发行版 `tools/automation/test_packages.py`（环境模拟）：容器里**真实安装**——deb→`ubuntu:24.04`
    （apt）、rpm→`fedora:latest`（dnf）、AppImage→本机 `--appimage-extract`；断言可执行文件存在、
    模型 7/7、`ldd` 无缺失。容器运行时用 `docker`（GitHub runner 自带）；本机 `DOCKER=podman`。
- **UI 字符串门禁** `tools/automation/i18n_lint.js`（`-Gate ui` 内）：规则见 AGENTS.md §4。
- **Python 脚本输出编码**：`tools/automation/` 里的脚本大量打中文（门禁项名、报错原因、
  产物清单），而 **Windows runner 的 Python 默认按控制台代码页 cp1252 输出** —— 一句
  `print("门禁全部通过")` 就是 `UnicodeEncodeError`，而且是**门禁跑完了才炸**，看起来
  像门禁本身不通过。本机控制台是 cp936/UTF-8，所以本地永远绿。因此每个会打中文的脚本
  都要在 import 之后、任何 print 之前调 `_utf8.force()`（`tools/automation/_utf8.py`，
  用 `errors="replace"` 免得子进程的 OEM 文本反过来把门禁弄崩）；三个 workflow 的 `env:`
  里另有 `PYTHONIOENCODING: utf-8` 作双保险（但它只对 CI 有用，本机跑脚本时靠的是脚本
  内部那次调用）。
  **验证办法**：`PYTHONIOENCODING=cp1252 python tools/automation/check.py -Gate version`
  —— 这正是 Python 决定 stdout 编码的环境变量，效果与 runner 上的 cp1252 控制台一致。

---

## 4. 与 PureVox 相关的待决问题

以下问题需要先讨论再动手，决定后写回本文并删掉对应条目：

（暂无）

已决（写回 §1/§3）：
- 用 **Tauri 3 alpha**（`3.0.0-alpha.5`）+ **wry**（系统 WebView2，包小）。
- 音频引擎**用 Rust 原生重写**（设备 I/O + onnxruntime Rust 绑定），不挂 Python sidecar。
- **Linux 打包依赖**：wry 需 WebKitGTK 4.1、cpal 需 ALSA、rpm 需 rpmbuild，托盘不再需 appindicator
  （见 §3.2）；Linux 出包用 `ubuntu-24.04`（24.04），**要求 glibc ≥ 2.39**（onnxruntime 预编译包所致）。
- **多平台 CI + 无自动触发**：测试手动、打包 tag 触发，三平台（Windows / Linux / Android），
  无 Lite 变体（见 §3.7）。

已决定：Tk 桌面 UI 与全部旧 Python 代码已从主线删除（2026-10-07），只保留归档快照。
