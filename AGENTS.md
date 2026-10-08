# PureVox — AI 麦克风降噪（Tauri 3 迁移期）

实时 AI 音频降噪 / 目标说话人提取 / 回声消除的桌面应用。当前处于**框架迁移期**：
旧的 Python + Tkinter 实现已整体归档，主线从零开始用 Tauri 3 重建。

**栈**：Tauri 3（alpha）+ Rust 后端 + WebView 前端（Windows WebView2，wry 运行时）
**工程**：`src-tauri/`（编译/运行/打包命令与工具链安装见 `TAURI3.md`）
**状态**：Windows 上调试面板与 HTTP 调试接口已实现（系统指标、GPU、设备枚举）；输入采集 + 重采样到
48 kHz（16k / 44.1k / 48k 三路并发已测）；输出（测试音或任意输入源 → 输出设备，含时钟伺服，虚拟与
真实设备均已测）；**降噪模型（ONNX，`ort`）已接入输入采集**（实测 1.8 ms/hop）；目标说话人（TSE）与
回声消除（AEC）模型未接入。

> 文档索引：
> - `DESIGN.md`：顶层设计（分层、数据不变量、插件式链路 Stage/Pipeline、信号节点模型、
>   SessionPlan、设备生命周期与热插拔、模型生命周期、内存管理、扩展指南、实施顺序）。
>   **实现与 DESIGN.md 冲突时以 DESIGN.md 为准**；改设计先改它再改代码。**开工前先读**。
> - `TAURI3.md`：Tauri 3 现状、文档来源与固化方式、版本锚点、编译步骤、踩坑。**开工前先读**。
> - `legacy-v2026.09.30.1944/`：旧实现最后版本（含当时的 AGENTS.md / DESIGN.md / 使用手册），
>   迁移任何功能前先在这里查清旧行为与设计意图。

---

## 1. 调试优先（迁移期最高准则）

我们对新框架和系统行为都还不了解，所以**调试信息永远可见、调试接口永远可用**：
看不见的状态等于不存在的状态。本节在迁移期内不得删减、不得默认关闭，
release 构建同样保留（不允许用编译开关剔除）。

### 1.1 UI 常驻调试面板

- 主界面调试面板由顶栏「调试面板」开关显隐（状态记忆，**默认收起**，收起时不占列宽度、不挤占横向空间；
  展开时更宽以免文字省略）；内容与 1.2 节接口**同源同字段**，随时可展开，迁移期不可移除。
- 数值固定格式、固定行数刷新（不随数值长度跳动布局），刷新频率 1~10 Hz，不得卡 UI。
- 采集失败的项显示「不可用 + 原因」，**禁止显示伪造的 0**。

### 1.2 本机 HTTP 调试接口

- 只绑 `127.0.0.1`（禁止监听 `0.0.0.0` 或局域网地址），只读 JSON，无鉴权、无写操作。
- 端口 **47821**（`http://127.0.0.1:47821/debug`），唯一定义在 `src-tauri/src/debug/mod.rs` 的
  `DEBUG_HTTP_PORT`，改动须同步本节；端口被占用时日志报错并在 UI 面板「调试接口」行显示原因，不静默失败。
- 每项指标是一个 Probe：`{"state":"ok","value":…}` / `{"state":"pending"}`（如 CPU、GPU 占用率首轮差分前）/
  `{"state":"unavailable","reason":"…"}`。未知路径返回 404 + 端点列表，非 GET 返回 405。
- 端点（每个返回体都带 `ts`（毫秒时间戳）与 `uptime_ms`）：

