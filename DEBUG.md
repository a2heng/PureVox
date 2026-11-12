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
  **隔离测试实例**可用环境变量 `PUREVOX_DEBUG_PORT` 覆盖（默认仍是 47821，`debug::http_port()` 是唯一入口），
  这样本机已有 PureVox 在跑时第二个实例也能被脚本读到。
- 每项指标是一个 Probe：`{"state":"ok","value":…}` / `{"state":"pending"}`（如 CPU、GPU 占用率首轮差分前）/
  `{"state":"unavailable","reason":"…"}`。未知路径返回 404 + 端点列表，非 GET 返回 405。
- 端点（每个返回体都带 `ts`（毫秒时间戳）与 `uptime_ms`）：

| 端点 | 内容 |
| --- | --- |
| `GET /debug` | 汇总：下列全部端点的合集 |
| `GET /debug/system` | **CPU**：系统总占用、本进程占用、逻辑核数；**内存**：系统总量/可用、本进程工作集与私有字节；**GPU**：每块 GPU 的名称、占用率、显存总量/已用（含本进程）；**系统时钟粒度** `timer_resolution_ms`（Windows 默认 15.625 ms —— 它决定 `sleep` 的实际精度，见 `NETWORK.md` §3.3.2） |
| `GET /debug/audio` | 每路音频流：**结论（`health`，Rust 侧统一判定的健康度 `{level, text}`，阈值集中在 `audio/health.rs`，面板只渲染不算，见 1.3）**、方向、设备、**信号源（仅输出）**、状态、降噪状态（字段保留但已废弃：降噪现为列处理行，见 `audio.engine`）、设备原生格式、实测**设备侧 / 引擎侧速率**（3 s 窗口）、重采样器与其延迟、**输出流的时钟伺服修正（ppm）**、回调块大小（最近/最小/最大）与**回调间隔（最近/最大 ms，音频回调里锁-free 实测：只看回调次数看不出「卡了 2 秒又继续」）**、帧与 hop 计数、不足一 hop 的剩余帧、峰值/RMS 电平、**输出流欠载补静音样本数与重同步次数**、溢出/丢弃样本、流错误计数、**上述事件的实测每秒速率（`underrun_per_s` / `resync_per_s` / `overrun_per_s` / `stream_error_per_s`，3 s 窗口；累计计数只涨不降，速率才看得出现在还在不在发生）**、48k 缓冲水位与**设备侧缓冲水位**、端到端延迟、**模型推理耗时（均值/最大）**；另有汇总结论 `audio.verdict`（全部流的整体判断）；最近 50 ms 波形与平均频谱（481 bin，NFFT 960，输入流为降噪后、输出流为重采样前） |
| `GET /debug/devices` | 全部枚举到的输入/输出设备：ID、名称、接口、原生采样率/声道、是否默认、是否被选中、**是否已打开与设备池引用数**；以及最近一次枚举的时间与耗时。**热插拔监测**（`devices::spawn_watch`，Windows 3 s / 其它 5 s 比对一次 `(ID, 能输入, 能输出)` 指纹，连续两次一致才算变化）发现变化会自动重刷，事件日志留一行「设备列表已变化（新增 N / 消失 M）」 |
| `GET /debug/latency` | **逐列延迟预算**：处理链（输入 hop → 输出 hop）的实测互相关延时、相关度、抖动，以及输入设备回调 / 采集环 / 行输入环 / 输出混音源环 / 播放 48k 环 / 播放设备环 / 输出设备回调各段（ms）与最大的一段（见 1.5）。与快照 `latency`、`~/.purevox/latency_last.json` 同一份 |
| `GET /debug/ui/move?id&x&y` | 把节点移到画布坐标（界面命令队列；用 API 驱动，不模拟鼠标） |
| `GET /debug/ui/connect?from&from_port&to&to_port` | 连一条线（跨列自动并组；成环会拒绝） |
| `GET /debug/ui/disconnect?from&from_port&to&to_port` | 断一条线（节点保留，立即重建图） |
| `GET /debug/ui/channels?id&channels` | 改节点通道数；只丢弃越界入线，合法连线保留 |
| `GET /debug/ui/enabled?id&enabled=0\|1` | 启用 / 静音节点 |
| `GET /debug/ui/select?id` / `GET /debug/ui/arrange` | 选中节点 / 按列整理 |
| `GET /debug/calib` | AEC 校准的**结构化报告**（与快照 `calib_report`、`~/.purevox/calib_last.json` 同一份，见 1.4）：`schema_version` / `ok` / `created_ms` / 判定码与结论 / 两路峰值时刻与延时 / 回声路径 / 探针带电平与 SNR / 回填增益 / 平滑后的带内能量包络 `series` / 原始 dump 路径；未跑过时为 `pending` |
| `GET /debug/ui` | **调试面板页面本体**（1.1 用系统浏览器打开它）：HTML + 同源静态资源（`/debug/ui/{debug.js,debug.css,i18n.js,snapshot.js,jsdebug.js}`）。页面只 `fetch('/debug')` 渲染，不经过 IPC；内嵌在二进制里，装完也能用 |

