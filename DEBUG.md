# DEBUG — 调试优先与实现约束

> 从 `AGENTS.md` §1 拆出。调试接口、Probe 结构、实现约束、AI 可复现要求都在这里。
> 界面/交互规则见 `UI.md` / `UX.md`；网络见 `NETWORK.md`；插件见 `PLUGINS.md`。

## 1. 调试优先（最高准则）

我们对系统行为永远有不知道的部分，所以**调试信息永远可见、调试接口永远可用**：
看不见的状态等于不存在的状态。本节不得删减、不得默认关闭，
release 构建同样保留（不允许用编译开关剔除）。

### 1.1 独立调试面板（常驻可用）

- 顶栏「调试面板」（命令 `open_debug_window`）用**系统浏览器**打开 `http://127.0.0.1:47821/debug/ui`
  —— 页面是 `ui/debug.html`，由 1.2 节的调试接口自己提供（静态资源随二进制内嵌）。它是**独立进程**：
  只 `fetch('/debug')` 渲染，不经过 WebView2 / IPC，关掉标签页即可，不占主窗口空间。
- **不要退回「第二个 WebView2 窗口」**：Tauri 3 alpha 建第二个 webview 时页面根本不会导航
  （CDP 里目标停在 `about:blank`、`Chrome_WidgetWin_0` 是 0×0，整窗纯白且时好时坏；
  2026-10-26 实测）。解耦也不只是绕坑：面板崩了/卡了不会带上主界面。
- 解耦的代价：面板截不到主窗口的图（跨进程），所以**没有截图功能**（内置截屏已整体移除）；
  抓界面截图用系统截图工具。
- 数值固定格式、固定行数刷新（不随数值长度跳动布局），刷新频率 1~10 Hz，不得卡 UI。
- 采集失败的项显示「不可用 + 原因」，**禁止显示伪造的 0**。

### 1.2 本机 HTTP 调试接口

- 只绑 `127.0.0.1`（禁止监听 `0.0.0.0` 或局域网地址），只读（JSON + 面板静态页面），无鉴权、无写操作。
- 端口 **47821**（`http://127.0.0.1:47821/debug`），唯一定义在 `src-tauri/src/debug/mod.rs` 的
  `DEBUG_HTTP_PORT`，改动须同步本节；端口被占用时日志报错并在 UI 面板「调试接口」行显示原因，不静默失败。
- 每项指标是一个 Probe：`{"state":"ok","value":…}` / `{"state":"pending"}`（如 CPU、GPU 占用率首轮差分前）/
  `{"state":"unavailable","reason":"…"}`。未知路径返回 404 + 端点列表，非 GET 返回 405。
- 端点（每个返回体都带 `ts`（毫秒时间戳）与 `uptime_ms`）：

| 端点 | 内容 |
| --- | --- |
| `GET /debug` | 汇总：下列全部端点的合集 |
| `GET /debug/system` | **CPU**：系统总占用、本进程占用、逻辑核数；**内存**：系统总量/可用、本进程工作集与私有字节；**GPU**：每块 GPU 的名称、占用率、显存总量/已用（含本进程）；**系统时钟粒度** `timer_resolution_ms`（Windows 默认 15.625 ms —— 它决定 `sleep` 的实际精度，见 `NETWORK.md` §3.3.2） |
| `GET /debug/audio` | 每路音频流：**结论（`health`，Rust 侧统一判定的健康度 `{level, text}`，阈值集中在 `audio/health.rs`，面板只渲染不算，见 1.3）**、方向、设备、**信号源（仅输出）**、状态、降噪状态（字段保留但已废弃：降噪现为列处理行，见 `audio.engine`）、设备原生格式、实测**设备侧 / 引擎侧速率**（3 s 窗口）、重采样器与其延迟、**输出流的时钟伺服修正（ppm）**、回调块大小（最近/最小/最大）与**回调间隔（最近/最大 ms，音频回调里锁-free 实测：只看回调次数看不出「卡了 2 秒又继续」）**、帧与 hop 计数、不足一 hop 的剩余帧、峰值/RMS 电平、**输出流欠载补静音样本数与重同步次数**、溢出/丢弃样本、流错误计数、**上述事件的实测每秒速率（`underrun_per_s` / `resync_per_s` / `overrun_per_s` / `stream_error_per_s`，3 s 窗口；累计计数只涨不降，速率才看得出现在还在不在发生）**、48k 缓冲水位与**设备侧缓冲水位**、端到端延迟、**模型推理耗时（均值/最大）**；另有汇总结论 `audio.verdict`（全部流的整体判断）；最近 50 ms 波形与平均频谱（481 bin，NFFT 960，输入流为降噪后、输出流为重采样前） |
| `GET /debug/devices` | 全部枚举到的输入/输出设备：ID、名称、接口、原生采样率/声道、是否默认、是否被选中、是否已打开；以及最近一次枚举的时间与耗时 |
| `GET /debug/calib` | AEC 校准的**结构化报告**（与快照 `calib_report`、`~/.purevox/calib_last.json` 同一份，见 1.4）：`schema_version` / `ok` / `created_ms` / 判定码与结论 / 两路峰值时刻与延时 / 回声路径 / 探针带电平与 SNR / 回填增益 / 平滑后的带内能量包络 `series` / 原始 dump 路径；未跑过时为 `pending` |
| `GET /debug/ui` | **调试面板页面本体**（1.1 用系统浏览器打开它）：HTML + 同源静态资源（`/debug/ui/{debug.js,debug.css,i18n.js,snapshot.js,jsdebug.js}`）。页面只 `fetch('/debug')` 渲染，不经过 IPC；内嵌在二进制里，装完也能用 |

- 字段只增不改名；新增字段同步更新本表。
- 快照另带 `ui`：界面自报的最近消息数组（新在前，最多 50 条，见 `ui_report` 命令）；
  `audio.engine`（列概要）含每列 `hop/s` 节拍速率（正常 100，见 ARCHITECTURE.md §3.3）；AEC 行另带
  **回声诊断**（滚动 1 s 窗）：`远端解释度 ρ`（远端窗口对近端的归一化互相关，判断参考/对齐对不对）
  与 `回声抑制 dB`（10log10(Σ近端²/Σ模型输出²)，判断模型有没有出力）——两者一起看才能分清
  「参考没对齐」还是「麦克风里根本没有回声」；
  `recorder`（TSE 参考录制进度/结果）与 `calib`（AEC 延时校准进度/结果；
  **新代码读 `calib_report`**：结构化、带 `schema_version` 与曲线，见 1.4），以及
  `virtual_mic`（Linux 虚拟驱动状态：sink / 真源是否已创建；非 Linux 为 `unavailable` + 原因）。
- 另有 `net`（手机 ⇄ 电脑，NETWORK.md §4.1）：服务端口、已连接客户端数、订阅音频数、
  进出包与样本计数、**每方向的水位/深度/抖动（EMA + 最坏）/最大到达间隔**、
  **实测单向延迟**（NTP 偏移 + 帧头时间戳）、欠载 / PLC / 裁剪 / 空环到达 / 丢包计数、
  出站每路的待发 hop 与丢弃数、**客户端自报**（环水位、欠载、掩盖、时钟偏差 ppm）、
  时钟偏移估计、RTT、已注入按键与字符数、`remote_input` 开关、最近错误原因，以及
  `codec`（Opus 版本 + 实际加载到的 `opus.dll` 路径）。判定在 Rust 一侧算
  （`hub.rs` 的 `rx_verdict`/`tx_verdict`），面板只渲染。
- 另有 `update`（更新检查，RELEASE.md §4.2）：阶段（`idle` / `checking` / `up_to_date` /
  `available` / `downloading` / `ready` / `installing` / `installed` / `unsupported` / `failed`）、
  本机显示版本与构建时间戳、**本机安装方式 `install_kind`**（`appimage` / `deb` / `rpm` /
  `nsis` …；Linux 由二进制里的 bundle type 标记决定，产物**按它挑**）、
  最新 tag / 显示版本 / 发布日期 / 更新说明、匹配到的产物名与地址、下载字节进度
  （**续传后的总进度**）、已下载路径、失败原因（带「已收 / 总大小」）。
  **下载与检查是两个客户端**：检查 15 s **总**超时；下载**连接 15 s + 空闲 30 s**
  （`IDLE_TIMEOUT_S`，每收到一块就重置）+ **无总超时** + 断点续传 + 重试（`.part` 续传点）。
  把 15 s 总超时套到下载上就是「下载中断：error decoding response body」的根因（2026-10-18
  实测收到 735703/76346174 字节即断；安装包 50~140 MB，慢链路上必超）。**别再合回一个**。
  **注意**：`net` 是用户可见功能，**绑所有网卡的 59123**（手机要能连），
  与本节「只绑 127.0.0.1」的调试接口是两回事；远程输入另有独立开关且**默认关闭**。