| 端点 | 内容 |
| --- | --- |
| `GET /debug` | 汇总：下列全部端点的合集 |
| `GET /debug/system` | **CPU**：系统总占用、本进程占用、逻辑核数；**内存**：系统总量/可用、本进程工作集与私有字节；**GPU**：每块 GPU 的名称、占用率、显存总量/已用（含本进程） |
| `GET /debug/audio` | 每路音频流：方向、设备、**信号源（仅输出）**、状态、降噪状态（字段保留但已废弃：降噪现为列处理行，见 `audio.engine`）、设备原生格式、实测**设备侧 / 引擎侧速率**（3 s 窗口）、重采样器与其延迟、**输出流的时钟伺服修正（ppm）**、回调块大小（最近/最小/最大）、帧与 hop 计数、不足一 hop 的剩余帧、峰值/RMS 电平、**输出流欠载补静音样本数与重同步次数**、溢出/丢弃样本、流错误计数、48k 缓冲水位与**设备侧缓冲水位**、端到端延迟、**模型推理耗时（均值/最大）**；最近 50 ms 波形与平均频谱（481 bin，NFFT 960，输入流为降噪后、输出流为重采样前） |
| `GET /debug/devices` | 全部枚举到的输入/输出设备：ID、名称、接口、原生采样率/声道、是否默认、是否被选中、是否已打开；以及最近一次枚举的时间与耗时 |

- 字段只增不改名；新增字段同步更新本表。
- 快照另带 `ui`：界面自报的最近消息数组（新在前，最多 50 条，见 `ui_report` 命令）；
  `audio.engine`（列概要）含每列 `hop/s` 节拍速率（正常 100，见 DESIGN.md §3.3）；
  `recorder`（TSE 参考录制进度/结果）与 `calib`（AEC 延时校准进度/结果），以及
  `virtual_mic`（Linux 虚拟驱动状态：sink / 真源是否已创建；非 Linux 为 `unavailable` + 原因）。
- 另有 `net`（手机 ⇄ 电脑，DESIGN.md §4.1）：服务端口、已连接客户端数、订阅音频数、
  进出包与样本计数、进／出站水位、欠载与丢弃数、已注入按键与字符数、客户端自测 RTT、
  `remote_input` 开关状态、最近错误原因，以及 `codec`（Opus 版本 + 实际加载到的
  `opus.dll` 路径；加载不到时为 `unavailable` + 原因）。
  **注意**：`net` 是用户可见功能，**绑所有网卡的 59123**（手机要能连），
  与本节「只绑 127.0.0.1」的调试接口是两回事；远程输入另有独立开关且**默认关闭**。

### 1.3 实现约束

- **单一数据源**：Rust 侧 `DebugHub`（`src-tauri/src/debug/mod.rs`）维护唯一快照，UI 面板
  （Tauri 命令 `debug_snapshot`，2 Hz 轮询）与 HTTP 接口序列化的是**同一个 `DebugSnapshot`**；
  前端（`src-tauri/ui/debug.js`）只渲染，禁止自己另算一套。
- **采集来源（Windows）**：CPU / 内存用 `sysinfo`，私有字节用 `GetProcessMemoryInfo`；GPU 适配器与
  本进程显存用 DXGI，整卡占用率与显存已用用 PDH 计数器 `GPU Engine` / `GPU Adapter Memory`
  （任务管理器同源）；设备用 `cpal`（后续音频 I/O 同用此库）。设备刷新单一入口
  `devices::spawn_refresh`（启动时 + 面板「刷新」按钮），后台线程执行。
- 「本进程」指标不含 WebView2 子进程（界面渲染在 `msedgewebview2.exe`）。
- **采集链路**（`src-tauri/src/audio/`）：设备按原生默认格式打开 → 回调线程只做下混单声道 +
  写 `rtrb` 无锁环 + 原子计数 → 每路一个工作线程（独占 cpal 流）用 rubato `Async` sinc 重采样到
  48 kHz（`resampler::ToHops`，引擎唯一重采样实现；原生 48k 直通）并在 48k 输出侧切 480 样本 hop。
  流健康以「回调是否持续到达」判定（停顿 ≥ 1 s 为不可用）；WASAPI 偶发的不连续标志只计入流错误数，
  不判失败。
