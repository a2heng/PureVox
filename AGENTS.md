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
  `recorder`（TSE 参考录制进度/结果）与 `calib`（AEC 延时校准进度/结果）。

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
- **会话入口**：计划（`plan.rs`）+ 配置（`config.rs`，存 `~/.purevox/session.json`）→ 命令
  `get_plan` / `apply_plan`（结构性变更 → 重建整个会话）。列 UI（`ui/columns.js`）编辑计划：
  每列两端固定为输入/输出、中间可加/删/移行；输入/输出行选设备，处理行选型号。
  测试音是输入行的一种（`ptype = tone`，全局共享线程）。
- **推理链路**（`src-tauri/src/infer/`）：用 `ort`（onnxruntime）流式推理。模型契约统一为
  `*_hop [1,480]`（10 ms 波形）+ `cache_in [1,D]`（扁平流式缓存，首帧零起）→ `enh_hop [1,480]`
  （**滞后 1 hop**）+ `cache_out [1,D]`；缓存维度从模型输入读，不写死；STFT 在模型图内。
  降噪是**列里的处理行**（`engine/components/denoise.rs`，`Stage`），在**列工作线程**内惰性加载
  （每行一份，只加载一次）；模型文件选择存计划，改型号 = 结构性变更 → 重建会话。
  现役模型常量 `infer::MODEL_DENOISE`。
- **AEC 行 / TSE 行**：AEC 是**输入行**（`ptype = echo_cancel`，本行 `device` = 近端 mic，
  `params.far_device` = 远端参考：`loopback` = 系统默认输出回环、`loopback:<渲染端点ID>` = 指定输出
  回环（WASAPI loopback，`audio/loopback.rs`，仅 Windows）、其它 = 输入设备；`far_delay_ms` **有符号**
  （正 = 远端超前、向后取历史；负 = 远端缓冲超前、向前取）。far 历史 2 s 采样网格（`engine/aec.rs`），
  窗口 = mic 采样序号 − 延时，历史不足先退最近段、再没有就直通 mic）。TSE 是**处理行**
  （`ptype = tse`，`params.reference` = 参考 WAV，默认 `~/.purevox/tse_reference.wav`，须 48 kHz 单声道；
  10 s → `enr_tok`；无参考直通）。行状态（对齐计数 / 推理耗时 / 参考状态）经 `Stage::status` 进列概要。
- **参考录制 / 延时校准**：命令 `record_tse_reference(seconds)` 录「降噪后、TSE 前」的信号（`recorder.rs`，
  RMS 归一化到 -20 dBFS，峰值不削顶）→ `~/.purevox/tse_reference.wav`；命令 `calibrate_aec_delay()`
  对第一个 AEC 行采集 1.6 s、向被回环的输出送 800→6000 Hz 扫频探针，FFT 互相关**对称搜索**延时，并
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

---

## 2. 维护准则

1. **一个功能只有一条实现路径**。有多种做法时只保留一种并写进文档，被替代的实现直接删除，不留备选。
2. **先扩展，再新建**：先看已有函数/模块能否扩展，确认不能才新建。
3. **legacy 快照只读冻结**：`legacy-v2026.08.20.1943/`（音频链条化重构前）与
   `legacy-v2026.09.30.1944/`（Tauri 迁移前最后的 Tk 版本，已剔除二进制）禁止修改、删除，
   不参与构建、CI、打包、测试及任何批量改动（格式化、换行符归一、重命名等一律绕过）。只读查阅。
4. **迁移期不写用户更新日志**：旧的 `about/changelog.md` 已随旧实现归档；迁移进度记在 `TAURI3.md`。
   新版具备用户可见功能后再恢复更新日志，届时在本节写明位置。
5. **未接入的遗留二进制**：`assets/`（图标、像素字体）、`server/opus.dll`、
   `html/wasm/libopus-encoder.wasm.min.wasm`、`android/gradle/wrapper/gradle-wrapper.jar`
   是旧实现留下的二进制，按原路径保留以备复用；新实现决定使用或确定弃用时再移动/删除并在此更新。
6. **发版 tag**：主线 `v<yyyy.MM.dd.HHmm>`。CI 失败、从未生成 release 的 tag 必须删除
   （`git tag -d <tag> && git push origin :refs/tags/<tag>`），否则会截断下一个 release 的提交记录。
   迁移期暂无 CI 与发版流程。

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
- **许可证头**：每个源码文件（`.rs` / `.toml` / `.html` / `.js` / `.ts` / `.css` 等）顶部必须带 GPL-3.0 版权头 +
  模型声明 + `SPDX-License-Identifier: GPL-3.0-or-later`，照抄 `src-tauri/src/main.rs` 顶部并按注释风格替换。
  JSON 无注释语法，豁免。
- **README 双语**：中文 `README.md` + 英文 `README_EN.md`，结构变化时两处同步。

---

## 5. 许可证

- 源码 **GPL-3.0**（`GPL-3.0-or-later`），见 `LICENSE`；第三方声明见 `LICENSE-THIRD-PARTY.txt`。
- 内置 AI 模型（`*.onnx`）归 a2heng 所有，仅随 PureVox 经授权使用，见 `MODEL-LICENSE.md`。
- 作者另有 MIT 模型仓库可自由使用：`lightweight-denoise-48k` / `lightweight-aec-48k`。