### 1.3 实现约束

- **单一数据源**：Rust 侧 `DebugHub`（`src-tauri/src/debug/mod.rs`）维护唯一快照，UI 面板
  （Tauri 命令 `debug_snapshot`，2 Hz 轮询）与 HTTP 接口序列化的是**同一个 `DebugSnapshot`**；
  前端（`src-tauri/ui/debug.js`）只渲染，禁止自己另算一套。
- **采集来源**（`debug/{system,gpu_*,mem_*}.rs`，按平台分文件）：
  - CPU / 内存总量与工作集：两平台都用 `sysinfo`，同一套口径。
  - **私有字节**：Windows `GetProcessMemoryInfo`（`PrivateUsage`，含已换出）；
    Linux `/proc/self/smaps_rollup` 的 `Private_Clean+Private_Dirty+Private_Hugetlb`
    （**只算常驻**，换出后会变小 —— 口径差异写在这里，不假装等价）；
  - **GPU**：Windows 用 DXGI（适配器 / 本进程显存）+ PDH 计数器 `GPU Engine` /
    `GPU Adapter Memory`（任务管理器同源）；Linux 用 `nvidia-smi`（整卡占用 / 显存 /
    计算上下文的进程显存，2 s 缓存）或 `amdgpu` 的 `/sys/class/drm/card*/device/{gpu_busy_percent,
    mem_info_vram_*}`。单进程占用率两家 Linux 驱动都不提供 → 明确 `unavailable`，不编数。
  设备用 `cpal`（后续音频 I/O 同用此库）。设备刷新单一入口
  `devices::spawn_refresh`（启动时 + 面板「刷新」按钮），后台线程执行。
- 「本进程」指标不含 WebView2 子进程（界面渲染在 `msedgewebview2.exe`）。
- **采集链路**（`src-tauri/src/audio/`）：设备按原生默认格式打开 → 回调线程只做下混单声道 +
  写 `rtrb` 无锁环 + 原子计数 → 每路一个工作线程（独占 cpal 流）用 rubato `Async` sinc 重采样到
  48 kHz（`resampler::ToHops`，引擎唯一重采样实现；原生 48k 直通）并在 48k 输出侧切 480 样本 hop。
  **放行的节拍只看这一路自己的时钟**（`capture.rs` 的 `release_realtime` / `clamp_excess`）：
  ① **实时预算** —— 每拍只放「距上次放行这段时间该放的量」（上限一格），有些源是**突发**的
  （`parec` 一次给一整块；100 ms 缓冲时实测 4800 样本/次、现取 30 ms = 1440 样本、~33 次/s），
  一次性倒进扇出会把下游行输入环冲爆、块之间又立刻饿死，`ROW_CAP` 于是静默丢掉 ~80%
  （实测 38333 样本/s，「手机/电脑声音串流吞吞吐吐」的真正原因）；按实时预算放，整块自然被摊成
  平滑 hop 流；② **延迟硬顶** —— 设备环超 `DEV_MAX_MS`（60 ms）就**丢最旧到 `DEV_TARGET_MS`**
  （10 ms）并把丢弃计数（进 `overruns` + 速率）。**别退回「拿下游水位当放行开关」**：
  放行速率恰好等于源速率时水位是随机游走，一旦被垫高就永不回落 —— 流刚打开时设备预填的那
  一大块会**永久**留在环里，实测单列麦克风监听长期 289 ms（v18 的线上表现）。
  流健康以「回调是否持续到达」判定（停顿 ≥ 1 s 为不可用）；WASAPI 偶发的不连续标志只计入流错误数，
  不判失败。
- **流健康度（结论）只有一处实现**（`audio/health.rs`）：`Health { level, text }` 全由 `annotate`
  统一判定（回调间隔、速率偏差、欠载/重同步/丢弃/错误的**每秒速率**），汇总结论 `overall` 同理；
  阈值是那两条常量（`RATE_TOLERANCE` / `EVENT_EPS`）。**面板与 HTTP 序列化的是同一份判定结果**，
  UI 只按 `level` 上色，禁止自己再算一套（1.3 单一数据源）。回调间隔在回调线程里量：每路一个
  `Instant` 起点 + 相对纳秒的原子写，不加锁、不分配、不进音频路径。
- **输出链路**（`src-tauri/src/audio/playback.rs`，播放正确性只此一处）：源扇出（`fanout.rs`）→
  48k 订阅环 → 每路工作线程用同一 [`Converter`] 重采样到设备采样率（含 48k→48k，因需调比例）→
  设备环；**设备回调是唯一主时钟**，回调里只取样、缺数据补静音（不在回调里写任何缓冲策略）。
  工作线程策略：预热到「目标水位 + 设备环」再开声；PI 伺服按 48k 环水位微调重采样比例限幅 **±3%**
  消化源/设备时钟差；源断流则回预热重同步；48k 环超 300 ms 封顶丢最旧。伺服目标水位 40 ms。
- **Linux 设备面（Pulse/PipeWire，OBS + 系统声音设置同源）**：不再遍历 ALSA 插件 PCM（本机曾有 108 条）。
  用 `pactl -f json list sources/sinks` 取每台设备一条；**输入一律用 `pulse:<source>`**（`parec` 采集，
  PipeWire 混音，浏览器/OBS/PureVox 可同时用麦克风，不再因 ALSA `plughw` 排他而 busy）；输出暂时仍映射到
  `alsa:plughw:CARD=…,DEV=…`（输出要设备时钟伺服；PipeWire 输出通路待做）。
  **设备名必须取系统同一份**：`pactl -f json` 的 `description` 在 PipeWire 上常是 `(null)`，改用
  `pactl list sources|sinks` 文本的 `Description:`（`pa_source_info.description`），并用 `pw-dump` 的
  `node.description` 兜底——否则会出现三只「USB Audio」。非 ALSA 输出（蓝牙/虚拟 sink）当前明确跳过，
  不伪装成可打开的设备。
- **Linux AEC 远端回环**（`audio/loopback_linux.rs`）：cpal 没有「监听某个 sink」，改用 `parec` 监听输出的
  monitor（`float32le` 48k 单声道）再喂进与 cpal 采集同一个工作循环（重采样/计量/扇出一致）。`loopback` =
  系统默认输出、`loopback:<sink>` = 指定 PipeWire sink；候选由命令 `list_loopback_targets` 给出
  （Linux = `pactl list sinks`，Windows = cpal 输出设备）。Windows 侧仍是 WASAPI loopback（`audio/loopback.rs`）。
  **`parec`/`pacat` 必须压低缓冲，取值唯一来源 `loopback::PARE_LATENCY_MS` / `PARE_PROCESS_MS`
  （现为 30/10，探针复用同一对常量）**：默认缓冲 ≈ 1.9 s，far 会比 mic 晚近 2 s，
  校准的 ±1 s 搜索窗够不着 → 一直「相关太弱（0.000）」（Windows WASAPI 回环无此问题）。
  **这对值同时就是这一路回环的固定延迟下限**（100 ms 时块 = 4800 样本，电脑声音串流听得出拖一截；
  30 ms 时块 = 1440 样本）；别再往下压，太小会开始欠载（丢弃/补零在 `/debug/audio` 有统计）。`pactl` 解析一律
  `LC_ALL=C`（中文 locale 下输出「名称/描述」，按 `Name:` 会解析出 0 个 sink）。
- **虚拟驱动页（顶栏「驱动」）**：按平台切同一面板——Linux（`audio/virtual_mic.rs` + `ui/drivers_linux.js`）
  照 legacy 建 `purevox_out` null-sink（`pw-cli create-node adapter`）+ monitor + `pactl module-remap-source`
  真源 `purevox_mic`，移除 = `pactl unload-module` + `pw-cli destroy`；Windows（`ui/drivers_windows.js`）
  是 VB-CABLE 驱动下载 / 视频教程 / 有无检测。两平台实现分文件，外壳 `ui/drivers.js` 按 `get_platform` 切，
  按钮与面板位置、布局一致。