- **输出链路**（`src-tauri/src/audio/playback.rs`，播放正确性只此一处）：源扇出（`fanout.rs`）→
  48k 订阅环 → 每路工作线程用同一 [`Converter`] 重采样到设备采样率（含 48k→48k，因需调比例）→
  设备环；**设备回调是唯一主时钟**，回调里只取样、缺数据补静音（不在回调里写任何缓冲策略）。
  工作线程策略：预热到「目标水位 + 设备环」再开声；PI 伺服按 48k 环水位微调重采样比例限幅 **±3%**
  消化源/设备时钟差；源断流则回预热重同步；48k 环超 300 ms 封顶丢最旧。伺服目标水位 40 ms。
- **Linux 设备面**：cpal 在 Linux 只有 ALSA，枚举会把插件 PCM 与同一张卡的多别名全列出来（本机 108 条）。
  `devices::simplify` 精简为「每个去重后的设备名一条」：丢 `null/lavrate/samplerate/speexrate/jack/oss/
  pipewire/pulse/speex/upmix/vdownmix/dmix/dsnoop/usbstream/iec958/surround*` 等插件，同卡按名去重，
  优先 `plughw`（自动格式转换）> `hdmi` > `hw` > `sysdefault` > `front`，`default` 保留。其它平台枚举本就干净，不动。
- **Linux AEC 远端回环**（`audio/loopback_linux.rs`）：cpal 没有「监听某个 sink」，改用 `parec` 监听输出的
  monitor（`float32le` 48k 单声道）再喂进与 cpal 采集同一个工作循环（重采样/计量/扇出一致）。`loopback` =
  系统默认输出、`loopback:<sink>` = 指定 PipeWire sink；候选由命令 `list_loopback_targets` 给出
  （Linux = `pactl list sinks`，Windows = cpal 输出设备）。Windows 侧仍是 WASAPI loopback（`audio/loopback.rs`）。
- **虚拟驱动页（顶栏「驱动」）**：按平台切同一面板——Linux（`audio/virtual_mic.rs` + `ui/drivers_linux.js`）
  照 legacy 建 `purevox_out` null-sink（`pw-cli create-node adapter`）+ monitor + `pactl module-remap-source`
  真源 `purevox_mic`，移除 = `pactl unload-module` + `pw-cli destroy`；Windows（`ui/drivers_windows.js`）
  是 VB-CABLE 驱动下载 / 视频教程 / 有无检测。两平台实现分文件，外壳 `ui/drivers.js` 按 `get_platform` 切，
  按钮与面板位置、布局一致。
- **会话入口**：计划（`plan.rs`）+ 配置（`config.rs`，存 `~/.purevox/session.json`）→ 命令
  `get_plan` / `apply_plan`（结构性变更 → 重建整个会话）。列 UI（`ui/columns.js`）编辑计划：
  每列两端固定为输入/输出、中间可加/删/移行；输入/输出行选设备，处理行选型号。
  测试音是输入行的一种（`ptype = tone`，全局共享线程）。