- `engine_info.threads[]` 带 `tid`（操作系统线程 ID）：与任务管理器/PerfMon 的线程 CPU 对得上号，
  排查「哪个线程在烧 CPU」不用猜。
- 字段只增不改名；新增字段同步更新本表。
- 快照另带 `ui`：界面自报的最近消息数组（新在前，最多 50 条，见 `ui_report` 命令）；
  `audio.engine`（列概要）含每列 `hop/s` 节拍速率（正常 100，见 ARCHITECTURE.md §3.3）；AEC 行另带
  **回声诊断**（滚动 1 s 窗）：`远端解释度 ρ`（远端窗口对近端的归一化互相关，判断参考/对齐对不对）
  与 `回声抑制 dB`（10log10(Σ近端²/Σ模型输出²)，判断模型有没有出力）——两者一起看才能分清
  「参考没对齐」还是「麦克风里根本没有回声」；
  `recorder`（TSE 参考录制进度/结果）与 `calib`（AEC 延时校准进度/结果；
  **新代码读 `calib_report`**：结构化、带 `schema_version` 与曲线，见 1.4），以及
  `latency`（**逐列延迟预算**：处理链互相关实测 + 各段缓冲归因，与 `columns[].latency` 同源，
  见 1.5）、`virtual_mic`（Linux 虚拟驱动状态：sink / 真源是否已创建；非 Linux 为 `unavailable` + 原因）。
- 另有 `net`（手机 ⇄ 电脑，NETWORK.md §4.1）：服务端口、已连接客户端数、订阅音频数、
  进出包与样本计数、**每方向的水位/深度/抖动（EMA + 最坏）/最大到达间隔**、
  **实测单向延迟**（NTP 偏移 + 帧头时间戳）、欠载 / PLC / 裁剪 / 空环到达 / 丢包计数、
  出站每路的待发 hop 与丢弃数、**客户端自报**（环水位、欠载、掩盖、时钟偏差 ppm）、
  时钟偏移估计、RTT、最近错误原因，以及
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
  与本节「只绑 127.0.0.1」的调试接口是两回事。

### 1.3 实现约束

- **单一数据源**：Rust 侧 `DebugHub`（`src-tauri/src/debug/mod.rs`）维护唯一快照，UI 面板
  （GPUI 界面直接读 `DebugHub`，50 ms 轮询）与 HTTP 接口序列化的是**同一个 `DebugSnapshot`**；
  调试面板（`src-tauri/ui/debug.js`）只渲染，禁止自己另算一套。