- **会话入口**：计划（`plan.rs`）+ 配置（`config.rs`，存 `~/.purevox/session.json`）→ 命令
  `get_plan` / `apply_plan`（结构性变更 → 重建整个会话）。
  **行级热参数**（与模型无关的标量：`gain` 行的 `gain_db`、`loopback` 行的 `volume_db`）走命令
  `set_row_param` + `engine/hot.rs` 的 `HotGain` 原子单元格：**不重建会话**、下一 hop 生效；
  判定唯一来源 `registry::is_hot`，界面（`ui/columns.js` 的 `p.hot`）照它选路径。
  **别把它并回 `apply_plan`**：全量重建要关掉全部设备再重开，Linux 上 ALSA 排他、几百毫秒起步
  （2 列撞同一张卡实测 7.25 s/次），改增益每 1 dB 重建一次会把命令队列堆死
  —— 用户看到的就是「改音量卡死、连停止都点不动」（2026-10-18 实测）。列 UI（`ui/columns.js`）编辑计划：
  每列两端固定为输入/输出、中间可加/删/移行；输入/输出行选设备，处理行选型号。
  **启用开关**：行级 `RowSpec.enabled` 已有（运行时逐 hop 生效，媒体行还转发给 `MediaSlot`）；
  列级 `ColumnSpec.enabled` **已接入**（列头开关）—— 语义是「保留配置、放掉这一列对设备的引用」，
  不是删除、不是全量重建（见 ARCHITECTURE.md §6.5「启用 / 停用」）。
  测试音是输入行的一种（`ptype = tone`，全局共享线程）。
- **设备接管 = 设备池**（`audio/device_pool.rs`，2026-10-19 已实现，ARCHITECTURE.md §6.5）：
  每个 `(设备 ID, 角色)` **只打开一次**，按引用计数持有；逻辑行只拿**租约**（输入行 `InputLease`
  拿扇出订阅、输出行 `OutputLease` 往该设备**唯一**那条混音登记一路源）。于是
  「同一设备被多列用」不再报 busy（第二次是订阅/混音），**会话重建也不再关设备**
  —— 重建是**先建新会话、再停旧会话**，没变的设备复用同一条流（`started_at` 都不变 = **差异重启**）。
  流标识随设备走（`input:<设备>` / `output:<设备>`），不再带列/行（列会增删，流不会）。
  角色只有**输入 / 输出**：同一张全双工声卡同时做输入与输出**合法**（两个 PCM）。
  **别再把 `capture::spawn` / `playback::spawn` 直接放进逻辑行** —— 那等于把「硬件独占」
  焊回逻辑结构。**回环（`loopback` 行 / AEC 远端）与 AEC 行的 mic/far 仍各自持有**（待做）。
- **输出混音**（`audio/mixer.rs`，同批）：一个输出设备一条混音 —— 每源一路 48k→48k **可调比例**
  重采样 + **累计器水位伺服**（±3%），对齐到设备时钟后逐 hop 相加；缺料的源补零、不拖慢别人。
  **不许直接相加**：各源时钟有 ppm 级差异，直接相加会一路涨一路空，最后只能靠丢/补收拾
  （听感是周期性咔哒）。相加**不自动限幅**（两路同一信号 = +6 dB，要平衡用行内增益）。
- **列头开关 / 行开关**（`ColumnSpec.enabled` / `RowSpec.enabled`）= 订阅与设备引用的**挂摘**：
  不产出 + 放掉这一份引用（池归零才真关），**不是删除**（行 / 设备选择 / 参数全留，随时开回来）；
  运行时 `acquire` 失败就停在停用态 + 明确原因（不许「点了没反应」）。
- **会话重建 / 设备打开**：`start` / `stop` / `apply_plan` 仍共用一把 `rebuild` 互斥锁（界面连改参数
  会并发触发 `apply_plan`）；**但不再需要「先停旧会话再建新会话」**（排他由设备池承担）。
  打开设备走 `with_device_retry`（对 `busy` / `not available` / `disconnected` 退避重试 ~3 s）
  —— 这条现在只会来自**别的进程**占用（各平台的打开语义差异见 ARCHITECTURE.md §6.5 的实现差异表）。
- **AEC 远端取值**：`far_device` 缺省 = `loopback`（系统默认输出）——AEC 要消除的就是输出设备的声音，
  界面也不再提供「未选择」。**远端是否接受裸输出设备 ID 按平台**（`devices::loopback_accepts_device_id`）：
  Windows = WASAPI 端点可以（`loopback:<端点>` 或裸端点 ID），Linux = **只认 `loopback` / `loopback:<Pulse sink>`**
  （填 ALSA 设备 ID 会拼出 `<id>.monitor` 这种无效 monitor，far 永远取不到）。