- **网络（手机 ⇄ 电脑）**（`src-tauri/src/net/`，协议见 DESIGN.md §4.1）：一个 axum WebSocket
  服务，端口 **59123**，路径 `/ws`（另有 `/health` 自检）。三条能力共用一条连接：
  `remote_mic` 输入行（手机麦克风→电脑，**不需要虚拟声卡驱动**，直接进引擎可降噪）、
  `remote_speaker` 输出行（电脑→手机扬声器，按位置 tap）、远程输入（`text` 打字 + `key`
  全尺寸键盘，**两件事分开**，用 `net_set_remote_input` 开关且**默认关闭**）。
  命令 `net_start` / `net_stop` / `net_status` / `net_set_remote_input`；状态进快照 `net` 段。
  - **Opus 不引入 C 编译链**：`net/opus_sys.rs` 运行时动态加载——Windows 用
    `LoadLibraryExW` + `GetProcAddress` 加载预编译 x64 `opus.dll`（`src-tauri/opus.dll`，
    随包分发；`tauri.conf.json` 的 resources 已登记；查找 `<exe 同目录>/opus.dll` →
    仓库 `src-tauri/opus.dll`），Linux 用 `dlopen`/`dlsym` 加载系统 `libopus.so.0`
    （Debian 包名 `libopus0`；缺失时网络音频降级为明确原因，其余功能正常）。
    注意 `opus_encoder_get_size` / `opus_decoder_get_size` 是**兼容桩恒返回 0**，
    一律用 `*_create`（libopus 内部分配）。**帧长不固定**：PC 发 10 ms（= 1 hop），
    接收端按样本累积后重切 hop（Android 的 opus 常见 20 ms），10 ms 网格不受影响。
  - **主时钟在引擎侧**：网络任务只把解码后的样本塞进队列（进出各有界：目标 50 ms、
    硬顶 80 ms，超限丢最旧），由列工作线程按自己的 10 ms 节拍取；欠载补静音。
  - **出站节拍用绝对时刻**（`next += 10ms` 再 `sleep_until(next)`），不要
    `sleep(10ms - 本轮已用)`：后者实测只产 ~64 包/s（列工作线程与反馈端同理，§3.3）。
  - **按键注入**（`net/keys.rs`）：`SendInput` + **Set 1 扫描码**（不是虚拟键码，能走真实
    键盘布局并区分左右修饰键）；文本用 `KEYEVENTF_UNICODE` 逐 UTF-16 码元注入（中文、emoji
    都能打，且不经过电脑端输入法）。映射表在 `net/keymap.rs`（105 键，Android 键码常量
    **从 AOSP `KeyEvent.java` 逐个提取，勿手抄**）。注入后端按平台分：Windows 已实现，
    其余平台返回明确的不可用原因（Linux 需 uinput/XTEST），不静默失败。
  - **安全取舍（明确记录）**：局域网同网段可直连、**无鉴权**，因此远程输入默认关闭、
    界面常驻显示状态、有一键关闭；不要在没有开关保护的前提下默认开启按键注入。
- **推理链路**（`src-tauri/src/infer/`）：用 `ort`（onnxruntime）流式推理。模型契约统一为
  `*_hop [1,480]`（10 ms 波形）+ `cache_in [1,D]`（扁平流式缓存，首帧零起）→ `enh_hop [1,480]`
  （**滞后 1 hop**）+ `cache_out [1,D]`；缓存维度从模型输入读，不写死；STFT 在模型图内。
  降噪是**列里的处理行**（`engine/components/denoise.rs`，`Stage`），在**列工作线程**内惰性加载
  （每行一份，只加载一次）；模型文件选择存计划，改型号 = 结构性变更 → 重建会话。
  现役模型常量 `infer::MODEL_DENOISE`。
- **AEC 行 / TSE 行**：AEC 是**输入行**（`ptype = echo_cancel`，本行 `device` = 近端 mic，
  `params.far_device` = 远端参考：`loopback` = 系统默认输出回环、`loopback:<渲染端点ID>` = 指定输出
  回环（WASAPI loopback，`audio/loopback.rs`，仅 Windows）、其它 = 输入设备；`far_delay_ms` **有符号**
  （正 = 远端超前、向后取历史；负 = 远端缓冲超前、向前取）。far 历史 2 s 采样网格（`engine/aec.rs`）
  **按实时推进**（回环设备空闲不出数据时补零，否则序号会随会话时长越落越后、取窗口永远失败），
  窗口 = mic 采样序号 − 延时，历史不足先退最近段、再没有就直通 mic；far 与 mic 序号同原点
  （会话起步 / 校准开始归零）。TSE 是**处理行**
  （`ptype = tse`，`params.reference` = 参考 WAV，默认 `~/.purevox/tse_reference.wav`，须 48 kHz 单声道；
  10 s → `enr_tok`；无参考直通）。行状态（对齐计数 / 推理耗时 / 参考状态）经 `Stage::status` 进列概要。