- **采集来源**（`debug/{system,gpu_*,mem_*}.rs`，按平台分文件）：
  - CPU / 内存总量与工作集：两平台都用 `sysinfo`，同一套口径。
  - **私有字节**：Windows `GetProcessMemoryInfo`（`PrivateUsage`，含已换出）；
    Linux `/proc/self/smaps_rollup` 的 `Private_Clean+Private_Dirty+Private_Hugetlb`
    （**只算常驻**，换出后会变小 —— 口径差异写在这里，不假装等价）；
  - **GPU**：Windows 用 DXGI（适配器 / 本进程显存）+ PDH 计数器 `GPU Engine` /
    `GPU Adapter Memory`（任务管理器同源）；Linux 用 `nvidia-smi`（整卡占用 / 显存 /
    计算上下文的进程显存，2 s 缓存）或 `amdgpu` 的 `/sys/class/drm/card*/device/{gpu_busy_percent,
    mem_info_vram_*}`。单进程占用率两家 Linux 驱动都不提供 → 明确 `unavailable`，不编数。
  设备**枚举**用 `cpal`（`devices.rs`）；**音频 I/O 不用 cpal**：输出直连 WASAPI
  （`audio/wasapi_out.rs`，`IAudioClient3` 128 帧低延迟共享），其它平台走 `audio/cpal_out.rs`，
  由 `audio/out.rs` 分平台选择。设备刷新单一入口
  `devices::spawn_refresh`（启动时 + 面板「刷新」按钮），后台线程执行。
- GPUI 界面在进程内绘制；「本进程」指标包含界面开销（不再有独立的 WebView2 子进程）。
- **采集链路**（`src-tauri/src/audio/`）：设备按原生默认格式打开 → 回调线程只做交织样本写入 +
  写 `rtrb` 无锁环 + 原子计数 → 每路一个工作线程（独占一条设备流）先按订阅者**逐通道拆分**
  （`dsp::split::split_active`：按帧 `step_by` 整段取，没人订阅的通道一次都不碰；10ch/2 路
  192k 实测 5 ms 块 6.5 µs → 0.8 µs），再用 `dsp::resampler` 重采样到（整数倍/同采样率走 12/32 抽头多相，
  升采样 44.1k→48k 走 64 抽头多相，非整数降采样仍用 rubato sinc）
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
  消化源/设备时钟差；源断流则回预热重同步；48k 环超 300 ms 封顶丢最旧。伺服目标水位 **10 ms**
  （2026-11-10 从 40 降下来：逐列延迟探针实测它是全链最大的一段）。
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
- **设备不在不是「关键失败」**（2026-11-03，ARCHITECTURE.md §6.3）：应用计划时设备打不开只写进
  `problems`（界面逐行显示），会话照常提交；坏行按 1→30 s 退避自动重开，同一 ID 回来即恢复。
  输入行**5 s 一个 hop 都没有**（拔出 / 休眠回来流停）就放掉租约交给退避重试重新打开。
  只有模型 / 参考这类**不会自己好**的失败才回滚（保留旧会话）。**别把这条改回去**：
  回滚会让「两列都指向暂时不在的设备」变成死局（issue #10：改一列就被打回原样）。
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
  `/libopus-wasm.js` 给网页版、`/` 是网页版客户端）。两条音频能力共用一条连接：
  `remote_mic` 输入行（手机麦克风→电脑，**不需要虚拟声卡驱动**，直接进引擎可降噪）、
  `remote_speaker` 输出行（电脑→手机扬声器，按位置 tap）。**只传音频**：
  手机端只有麦克风上行 / 扬声器下行，远程打字与按键注入已整体删除。
  命令 `net_start` / `net_stop` / `net_restart` / `net_status` / `net_card` / `net_running` /
  `net_set_port` / `net_ifaces` / `net_select_ip` / `net_local_addr` /
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
- **Android 客户端（`android/`，APK）**：与上面两件音频能力对同一个 WebSocket 的另一端，
  单 Activity + Material3 **卡片式**布局（连接 / 功能 / 后台保活 / 更新），
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
  - **只传音频**：远程打字 / 长文本 / 实体键注入与电脑端接收端已整体删除；
    协议里没有 `text` / `key`，手机端只有麦克风与扬声器。
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
- **手机 / 客户端的入口 = 混音台的输入源 / 输出目的地（`SESSION-V2.md` / `NETWORK.md`）**：没有独立的「手机」面板——
  网络麦克风 / 网络监听就是输入条的一行 / 一条总线。
  卡片分三层职责，**不要混**：① **服务本身**（状态灯 + **一句短状态**、`启动` / `停止` /
  `重启服务`；全文看 `/debug` 的 `net` 段，`net_card` 只给结构化短字段）；② **面向客户端的
  地址**（网卡下拉 + 可编辑端口，二维码 / 复制 / 网址都跟着选中的地址走；`net_ifaces` /
  `net_select_ip` / `net_set_port`）；③ **没有会话级 AEC 开关**（AEC 是独立节点，见
  `SESSION-V2.md` §3.3）。**极致紧凑**（用户 2026-10-27）：不可改的说明
  文字一律不上卡片（「近端永远是网络输入」这种写一次设计不够，别每张卡占一行），**二维码
  右边只放能点的东西**（证书 / AEC / 缓冲），不铺说明栏。
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