- **网络（手机 ⇄ 电脑）**（`src-tauri/src/net/`，协议 / 卡片 / TLS 见 NETWORK.md §4.1、§4.1.11、
  §4.1.12）：一个 axum 服务绑 `0.0.0.0`，默认端口 **59123**（界面可改，持久化到
  `~/.purevox/settings.json`），路径 `/ws`（另有 `/health` 自检、`/ca-cert` 下载 CA、
  `/libopus-wasm.js` 给网页版、`/` 是网页版客户端）。三条能力共用一条连接：
  `remote_mic` 输入行（手机麦克风→电脑，**不需要虚拟声卡驱动**，直接进引擎可降噪）、
  `remote_speaker` 输出行（电脑→手机扬声器，按位置 tap）、远程输入（`text` 打字 + `key`
  全尺寸键盘，**两件事分开**，用 `net_set_remote_input` 开关且**默认关闭**）。
  命令 `net_start` / `net_stop` / `net_restart` / `net_status` / `net_card` / `net_running` /
  `net_set_remote_input` / `net_set_port` / `net_ifaces` / `net_select_ip` / `net_local_addr` /
  `net_ca_pem`；状态进快照 `net` 段（`net_card` 是卡片用的结构化短字段，与它同源）。
  - **重连继承环深度**：`ready.target_ms` 必须是 `hub.ring_ms()`（电脑设置值），不能写死常量
    20，否则手机重连后用户调好的 60/90 ms 被拉回 20。**断开时入站计数归零**：
    `RateMeter` 遇到 `total < 上一窗口` 必须丢旧窗口重算（u64 下溢 → 主线程 panic →
    WebView2 回调里 `panic in a function that cannot unwind` → **exe 秒退**）；
    `NetHub` 的锁一律 `unwrap_or_else(into_inner)`，网络线程的 panic 不许带死主线程轮询。
  - **只监听 HTTPS，证书自签（CA + 叶两级）**（NETWORK.md §4.1.12）：网页版客户端的麦克风靠
    `getUserMedia`，它**只在安全上下文里可用**，`http://<局域网IP>` 浏览器直接拒绝、**没有
    任何绕过办法**。CA（10 年）与叶证书（397 天，SAN 覆盖 `localhost` + 当前全部可直连 IPv4，
    换网络重签）存 `~/.purevox/ca/`；**必须两级**——Android 的「安装证书 → CA 证书」只接受
    `basicConstraints: CA:TRUE`，一张自签叶证书装不进去。不想装证书就点一次「继续」
    （Chrome 绕过证书错误后仍算安全上下文）。
  - **mDNS 广播 `_purevox._tcp.local.`**（`net/mdns.rs`，随服务启停；TXT 只放 `v` 与 `proto`）：
    客户端据此自动发现电脑，**服务没开就扫不到是预期行为**。**改网卡（子网）不重启服务、
    改端口必须重启**：服务绑的是 `0.0.0.0`，换子网服务层面什么都没变、重启只会把已连着的
    客户端全踢掉；`net::stop()` 要真的 `abort` 监听任务，否则改端口后重启必然 `address in use`。
  - **编解码在客户端用 wasm**（`net/wasm/libopus-wasm.js`，MIT 预编译产物随包分发，**不新增
    C 编译链**）：不用浏览器内置 WebCodecs——`AudioEncoder` 只有 Chrome 94+ 才有，实测就有
    浏览器根本没有，于是在那儿麦克风与扬声器一起不可用。
  - **Opus 不引入 C 编译链（电脑端）**：`net/opus_sys.rs` 运行时动态加载——Windows 用
    `LoadLibraryExW` + `GetProcAddress` 加载预编译 x64 `opus.dll`（`src-tauri/opus.dll`，
    随包分发；`tauri.conf.json` 的 resources 已登记；查找 `<exe 同目录>/opus.dll` →
    仓库 `src-tauri/opus.dll`），Linux 用 `dlopen`/`dlsym` 加载系统 `libopus.so.0`
    （Debian 包名 `libopus0`；缺失时网络音频降级为明确原因，其余功能正常）。
    注意 `opus_encoder_get_size` / `opus_decoder_get_size` 是**兼容桩恒返回 0**，
    一律用 `*_create`（libopus 内部分配）。**帧长不固定**：PC 发 10 ms（= 1 hop），
    接收端按样本累积后重切 hop，10 ms 网格不受影响。
  - **Android 端 Opus = 内置 libopus，不用系统 MediaCodec**（§1.2 续）：`libpvopus.so`
    （`jniLibs/arm64-v8a/`，NDK 预编译入库，含 libopus 1.5.2 静态库 + `cpp/pv_opus.cpp`
    的 JNI 转发层；**Gradle 构建不需要 NDK**，CI 也不用装），重建走
    `tools/automation/build_opus_android.py`。**原因**：Android 上**从来没有
    `audio/opus` 这个 mime**——系统 Opus codec 注册的是 `audio/raw` /
    `audio/ogg;codecs=opus` / `audio/webm;codecs=opus`，后两者只认容器封装，喂裸 Opus
    包会**照单全收并输出垃圾 PCM（听感满屏刺啦且不报错）**；想吃裸包就得自己实现
    Ogg/WebM 封装，那是拿「省一个依赖」换「自己维护一个协议」。libopus 是参考实现，
    且与网页版的 `libopus-wasm` 同源，两端天然对齐。`audio/OpusSupport.kt` 因此**只剩
    诊断**（把本机系统 codec 列出来，解释为什么不采用），**不是第二条实现路径**。
    JNI 边界上做 float↔int16 转换（libopus 只吃 `opus_int16`；`OPUS_SET_INPUT_SIGNATURE`
    那套只在自定义构建里有，标准构建走 `OPUS_SET_LSB_DEPTH(16)`），Java 侧仍是
    `FloatArray` 全链路，转换只发生一次。
  - **每个方向只有一处缓冲，且在接收方**（NETWORK.md §4.1.4）：入站环固定 **20 ms**，
    出站**没有队列**——引擎每出一个 hop，连接任务立刻编码发走（服务端不再有定时器）。
    接收方一个固定深度的环同时吸收网络抖动与两端晶振差；运行期只有两种动作：
    **超了丢一格（10 ms）、空了补一格（PLC / 淡出）**。
    **不做任何运行期自适应**（不按抖动调深度、不做 PI 伺服、不重采样）：实测晶振差
    −24/−29 ppm、局域网抖动 0.4~1 ms、10 ms 绝对节拍误差 0.0%（`audio/clock_probe.rs`），
    25 ppm 的漂移约 13 分钟才需要修正一格；前两版按抖动自适应 + 变速重采样，实测反而
    摆出 ±1500 ppm 颤音、起播被误判成欠载。**网络差就是效果差，不为坏网络做补偿。**
  - **发送节奏**：出站用「推来即发」（`OutboundSub::recv()`），**没有 10 ms 定时器**；
    上行是手机每读满 480 样本就发一包。定时器 + 队列正是旧版 20~50 ms 延迟的来源。
    两端的 `TCP_NODELAY` 必须开（10 ms 一包、几十字节，Nagle 会攒成串）。
  - **端口 / Windows 定时器精度**：出站不再需要 1 ms 定时器精度，`pacer_*` 已删除；
    别再因为「音频要准」把 `timeBeginPeriod` 加回服务端（那只在测试脚本里需要）。
  - **测量是设计的一部分**（DEBUG.md §1.4）：音频帧头 8 字节（`seq` + 发送方 µs），
    `sync`/`syncreply`/`offset` 做 NTP 四时戳估时钟偏移，客户端 2 Hz 上报 `st`
    （环水位/欠载/掩盖/晚到/单向延迟/**自己实测的音频时钟 ppm**）。结论进 `/debug` 的
    `net` 段与手机「诊断 → 音频」，判定只在 Rust 一侧算（`hub.rs` 的 `rx_verdict`/`tx_verdict`）。
  - **按键注入**（`net/keys.rs`）：`SendInput` + **Set 1 扫描码**（不是虚拟键码，能走真实
    键盘布局并区分左右修饰键）；文本用 `KEYEVENTF_UNICODE` 逐 UTF-16 码元注入（中文、emoji
    都能打，且不经过电脑端输入法）。映射表在 `net/keymap.rs`（105 键，Android 键码常量
    **从 AOSP `KeyEvent.java` 逐个提取，勿手抄**）。注入后端按平台分：Windows 已实现，
    其余平台返回明确的不可用原因（Linux 需 uinput/XTEST），不静默失败。
  - **安全取舍（明确记录）**：局域网同网段可直连、**无鉴权**，因此远程输入默认关闭、
    界面常驻显示状态、有一键关闭；不要在没有开关保护的前提下默认开启按键注入。
- **Android 客户端（`android/`，APK）**：与上面三件能力对同一个 WebSocket 的另一端，
  单 Activity + Material3 **卡片式**布局（连接 / 功能 / 远程输入 / 后台保活 / 更新），
  底部「诊断」分**网络**（RTT 窗口/抖动/真实速率/质量结论）与**音频**（上/下行电平条、
  缓冲水位、欠载/丢包/解码掩盖）两块 + 日志，**默认收起**、标题行点击展开（状态面板不消失，
  只是不再一上来占满屏），「复制诊断」一键把整块塞剪贴板（报问题不必截图）；
  实时量没有样本时写「采样中…」、没连上写「未连接」、没订阅写「未开启」，**不伪造 0**。
  - **地址两格 + 扫码**：连接卡片里 IP/主机名与**端口分成两格**（默认 59123、可改；
    粘整条 `https://ip:端口/` 会自动拆回两格），端口右边一个「扫描」按钮打开相机扫
    电脑端网络卡片的二维码（`PvScanActivity`：竖屏 + 正方形四角取景框），扫到就**自动连接**；
    发现到电脑也**不锁**这几个控件（锁过 → 扫描按钮点了没反应）。旧连接的
    `onClosed/onFailure/onMessage` 必须按 `client` 身份丢弃（迟到回调会把新会话关掉；
    重连/扫码两次断开的根因）。
  - **本机调试包与 CI 的 release 包共存**（2026-10-27）：debug 构建带
    `applicationIdSuffix = ".debug"` + `versionNameSuffix = "-debug"`，源集
    `app/src/debug/res/values/strings.xml` 把应用名覆盖成 **PureVox Debug**，APK 产物统一命名为
    `PureVox-<版本>[-debug]-android-arm64.apk`（`versionNameSuffix` 带出 `-debug`，别在文件名里
    再拼一次）。**原因**：Android 只允许同签名的包覆盖安装，而 release 用固定密钥、debug 用 AGP
    生成的 debug keystore，包名相同时**必须卸载线上包才能调试**（装回来还要重下一次）。换包名后
    两者是两个独立应用（独立偏好与数据），`packageName` 全是运行期取的、FileProvider 的
    authority 写的是 `${applicationId}.fileprovider`，所以跟着自动变。**别改回同名**，
    也别忘了 `release.yml` 的 Stage APK 步骤按新文件名 glob（写死 `app-release.apk` 只在打 tag
    时才炸）。调试包点「检查更新」仍拉 release 的包，装上去是**第二只应用**，这是刻意的真实链路。
  - **麦克风 API 下拉**（`audio/MicSources.kt`）：按**常用程度**从上到下列出
    `MIC` / `VOICE_RECOGNITION` / `VOICE_COMMUNICATION` / `CAMCORDER` / `UNPROCESSED` /
    `VOICE_PERFORMANCE` / `DEFAULT`，末尾是系统专用 / 隐藏 API（`VOICE_CALL` /
    `VOICE_UPLINK` / `VOICE_DOWNLINK` / `REMOTE_SUBMIX` / `HOTWORD` / `ECHO_REFERENCE` /
    `FM_TUNER` / `VOICE_COMMUNICATION_SIGNALLING`）。选择存 `micSource`（SharedPreferences），
    推流中改选择会重启采集；所选 API 建流失败时**自动回退 MIC 并显示原因**，不静默。
  - **缓冲 / 时钟与网页版同一原则**：`SpeakerDownlink` 是「包队列 + 解码线程 + 样本环形缓冲 +
    写入线程 + AudioTrack 主时钟」；环预缓冲 **50 ms**、硬顶 341 ms，AudioTrack 底层 **40 ms**
    （两者别叠成 200 ms），输出走 **stereo + `PERFORMANCE_MODE_LOW_LATENCY`（SDK>=26）**
    的低延迟快通路（mono/普通缓冲作为回退），音频线程 `THREAD_PRIORITY_URGENT_AUDIO`；
    欠载补静音、硬顶丢最旧。网页版是先行实验田，安卓照搬同一策略，别各自发明一套。
  - **会话保活**（`service/CaptureService.kt`）：只要「已连接」「正在推流」「连接中」**或界面在前台**
    就把常驻通知挂着（前台服务类型 `microphone|dataSync`，只报在用的那个），通知带「停止推流 / 断开连接」
    两个按钮 —— 按钮发**广播**回界面（`MainActivity` 的 `notifReceiver`），服务不认识协议。
    **为什么宽到「界面在前台就挂」**：2026-10-18 用户实测 ——「撤掉通知之后系统很快就把进程掐了」；
    而**后台里起前台服务会被系统拒绝**（`ForegroundServiceStartNotAllowedException`），
    那一次失败没有第二次机会，等于整个会话都在裸奔。所以：进入前台（`onStart`/`onResume`）就补挂，
    **挂不上不许静默** —— 原因写进「后台保活」卡片与「复制诊断」（`CaptureService.lastError`），
    界面上一眼看出「通知没挂上」还是「会话正常结束」。
    另有「忽略电池优化」入口（`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`）。
    **通知权限（Android 13+ `POST_NOTIFICATIONS`）是保活的前提，必须可见可修**：声明 + 首启申请
    （`requestPermissions`）、拒过一次再求一次（`maybeRequestNotifPermission`，靠
    `shouldShowRequestPermissionRationale` 判断系统还肯不肯弹）；**再拒就永久静默** ——
    此时唯一出路是设置页，所以「后台保活」卡片常驻一行「通知权限：已允许 / 没允许 / 渠道被关」
    + 「通知设置」按钮（`ACTION_APP_NOTIFICATION_SETTINGS`，渠道单独被关时进
    `ACTION_CHANNEL_NOTIFICATION_SETTINGS`）。用户 2026-10-18 反馈「从没见过通知权限弹窗」
    就是这种状态：没有权限 = 没有通知 = 系统随手掐进程，而界面上什么都看不出来。
  - **应用内更新**（`update/{UpdateChecker,UpdateDownloader,UpdateInstaller}.kt`）：
    **手动**检查（不自动），数据源与本仓库 GitHub Releases 一致；比较依据是 tag 的完整时间戳
    映射出的 `yyMMddHHmm ÷ 10` 与安装包 `versionCode` 比整数（开发构建 `versionCode=1` → 不下结论）；
    **÷10 不能省**：十位数 `2610172359` 超过 versionCode 的 Int32 上限 2147483647，
    溢出会被 Gradle 侧静默丢成 `1` —— 表现是 APK 永远报「开发构建」、应用侧也认不出 release
    （2026-10-17 实测踩到，`version.py` / `build.gradle.kts` / `UpdateChecker.stampToCode`
    三处必须同算法，`check.py -Gate version` 与 `build` 各有断言）；
    产物按 `.apk` 后缀匹配；下载落应用私有外部目录，经 `FileProvider`（`res/xml/file_paths.xml`）
    交系统安装器，需 `REQUEST_INSTALL_PACKAGES`。
  - **长文本 / 退格**：输入框是 `PvEditText`（包一层 `InputConnectionWrapper`）——「打字同步」
    照旧逐次转发并清空；**空框里按删除键**则把 Android `KEYCODE_DEL` 转发给电脑（映射成 PC
    Backspace），框里有字时本地删。另有「发送 / 清空」按钮把攒好的一整段一次性发出（同一个
    `text` 消息，只是时机不同）。
  - 图标见上（自适应 + monochrome），文案在 `res/values/strings.xml`（Android 侧独立，
    **不是**桌面 `ui/i18n.js` 那份字典）。
- **媒体行（音效板 / 音乐播放器）**：`soundpad` / `player` 是**处理行**（ARCHITECTURE.md §4），
  在自身链位置把媒体样本与信号相加（挂链尾 = 后级直通；拖到降噪之前则参与处理）。
  **这两类行的全部控件由 `ui/media.js` 独占**（音效网格 / 走带 / 每个音效的音量与热键）；
  `ui/columns.js` 对它们**不画通用参数控件**，否则同一件事有两个入口。
  - **入库唯一实现** `media_import`：任意来源文件经 symphonia 解码 → 下混单声道 →
    重采样 48 kHz → 写 16-bit WAV 落 `~/.purevox/soundpad/`（音效）或
    `~/.purevox/music/`（曲目），文件名 `<原名>-<内容哈希>.wav`（同内容幂等复用），
    配置只引用库内路径。移除音效/曲目时 `media_discard` 双重护栏：只删库内文件、
    仍被计划引用则不删。界面用 `<input type=file>` 取字节 → **base64** 过 IPC
    （逐字节 JSON 数组会让 10 MB 曲子变成几千万个数字）。
  - **symphonia 的 feature 分「容器」与「解码器」两类，缺一不可**：`pcm` 只开解码器、
    **没有 WAV 容器读取器**（wav 打不开）；vorbis 需要 `ogg`、m4a/AAC-in-MP4 需要
    `isomp4`。漏一个就是「某格式怎么都打不开」，见 `Cargo.toml` 注释。
  - **离线重采样必须冲尾**（`media::resample_to_48k`）：sinc 重采样器首批输入只填内部
    缓冲、不出样本。实时流永远在跑无所谓，**文件是一次性的，补不补就是每首短一截**
    （实测 44.1k→48k 短 480 样本 = 整整 1 hop）。末尾喂静音凑够精确目标长度再截断。
    **别改回 `ToHops`**：那是实时链路（保持 10 ms 网格与流式节奏）用的。
  - **音效板**：音效整段入内存（短音效起播零延迟），起播/结束/重播/停止一律 hop 内线性
    淡变（**首样本恒为 0**，杜绝咔哒）；重播同一音效 = 旧声部淡出、新声部从头起播；
    音量每个音效一个（±10 dB，实时）。**每个音效可绑一个全局热键**（点框录键，
    Delete 清除），热键**跟着计划走**（存 `pads` 参数的 `hotkey` 字段），由
    `plan::pad_hotkeys` 统一收口、`apply_plan` 时重注册——**不另设一份设置**，否则增删
    音效会出现「界面看着有键、按下去没反应」。
  - **播放器**：流式（后台解码线程 → 3 s 有界环 → 音频线程每 hop 取 480 样本），曲目环流
    （EOF 回卷）；播放/暂停/seek/音量；`resume_sec` 持久化。
  - **控制面**：播放/停止/seek/音量是**运行时命令**，经全局 `MediaHub`（`engine/media.rs`，
    与 `NetHub` 同款单例）按 (列, 行) 寻址下发；音量属**热参数**——拖动实时生效、抬手
    `persist_plan` 落盘，**不重建会话**；增删音效 / 改热键 / 换曲 = 结构性变更 → 重建。
    行状态（在播音效数 / 播放位置）经 `Stage::status` 进列概要，`/debug` 可见。
- **OBS 音频滤镜**（2026-10-25，参考 `~/obs-studio/plugins/obs-filters`）：处理行新增
  `noise_gate` / `compressor` / `limiter` / `expander` / `upward_compressor` / `eq3` / `invert`，
  参数默认值、范围、步进与 OBS 一致（滑块 + 数字输入）；DSP 在 `components/obs_dsp.rs`。
  不包含 Noise Suppression（AI 降噪替代）、VST 2.x（需要 VST 宿主）、Audio Monitor /
  Pan / Sync Offset（OBS 的源属性/路由；PureVox 输出行承担监听）。
- **31 段均衡（`eq31`）与自动增益（`agc`）**：都是处理行，DSP 用**开源库**——
  `biquad`（MIT/Apache，RBJ Cookbook + Direct Form 1）与 `dagc`（MIT/Apache）。
  31 段按 ISO 1/3 倍频程（20 Hz~20 kHz，±24 dB）；低切/高切是独立开关（12 dB/oct），
  频响也画进曲线。整条曲线是**一个 JSON 热参数 `settings`**（`HotEq` 无锁原子单元格，
  `DirectForm1::update_coefficients` 不重置状态）：拖动/预设**不重建会话**。
  显示曲线用 `eq_curve` 命令（Rust 算，Catmull-Rom 过频段中心 + 切点斜率；**不是**
  biquad 相加的精确响应，避免相邻频段的梳状起伏）。界面 `ui/eq.js`：可拖频段 +
  红/绿柱子（正/负增益）+ 6 内置 + 6 自定义槽（长按保存）。AGC 三个热参数是
  `target_db` / `speed` / `floor_db`；**dagc 的 `desired_output_rms` 实际是均方值**
  （传 10^(dB/10)，别传幅度，否则目标响 20 dB）。
- **桌面声音输入（`loopback` 输入行）**：`device` = `loopback`（系统默认输出）或
  `loopback:<渲染端点ID｜sink 名>`（按平台，同 AEC 远端取值规则），复用 AEC 的回环采集
  （`audio/loopback.rs`）；行内增益 `volume_db`（-30~6 dB，默认 -6 dB，系统混音偏响）。
- **手机 / 客户端的入口 = 网络行卡片（`ui/netrow.js`，NETWORK.md §4.1.11）**：顶栏**没有**独立的
  「手机」面板——网络麦克风 / 网络扬声器那两类行就是全部的手机操作入口（没建网络行就先加行）。
  卡片分三层职责，**不要混**：① **服务本身**（状态灯 + **一句短状态**、`启动` / `停止` /
  `重启服务`；全文看 `/debug` 的 `net` 段，`net_card` 只给结构化短字段）；② **面向客户端的
  地址**（网卡下拉 + 可编辑端口，二维码 / 复制 / 网址都跟着选中的地址走；`net_ifaces` /
  `net_select_ip` / `net_set_port`）；③ **会话级开关**（远程输入**默认关**、风险提示在
  tooltip、AEC 只在 `remote_mic` 行存在）。**极致紧凑**（用户 2026-10-27）：不可改的说明
  文字一律不上卡片（「近端永远是网络输入」这种写一次设计不够，别每张卡占一行），**二维码
  右边只放能点的东西**（证书 / 远程输入 / AEC / 缓冲），不铺说明栏。
  **AEC 与独立 AEC 行是同一个 `AecRow`、同一个内置模型**（`infer::aec::MODEL_AEC`）：计划里
  没有 `model`、界面不给选；`校准` / `结果` 都可用，命令 `calibrate_aec_delay(col, row)`
  测卡片自己那一行，远端为 `network` 时探针注入网络出站。另有「下载证书（手机装一次）」
  （`net_ca_pem`）。调试面板只留 `net` 段的只读状态行。
  本机地址取局域网 IP（`net_local_addr`，UDP connect 技巧，不发包）；远程输入注入见上文
  `net/keys.rs`（Windows 已实现，其余平台明确不可用）。
- **更新（顶栏「更新」，`src/update/`）**：**唯一数据源 = 本仓库 GitHub Releases**
  （`repos/a2heng/PureVox/releases/latest`）。三步状态机 `检查 → 下载 → 安装`，
  界面按状态机卡按钮，不许跳步。
  - **比较依据 = tag 的完整时间戳，不是显示版本号**：显示版本 `yy.M.d` 同日会重复，
    靠 `version::STAMP`（`yyyy.MM.dd.HHmm`）分新旧（`version::parse_stamp`）。
    开发构建没有 CI 注入的时间戳，此时**不下「已是最新」的结论**，报「开发构建」。
  - **产物按文件名后缀匹配，不看版本号**（同上）：Windows `-setup.exe`（NSIS；
    **Windows 只发 exe，不出 MSI**——`src-tauri/tauri.windows.conf.json` 把 Windows 的
    `bundle.targets` 钉成 `["nsis"]`）；Linux **按本机安装方式各认一条**（`.AppImage` /
    `.deb` / `.rpm`，不是优先级列表）；安卓 `.apk`。
    `release.yml` 统一重命名产物，后缀因此稳定。
  - **安装分平台 · 先认「本机是哪种安装」再挑包**（`update/install_{windows,linux,other}.rs`）：
    - **认安装方式用 Tauri 3 自己的标记，不靠 dpkg/rpm 查询**：bundler 打包时把
      `__TAURI_BUNDLE_TYPE_VAR_{APP,DEB,RPM}` 打进二进制（实测本仓库产物：deb 里是
      `VAR_DEB`、AppImage 里是 `VAR_APP`），`tauri::utils::platform::bundle_type()` 直接读它
      → `Status.install_kind`。不用去问包数据库、不用猜装到 `/usr/bin` 还是 `~/.local`。
    - **产物按安装方式挑，一种方式只认自己那种包**（不跨格式兜底）：deb 用户下 `.deb`、
      rpm 用户下 `.rpm`、AppImage 用户下 `.AppImage`。以前 Linux 是「AppImage 优先」，
      结果 deb 用户白下 140 MB 的 AppImage 而且装不上（`install()` 只会报不支持）。
      认不出安装方式（没打标的二进制，如直接跑 `target/debug/purevox`）→ 不下载，
      `unsupported` + 原因；release 里缺你那种包 → `failed` + 原因（都带 release 页面出口）。
    - **Windows** = NSIS 静默 `/S /UPDATE /NS /R`（**必须先退进程再装**：安装器开头有
      `CheckIfAppIsRunning` 会拦下正在运行的 exe，故用**没有窗口的 cmd**（`CREATE_NO_WINDOW`；
      **不能加 `DETACHED_PROCESS`**，否则 `ping` 会弹出一个可见终端窗口）先歇几秒再拉安装器，
      随后 `app.exit(0)`；命令行必须 `raw_arg` 原样传，`.args` 会把内部引号转义坏，
      详见 TAURI3.md §0 踩坑 6）。
    - **Linux = 三种包都能更新，分两条路**：AppImage 自身是可替换的文件 → 写同目录临时名
      再 `rename`（正在跑的进程用的是挂载副本，覆盖源文件不影响它；实测覆盖后挂载仍可读）；
      **deb / rpm 属于包管理器**（`/usr/bin/purevox` 是 root 所有，`cp` 覆盖运行中的二进制
      会 `ETXTBSY`），故走 `pkexec dpkg -i <包>` / `pkexec rpm -U <包>`（polkit 图形授权框，
      与上游 updater 2.10+ 同一条路），**不用 zenity/sudo 那套降级**；`pkexec` 缺失或用户
      取消 → 明确报错（带包路径），不静默。deb/rpm **装完不退出**（阶段 `installed`），
      重启 PureVox 生效 —— 不打断正在用的会话。
    - 其它平台明确不可用。**不静默失败**。`/debug` 的 `install_kind` 一眼看出走的是哪条路。
  - HTTP 客户端用 `reqwest`（已随 `tauri` 进入依赖树，TLS 后端与 `ort` 的 `ureq` 同一套
    native-tls，不新增 C 编译链）。状态进快照 `update` 段（与界面同源）。
- **推理链路**（`src-tauri/src/infer/`）：用 `ort`（onnxruntime）流式推理。**模型契约是谱接口**
  （2026-10-29 起）：`(1,2,481)` 复数谱（= 960/480 周期 Hann 分析帧）+ `cache_in [1,D]`
  （扁平流式缓存，首帧零起）→ 增强谱 + `cache_out [1,D]`。**傅里叶在图外**，由 `infer/spec.rs`
  统一做分析/合成与 OLA（逐样本除 Σw²），输出**滞后 1 hop**；缓存维度、TSE 的 keys/gb 形状
  都从模型输入读，不写死。
  降噪是**列里的处理行**（`engine/components/denoise.rs`，`Stage`），在**列工作线程**内惰性加载
  （每行一份，只加载一次）；模型文件选择存计划，改型号 = 结构性变更 → 重建会话。
  现役模型常量 `infer::MODEL_DENOISE`（降噪）、`infer::MODEL_AEC`（AEC，202610 单流：
  far/mic 各 5ch 第 0 层 early fusion + iLN，**无显式延迟线**（对齐靠 GRU 隐状态）、cache 91,920；
  2026-10-18 换下 202609cpx，旧文件已从 `models/` 移除）。
- **AEC 行 / TSE 行**：AEC 是**输入行**（`ptype = echo_cancel`，本行 `device` = 近端 mic，
  `params.far_device` = 远端参考：`loopback` = 系统默认输出回环、`loopback:<渲染端点ID>` = 指定输出
  回环（WASAPI loopback，`audio/loopback.rs`，仅 Windows）、其它 = 输入设备；`far_delay_ms` **有符号**
  （正 = 远端超前、向后取历史；负 = 远端缓冲超前、向前取）。far 历史 2 s 采样网格（`engine/aec.rs`）
  **按实时推进**（回环设备空闲不出数据时补零，否则序号会随会话时长越落越后、取窗口永远失败），
  窗口 = mic 采样序号 − 延时，历史不足先退最近段、再没有就直通 mic；far 与 mic 序号同原点
  （会话起步 / 校准开始归零）。TSE 是**处理行**
  （`ptype = tse`，参考固定为 `~/.purevox/tse_reference.wav`，须 48 kHz 单声道；界面不给路径设置，
  只显示录制时间 + 打开文件夹；10 s → `ref_encoder.onnx` → `keys`+`gb`（每句一次，与主图成对）；
  无参考直通）。
  行状态（对齐计数 / 推理耗时 / 参考状态）经 `Stage::status` 进列概要。
- **参考录制 / 延时校准**：命令 `record_tse_reference(seconds)` 录「降噪后、TSE 前」的信号（`recorder.rs`，
  **原样保存**，不做音量归一化 / 限幅；完成提示报实测 RMS 与峰值）→ `~/.purevox/tse_reference.wav`；命令 `calibrate_aec_delay()`
  对第一个 AEC 行采集 1.6 s、向被回环的输出送 500→7000 Hz 单次扫频探针（150 ms、平顶包络、0.98
  幅度；**峰宽由带宽决定，不是时长**），FFT 互相关**对称搜索**延时（**从零
  重置、固定参考延时、只喂精确窗口 → 测的是绝对延时**，不累加、每次可重复），并
  **自动配平**（近端/远端各自 RMS 归一到 -24 dBFS → 回填 `mic_gain_db` / `far_gain_db`）。去直流 + 带限
  （与探针带一致）+ 带内 RMS 归一化（`engine/calib.rs`）。最近一次缓冲落在 `~/.purevox/calib_last_{mic,far}.f32`
  便于离线排查。进度/结果都在快照里（`recorder` / `calib`），界面据此回填参数并重建会话。
  AEC 行另有 `bypass`（直通，跳过 AEC）供 A/B 对比。
  **校准结果分两件事报**（意义不同，别混）：① **回声路径**（far→mic 衰减，探针段带内 RMS 之差）
  ——扬声器实际播放的 vs 麦克风实际收到的；② **麦克风电平配平**（近端/远端各归一到 -24 dBFS）。
  例：`延时 150.0 ms（相关 0.30）｜回声路径（far→mic）−42 dB｜近端 +27 dB｜远端 −15 dB`。
  校准有三条 Linux 相关约束：① **探针必须送到 far 监听的同一只输出**（Linux 走
  `audio/probe_linux.rs` 的 `pacat`，cpal 打不开 Pulse sink 名；探针发声的设备 = AEC 要消除的设备），
  ② **会话必须先「启动」**（未运行直接报错，不静默卡在「采集中」），且采集有**独立看门狗**
  （列线程不喂数据也会超时结束），③ **探针峰值已接近满刻度（输出侧实测 -1.2 dBFS）**，
  麦克风收到的响度只由「系统音量 × 声学路 × 麦克风增益」决定——**不允许代改用户音量**。
- **onnxruntime 会话必须单线程且关闭自旋**（`with_intra_threads(1)` / `with_inter_threads(1)` /
  `with_intra_op_spinning(false)`）：默认按核数建池并忙等，会占满 CPU、和音频回调抢核，
  实测把 1.8 ms 的推理拖到 >100 ms。改这三项前先读 `infer/denoise.rs` 的注释。
- **不得影响音频**：音频线程只做无锁计数/写环形缓冲，汇总与序列化在采集线程完成；
  系统指标采样周期 1 s。
- **界面不要写死高度**：容器一律 `height: auto` + flex 纵向 + `flex-wrap: wrap`，
  **禁止 `grid-template-rows: 26px 26px` + `height: 66px` 那种「先说死要几行」的做法**
  （早期版本 AEC 4 行 / TSE 3 行 / 媒体行整块都写死了）。内容一多就被裁掉、控件点不到，
  表现为「行卡死」；音效板要塞整个音效列表、播放器要塞进度条 + 音量，固定高度根本不够用。
  需要「数值不跳动」的地方用 `tabular-nums` + 定宽数字列实现，不靠高度。
  规则写在 `ui/debug.css` 文件头，改样式前先读。
- **侧栏宽度只有一个来源**：`#shell` 的 grid 第一列（现 176px）。`#sidebar` **不许另写 width**
  ——两处不一致时侧栏内容会溢出列宽、被右侧工作区盖住（用户原话「启动/中英文被右边压着」，
  2026-10-26 实测）。窄屏塌缩档（`@media (max-width: 1020px)` 把侧栏压成 64px 图标条）**已删除**：
  桌面窗口没有「手机版」，按窗口宽度改变左栏会让菜单居中、文字换行、内容被裁；窗口变小由右侧
  列区自己换行/滚动承担。
- **新功能的完成标准**包含调试输出：新增模块（流、设备、模型、效果）必须同时把自身状态
  接入 1.2 节对应端点，否则视为未完成；测量 / 诊断类结果另按 1.4 提供结构化 JSON
  （落盘 + 端点）与可复现命令。
- **验证方式**：智能体与脚本验证运行状态一律请求 HTTP 接口（`curl http://127.0.0.1:<端口>/debug`），
  不靠模拟键鼠点界面（Windows 会拦截后台抢焦点，按键会打进别的窗口）。界面交互验证用
  Windows UI Automation（`TAURI3.md` 3.6 节）；**Linux 用 `tools/automation/ui.py`（AT-SPI）**——
  `list` 列控件、`click <可访问名>` 点按钮/勾选框（要 `DISPLAY=:0 XDG_RUNTIME_DIR=/run/user/1000
  DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus`），不要求人点。同一条链还有会话子命令：
  `start`（起应用并等 `/debug`；会话总线真缺 SNI watcher 时自动顶替身）/ `take '<jq 表达式>'`（读快照）/
  `verify-update <deb|rpm|appimage|none>`（点「更新 → 检查更新」，断言安装方式与**产物后缀对得上**，
  失败返回非 0）/ `stop`（收掉自己起的进程）。前置与跨总线限制见 `TAURI3.md` 3.6 节。
  **网络上下行 / 网页版播放不要靠耳朵听**：`python tools/automation/net_loopback.py`（`--mode speaker`
  测测试音→网络扬声器，`--mode roundtrip` 测网络麦克风→网络扬声器，含上下行 hub 与 10 ms 节拍；
  `--json` 出结构化结果、`--out` 落可听的 WAV）与 `python tools/automation/net_web_probe.py`
  （无头 Chrome 真开网页版，读 AudioWorklet 的欠载/水位，并验关/开后订阅 1→0→1）。
  两者都**隔离启动**（临时 HOME + 自动空闲端口），不碰用户的 `~/.purevox`、不需要手点界面。
  **少截图**，优先用调试接口读数。
  **不许动系统的音量 / 静音**（只读不写；用户音量不是我们该改的状态）。
- **测试自收尾**：任何启动进程的 shell 测试必须自己结束（PowerShell 用
  `try { … } finally { Get-Process purevox -ErrorAction SilentlyContinue | Stop-Process }`），
  命令一律带超时，后台任务不得残留；**不允许把运行中的 app 留给用户手动关闭**。
- **前端调试**：`ui/jsdebug.js` 把脚本错误、未处理的 Promise 拒绝、`console.error/warn` 转发到
  `ui_report`（即快照的 `ui` 数组）；顶栏「开发者工具」打开 WebView2 devtools（Cargo 已开
  `devtools` 特性，release 保留）。JS 静态检查用项目 `opencode.json` 的 LSP + `ui/jsconfig.json`
  （`checkJs`，配合 `ui/globals.d.ts`）。
- **外围**：系统托盘（启动/停止、显示/隐藏、退出；图标随运行状态在 `icons/tray_running.png` /
  `tray_stopped.png` 间切换，菜单随语言与状态重建）。**托盘注册失败不致命**：GNOME 上
  `org.kde.StatusNotifierWatcher` 由 appindicator 扩展提供，扩展没启用时总线上没有这个名字、
  ksni 直接报错 —— 此时只把「托盘不可用 + 原因」写进调试面板并继续跑（引擎与窗口跟托盘无关）；
  原来在这里 `?` 传播 = 启动即 panic，用户连界面都看不到（2026-10-18 实测）；
  **窗口最小化/关闭都收到托盘**（不退出）；**窗口位置/大小记忆**（`winstate.rs`）：Windows 用
  `GetWindowPlacement`/`SetWindowPlacement`（`rcNormalPosition` = 还原外框、`showCmd` = 是否最大化，
  所以最大化时不会把整屏写成用户的还原尺寸），其它平台用 Tauri 的 `outer_position`/`inner_size`；
  存 `settings.json` 的 `window`（**物理像素**），启动时夹进当前显示器（副屏拔掉也不会丢到屏外），
  运行中每 1.5 s 轮询、变了才落盘。主窗口在 `tauri.conf.json` 里是 `visible:false`，
  `setup` 恢复几何之后再 `show()`（否则会先按默认尺寸闪一下再跳位置）；
  开机自启（**官方插件** `tauri-plugin-autostart` 3.0.0-alpha：Windows 上是 `auto-launch`，
  先试 HKLM、非管理员回退 HKCU，**并写 `Explorer\StartupApproved\Run` 的「已启用」标记** ——
  只写 Run 键时任务管理器的「启动」页会当成被禁用，这就是旧版「开关没功能」的真因，
  2026-10-26 实测；不弹 UAC；`set_autostart` 失败会停在设置面板里显示原因）；**启动/停止**（命令 `set_running`/`get_running`，
  界面顶栏按钮（启动绿/停止红）、托盘、全局热键共用；快照带 `running`）；**全局热键**（`hotkey.rs`：
  Windows `RegisterHotKey` + 规范串「`Ctrl+Alt+Shift+Win` 顺序」，空串=不监听，注册结果进 `ui`；
  **Linux 走 XDG 门户 `org.freedesktop.portal.GlobalShortcuts`**（`hotkey_linux.rs`，Wayland 上唯一
  可能的路）—— 门户按 systemd 用户单元名认应用身份，**只有从应用菜单/桌面图标启动**才有 app id，
  终端里直接跑会被拒（`An app id is required`），此时界面上给的是这句可执行提示而不是「不支持」；
  首次绑定会弹一次系统对话框，最终绑定以门户返回的 `trigger_description` 为准并写进事件日志）；
  **提示音**（`cues.rs` 自合成 6 套预设，start 上行 / stop 下行；Windows `PlaySound` SND_MEMORY，
  Linux `paplay` 播同一份合成 WAV，临时文件播完即删）；
  **设置**（`~/.purevox/settings.json`：语言 / 热键 / 提示音；顶栏「设置」面板，键位可录制）；
  中英文（`ui/i18n.js`，中文即 msgid，顶栏「EN/中」）。应用图标 `src-tauri/icons/` 与 Android 的
  `res/mipmap-*` 由 `tools/automation/gen_icons.py` 生成（**不参与 CI**，一次性生成、产物入库；
  跨平台同一条命令、产物逐字节相同；P 的矢量定义与改法见 TAURI3.md §3.5.1）。
  **Android 图标 = 规范自适应图标**（与桌面版同一套 P + 渐变，不另画）：脚本一并写出
  `mipmap-anydpi-v26/{ic_launcher,ic_launcher_round}.xml`（含 `<monochrome>`，供 Android 13+
  主题图标着色），并生成五档密度的 `ic_launcher_foreground.png`（108dp 画布、P 落在 72dp 安全区）、
  `ic_launcher_monochrome.png`（单色剪影）与传统 `ic_launcher{,_round}.png`（API < 26 用）。打包用
  `cargo tauri build`。

### 1.4 AI / 多平台开发友好（硬约束）

调试信息不只给人看，还要给**脚本和 AI** 看：一次测量 / 诊断的结果必须是
「**可复现命令 + 机器可读 JSON**」，不能只有界面上的一句中文或一张图。

- **结构化结果是唯一数据源**：测量、校准、诊断类功能在 Rust 侧定义 `serde` 结构体，顶层带
  `schema_version`；**字段只增不改名**，单位写进字段名（`*_ms` / `*_db` / `*_hz`），电平注明是
  峰值还是 RMS、dBFS 还是 dB 差；没测到的值用 `null` + `reason`，**禁止用 0 冒充**（同 1.1 的 Probe）。
- **三处读同一份**：① 调试快照（`/debug` 与 UI 面板）；② 专用端点（只读 JSON，规则同 1.2，
  如 `GET /debug/calib`）；③ 落盘 `~/.purevox/<主题>_last.json`（覆盖写，带 `created_ms` 与
  原始 dump 路径）。**UI 只渲染，不在前端另算一套**（1.3 单一数据源）。新端点实现后同步登记到
  1.2 的表里，字段同样只增不改名。
- **失败也是结果**：`ok:false` 时也返回完整结构，带稳定的 `verdict_code`、人话 `verdict_text`
  和**下一步可执行的建议**（检查哪只设备 / 哪一路静音 / 哪个参数），不要只抛一个字符串。
- **图只是展示层**：包络 / 频谱 / 波形以数组进 JSON（`series`，标 `hop_ms` 与降采样倍率），
  UI 用 canvas 画同一份数据。**截图只证明布局与渲染** ——不是测量依据，
  AI 也不许靠 OCR 截图取数（内置截屏已移除，见 1.1）。
- **AI 友好 = 一条命令能复现**：结论必须能这样取到，例如
  `curl -s http://127.0.0.1:47821/debug/calib | jq .verdict_code`；工具 `--json` 的输出与端点
  用**同一套字段名**（同一个 schema），不是第二套。
- 现状与目标：AEC 校准目前只发一句状态串（`calib: Probe<String>`，界面再正则解析回填），
  按本节改造为上面的结构；这是**已记下的欠账**，不是可以继续照抄的先例。

**tools/automation 的跨平台纪律**（多平台开发是本项目常态，不是附加项）：

- 平台显式可选、默认自动探测：`--platform auto|windows|linux|macos`；探测结果、OS 版本、
  命令行列进输出的 `platform` 段，AI 读结果时不必再问「这是哪台机器」。
- **禁止硬编码设备序号 / 用户名路径 / 设备名**（例：`play=17, rec=19` 只对某一对 Realtek 耳机成立）：
  提供 `--list` 列设备 / 资源，由人或 AI 选；路径从 `~`、环境变量或参数来。
- **必须能离线**：`--from-dump` 读 App 落盘的结果（JSON + 原始 f32/WAV），没有硬件也能重算、
  出图、复核结论；在线与离线**走同一套分析函数**，别写两份算法。
- 平台相关 API 隔离并在输出里说明：Windows WASAPI 回环、Linux Pulse/PipeWire monitor、
  macOS CoreAudio；不支持时**明确报错 + 非 0 退出码 + 替代路径**，不静默降级、不假装成功。
- 输出分两层：`--out PNG` 给人看，`--json PATH` 给机器读；失败也写 JSON（`ok:false` + 原因）。
- 与 App 内实现共用同一份 schema（字段名、单位、`schema_version` 一致）；脚本只做离线复现，
  不引入第二套判定标准。

---