- **参考录制 / 延时校准**：命令 `record_tse_reference(seconds)` 录「降噪后、TSE 前」的信号（`recorder.rs`，
  RMS 归一化到 -20 dBFS，峰值不削顶）→ `~/.purevox/tse_reference.wav`；命令 `calibrate_aec_delay()`
  对第一个 AEC 行采集 1.6 s、向被回环的输出送 800→6000 Hz 扫频探针，FFT 互相关**对称搜索**延时（**从零
  重置、固定参考延时、只喂精确窗口 → 测的是绝对延时**，不累加、每次可重复），并
  **自动配平**（近端/远端各自 RMS 归一到 -24 dBFS → 回填 `mic_gain_db` / `far_gain_db`）。去直流 + 带限
  （与探针带一致）+ 带内 RMS 归一化（`engine/calib.rs`）。最近一次缓冲落在 `~/.purevox/calib_last_{mic,far}.f32`
  便于离线排查。进度/结果都在快照里（`recorder` / `calib`），界面据此回填参数并重建会话。
  AEC 行另有 `bypass`（直通，跳过 AEC）供 A/B 对比。
- **onnxruntime 会话必须单线程且关闭自旋**（`with_intra_threads(1)` / `with_inter_threads(1)` /
  `with_intra_op_spinning(false)`）：默认按核数建池并忙等，会占满 CPU、和音频回调抢核，
  实测把 1.8 ms 的推理拖到 >100 ms。改这三项前先读 `infer/denoise.rs` 的注释。
- **不得影响音频**：音频线程只做无锁计数/写环形缓冲，汇总与序列化在采集线程完成；
  系统指标采样周期 1 s。
- **新功能的完成标准**包含调试输出：新增模块（流、设备、模型、效果）必须同时把自身状态
  接入 1.2 节对应端点，否则视为未完成。
- **验证方式**：智能体与脚本验证运行状态一律请求 HTTP 接口（`curl http://127.0.0.1:<端口>/debug`），
  不靠模拟键鼠点界面（Windows 会拦截后台抢焦点，按键会打进别的窗口）。界面交互验证用
  Windows UI Automation，详见 `TAURI3.md` 3.6 节。**少截图**，优先用调试接口读数。
- **测试自收尾**：任何启动进程的 shell 测试必须自己结束（PowerShell 用
  `try { … } finally { Get-Process purevox -ErrorAction SilentlyContinue | Stop-Process }`），
  命令一律带超时，后台任务不得残留；**不允许把运行中的 app 留给用户手动关闭**。
- **前端调试**：`ui/jsdebug.js` 把脚本错误、未处理的 Promise 拒绝、`console.error/warn` 转发到
  `ui_report`（即快照的 `ui` 数组）；顶栏「开发者工具」打开 WebView2 devtools（Cargo 已开
  `devtools` 特性，release 保留）。JS 静态检查用项目 `opencode.json` 的 LSP + `ui/jsconfig.json`
  （`checkJs`，配合 `ui/globals.d.ts`）。
- **外围**：系统托盘（启动/停止、显示/隐藏、退出；图标随运行状态在 `icons/tray_running.png` /
  `tray_stopped.png` 间切换，菜单随语言与状态重建）；**窗口最小化/关闭都收到托盘**（不退出）；
  开机自启（`autostart.rs`，写/删 `HKCU\...\Run`）；**启动/停止**（命令 `set_running`/`get_running`，
  界面顶栏按钮（启动绿/停止红）、托盘、全局热键共用；快照带 `running`）；**全局热键**（`hotkey.rs`：
  `RegisterHotKey` + 规范串「`Ctrl+Alt+Shift+Win` 顺序」，空串=不监听，注册结果进 `ui`）；
  **提示音**（`cues.rs` 自合成 6 套预设，start 上行 / stop 下行，`PlaySound` SND_MEMORY）；
  **设置**（`~/.purevox/settings.json`：语言 / 热键 / 提示音；顶栏「设置」面板，键位可录制）；
  中英文（`ui/i18n.js`，中文即 msgid，顶栏「EN/中」）。应用图标 `icons/`（`cargo tauri icon`
  从源图生成，源图 = 深蓝渐变圆角矩形 + PureVox 像素 P）。打包用 `cargo tauri build`。