### 1.5 逐列延迟探测（唯一实现）

`engine/latency.rs`。回答的问题不是「缓冲里还有多少样本」，而是**一列到底把信号推迟了多少、
主要卡在哪一段**（ARCHITECTURE.md §8 的三条硬约束只管上界，不管归因）。

- **测量点**：列线程每 tick 把「第一路有数据的输入 hop」与「第一路输出行取到的 hop」**原样**
  拷进两条 `rtrb` 环（音频线程只 `push` 整 hop，不分配、不做 FFT、不等锁）；
  `latency-analyzer` 线程每 500 ms 取走并分析。列与列各自独立，多列同时测，互不影响。
- **算法**：两种波形 4:1 抽取到 **12 kHz（0.083 ms 分辨率）**，取 300 ms 输出窗与 920 ms
  输入历史，FFT **归一化互相关**找峰：搜索 0~560 ms，换算 `delay = (560 ms − k)`（只接受
  0~500 ms）。输出窗整体后移 60 ms，让 **0 ms 延时落在搜索窗中间**；边界 2 ms 不作结论。
  **波形对不上时退到 10 ms 能量包络**（降噪 / TSE 会改相位，实测波形相关只有 0.29；
  包络只要求响度起伏一致，同一套峰值/突出度判据）。报告里用 `measured_method`
  （`waveform` / `envelope`）与 `measured_resolution_ms` 标明这次是哪一种。
- **判据（集中在 `latency.rs` 顶部常量）**：输出窗 RMS ≥ −55 dBFS、峰值 ≥ 0.30、
  主峰比 3 ms 外次峰高 ≥ 0.10。**任何一条不满足就 `measured_ms = null` 并写明原因**，
  不编数字：静音、1 kHz 测试音（周期信号出现等高峰）、处理后波形差异过大（TSE/替换类）
  都测不出来。连续 5 次取中位数，波动 > 3 ms 时一并告警。
- **预算归因**：`stages[]` 逐段给 `ms`（`Option`）与来源（`measured` / `buffer` / `device` /
  `resampler`）。输入侧读**这一行对应的设备流**（`input:<设备>` / AEC 的 `input:c{列}r{行}m`
  / 回环的 `input:c{列}r{行}`）的回调周期、采集环水位、重采样延迟；输出侧同样是该行设备流的
  播放 48k 环、设备环、重采样与回调周期，再加这一行自己的混音源环。
  `total_ms` 是**已知段之和**；`complete=false` 时它只是下限，`detail` / `health` 会写明缺哪段。
  `dominant` 指向最大的一段 —— 这就是「延迟原因」。
- **不额外开设备**：设备侧只读已有流的实测值。**Windows 的设备侧缓冲以 WASAPI 共享模式的
  「每回调帧数 / 采样率」为实测代理**（隐藏的系统内部缓冲不在其中），字段与文案都按代理写，
  不假装是硬件的全部延迟。
- **三处读同一份**（1.4）：快照 `latency`（以及每列的 `columns[].latency`）、
  `GET /debug/latency`、`~/.purevox/latency_last.json`（每 10 s 覆盖写）。
  调试面板只渲染 `stages[]` / `dominant` / `verdict`，不在前端重算。
- **复现**：
  `curl -s http://127.0.0.1:47821/debug/latency | jq '.latency.value.columns[] | {column, measured_ms, measured_method, measured_resolution_ms, dominant, total_ms, complete}'`
  —— 需要**非周期**信号（说话 / 音乐 / 噪声）；静音与测试音下 `measured_ms` 为 `null` 是预期。
- **单测**：`cargo test --manifest-path src-tauri/Cargo.toml engine::latency`（合成宽带信号，
  0 / 10 / 37 / 120 / 480 ms 全部落在 ±1.2 ms；静音与周期信号必须报错）。