---

## 2. 维护准则

1. **一个功能只有一条实现路径**。有多种做法时只保留一种并写进文档，被替代的实现直接删除，不留备选。
2. **先扩展，再新建**：先看已有函数/模块能否扩展，确认不能才新建。
3. **legacy 快照只读冻结**：`legacy-v2026.08.20.1943/`（音频链条化重构前）与
   `legacy-v2026.09.30.1944/`（Tauri 迁移前最后的 Tk 版本；现役实现不再引用的旧二进制也
   集中收纳在这里，见 §2.5）禁止修改、删除，不参与构建、CI、打包、测试及任何批量改动
   （格式化、换行符归一、重命名等一律绕过）。只读查阅。
4. **迁移期不写用户更新日志**：旧的 `about/changelog.md` 已随旧实现归档；迁移进度记在 `TAURI3.md`。
   新版具备用户可见功能后再恢复更新日志，届时在本节写明位置。
5. **旧实现遗留二进制**（新实现不再引用的一律收进只读快照 `legacy-v2026.09.30.1944/`，
   现役树里只留仍在用的）：
   - **现役引用**：`server/opus.dll`（Windows 网络 Opus，运行时加载，见 §1.2）、
     `android/gradle/wrapper/gradle-wrapper.jar`（Android 客户端构建，**勿替换**）、
     `assets/icons/audio_icon_base.png`（`cargo tauri icon` 源图）。
   - **已收进快照**（新实现不引用，现役树里对应文件已删除，只存快照备查）：
     `assets/icons/{audio_icon.ico, tray_running.ico, tray_stopped.ico, lite_tray.ico, lite_tray.png}`、
     `assets/fonts/ark-pixel-12px-monospaced-zh_cn.ttf`、`html/wasm/libopus-encoder.wasm.min.wasm`。
     现役图标是 `src-tauri/icons/`、界面用系统字体（不捆绑字体文件），与旧资产**无重复**。
6. **发版 tag**：主线 `v<yyyy.MM.dd.HHmm>`。CI 失败、从未生成 release 的 tag 必须删除
   （`git tag -d <tag> && git push origin :refs/tags/<tag>`），否则会截断下一个 release 的提交记录。
   **应用/包版本号 = 日期**：由 `tools/automation/version.{sh,ps1}` 从 tag（或本机当前 UTC）推导——Tauri
   的 `version` 必须是 semver，故取 `yyyy.MMdd.HHmm`（如 tag `v2026.10.08.1430` → 版本 `2026.1008.1430`），
   写入 `src-tauri/.build-version.json` 并由 `cargo tauri build --config` 覆盖（不提交该文件）；窗口标题与
   调试面板「版本」走 `PUREVOX_BUILD_VERSION`（见 `src/version.rs`）。
7. **CI 与门禁（多平台：Windows + Linux + Android；测试与打包解耦，无自动触发）**：
   - **触发纪律**：**没有任何分支推送 / PR 触发**。`tests.yml`（只测试、不打包）只
     `workflow_dispatch` 手动触发；`release.yml`（打包发版）由 tag `v*` 触发、也可手动；
     `warm-cache.yml`（唯一缓存写入者）也只手动。日常提交零成本，要验证就手动 dispatch 一次。
   - 门禁唯一实现 = `tools/automation/check.ps1`（**跨平台 PowerShell**：`fmt` /
     `clippy`（`-D warnings`）/ `build` / `test` / `ui`（`tsc` + `i18n_lint`））；
     CI 与本机（Windows 与 Linux）跑同一份，按 `-Gate` 拆 step 便于定位日志。
   - 工作流（`.github/workflows/`）：
     - `tests.yml`：手动 → Rust 门禁矩阵 `windows-latest` + `ubuntu-latest`
       （fmt/clippy/build/test）、`ui` 门禁（`ubuntu-latest`：setup-node → `-Gate ui`）。**不打包**。
     - `warm-cache.yml`：**全仓库唯一缓存写入者**，手动；矩阵 Windows+Linux 冷构建
       target，预热 debug 三门禁 + `cargo tauri build`，再 save 四个桶（cargo registry /
       target-debug / target-release / cargo-tauri binary；`continue-on-error`，同键已存在即成功）。
     - `release.yml`：tag `v*` 或手动 → 三平台并行：
       **Windows**（门禁 + `cargo tauri build` + `assert_bundle.ps1 -Smoke` → MSI/NSIS）；
       **Linux**（`ubuntu-latest`：装 WebKitGTK/ALSA/rpm 依赖 + 门禁 + `cargo tauri build` +
       `assert_bundle_linux.sh --smoke` + `test_packages.sh`（deb→ubuntu / rpm→fedora 容器安装验证）→
       deb/rpm/AppImage）；
       **Android**（JDK17 + `install_android_sdk.sh` + `./gradlew assembleDebug` → debug APK）。
       tag 时再 `release_notes.ps1` 生成说明并 `gh release create` 附带全部产物；手动触发只打包、不建 release。
       **没有 Lite 变体**（其遗留资产已收进只读快照 `legacy-v2026.09.30.1944/`，见 §2.5）。
   - **缓存纪律**：`tests.yml` / `release.yml` 只 restore、不写缓存；键为
     `purevox-<RUNNER_OS>-<桶>-<CACHE_GEN>-…`。缓存桶与全部版本（含 `JDK_VER` /
     `ANDROID_PLATFORM` / `ANDROID_BUILD_TOOLS`）的唯一来源 = `tools/automation/versions.env`
     （其余 workflow 不得硬编码版本或直写缓存）。
   - **Linux 系统依赖**（本机与 CI 同）：`libwebkit2gtk-4.1-dev libssl-dev libxdo-dev
     librsvg2-dev libgtk-3-dev libasound2-dev pkg-config file rpm`（wry→WebKitGTK、
     cpal→ALSA、rpmbuild→rpm）；运行期还需系统 `libopus0`（缺时网络音频降级，见 §1.2）。
   - `legacy-*/` 快照与 `*.md` 改动不进 CI。

---

## 3. 沿用的硬约束

以下约束来自模型契约与产品定位，与框架无关，新实现必须遵守：

- **音频格式**：引擎内部一律 F32 单声道 48 kHz；设备原生格式只在平台层转换
  （重采样、下混），模型永远拿 48k 单声道。
- **10 ms hop 网格**：`hop = SAMPLE_RATE / 100`（48 kHz 下 480 样本，FFT/OLA 窗长 2×hop = 960），
  按时间派生而不是写死样本数。所有数据面（引擎帧、设备回调块、网络帧、缓冲水位、频谱可视化）
  都按 10 ms 整数倍前进，不得引入 1024/2048 等错位块。
- **模型**：`models/*.onnx` 为现役模型文件，不随 GPL 授权（见 `MODEL-LICENSE.md`），禁止提取用于其他项目。

---

## 4. 命名与文件规范

- **品牌名统一 `PureVox`**（P、V 大写）：窗口标题、UI、日志、文档一律如此，禁止 `Purevox` / `PUREVOX` 等变体。
- 平台/协议强制小写的标识保持小写：用户数据目录 `~/.purevox/`、Tauri `identifier`
  `com.purevox.desktop`、Android 包名 `com.purevox.mic`、模型代号 `purevox9` 等。
- **代码命名**：Rust snake_case 函数/变量、PascalCase 类型；JS/TS camelCase。
- **平台特征代码按文件分（`<功能>_<平台>.rs`）**：同一功能有平台差异时，各平台实现放进
  `<功能>_windows.rs` / `<功能>_linux.rs` / `<功能>_other.rs`（其它平台 = 明确不可用的桩，
  不静默失败），功能文件本身只保留共享部分 + `#[cfg] #[path = "…_<平台>.rs"]` 的平台分派；
  **禁止**在一个文件里用 `#[cfg(windows)]` 包住大段平台实现体。现状（全部按此分）：
  `audio/loopback_{windows,linux,other}.rs`、`audio/virtual_mic*.rs`、`hotkey*.rs`、
  `autostart*.rs`、`cues*.rs`、`net/keys*.rs`、`net/opus_sys*.rs`、`devices*.rs`、
  `debug/{mem,gpu}_*.rs`、`openurl_*.rs`。前端同理：`ui/drivers_linux.js` / `ui/drivers_windows.js`。
- **许可证头**：每个源码文件（`.rs` / `.toml` / `.html` / `.js` / `.ts` / `.css` 等）顶部必须带 GPL-3.0 版权头 +
  模型声明 + `SPDX-License-Identifier: GPL-3.0-or-later`，照抄 `src-tauri/src/main.rs` 顶部并按注释风格替换。
  JSON 无注释语法，豁免。
- **UI 字符串（i18n，沿袭旧实现 `i18n.py` / 旧 AGENTS 规则 13）**：中文字面量即 msgid，
  唯一字典 `src-tauri/ui/i18n.js`（zh 恒等返回、en 查表、缺键回退中文）。上屏只有三个入口：
  静态 `data-i18n="…"`、动态 `T('…')`（可带占位符 `T('列 {n}', { n: 1 })`，中英 `{name}`
  集合必须一致）、Rust 中文模板串 `__pvTpl(raw)`（TPL 表：zh = Rust `format!` 字面骨架、
  en = 同构英文模板、id 为稳定短名供业务逻辑判定，Rust 改拼串必须同步 TPL 骨架）。
  **Rust 提供的固定标签**（注册表节点/参数名、模型表、提示音预设）同样在界面侧经 `T()`
  翻译（中文即 msgid，条目必须在字典里）；**动态写入的文本**（状态串、按钮文字、下拉选项）
  必须同时可重放：记录最近状态 + 监听 `document` 的 `pv-langchange` 事件按新语言重绘。
  **其余中文一律不上屏**；不翻译的边界：注释、调试日志（`__pvDebug` / `ui_report` /
  `console`）、Rust 诊断文本（probe 的 reason 体，由变量拼接不经扫描）。**新增用户可见中文串
  必须同步加 en 条目**（漏了 en 就是漏译）。门禁 `tools/automation/i18n_lint.js`
  （挂 `check.ps1 -Gate ui`）：E1 裸中文上屏、E2/E3 缺键、E4 占位符奇偶、E5 模板 id 重复、
  E6 Rust 标签缺条目为错误；W1 孤儿键、W2 TPL 骨架与 Rust 源脱节为告警。
- **README 双语**：中文 `README.md` + 英文 `README_EN.md`，结构变化时两处同步。

---

## 5. 许可证

- 源码 **GPL-3.0**（`GPL-3.0-or-later`），见 `LICENSE`；第三方声明见 `LICENSE-THIRD-PARTY.txt`。
- 内置 AI 模型（`*.onnx`）归 a2heng 所有，仅随 PureVox 经授权使用，见 `MODEL-LICENSE.md`。
- 作者另有 MIT 模型仓库可自由使用：`lightweight-denoise-48k` / `lightweight-aec-48k`。
