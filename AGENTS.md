# PureVox — AI 麦克风降噪

Windows / Linux 桌面应用 + Android 客户端：实时 AI 音频降噪 / 目标说话人提取 / 回声消除，支持本地麦克风和远程网络推流。

**栈**: Python 3.12+ 标准库 Tkinter（桌面 UI）+ 纯 Python 组件化音频引擎（`pvengine` 包：numpy + scipy + onnxruntime，无任何自编译二进制）+ 自研 ctypes libpulse 绑定（Linux 音频桥，系统 libpulse）
**桌面入口**: `python run_tk.py`
**Android 入口**: `android/` — Kotlin + OkHttp + Opus JNI

> **文档索引（渐进式披露）**——本文件只保留跨模块的硬准则与不变式；任务配方按需加载，
> 不在此展开：
> - `DESIGN.md`：顶层设计与规范（分层架构/节点模型/数据流不变量/SessionPlan 契约/扩展指南）。
>   **实现与本文件/DESIGN.md 冲突时以 DESIGN.md 为准**。
> - `.opencode/skills/purevox-build/`：运行/构建/打包（bootstrap、各平台命令、产物布局）。
> - `.opencode/skills/purevox-ci-release/`：CI/发版（workflow、tag、release notes、缓存）。
> - `.opencode/skills/purevox-architecture/`：模块速查、Linux 音频架构、Android、网络协议。
> - `.opencode/skills/purevox-linux/`：Linux 宿主依赖、发行版打包声明、本地验证与排障。

---

## 维护准则（发版后管理，所有贡献者必读）

**功能最小化模型** —— 本项目的首要约束：

0. **更新日志写在哪里**：每次发版/改动（新增、删除、行为变化）都在
   `about/changelog.md` **顶部**追加一条「日期 — 标题」记录。
   仓库**没有独立的根级 CHANGELOG.md / 用户手册文件**——更新日志与两份使用手册
   是关于对话框的三个 markdown 页（`about/changelog.md` / `about/windows.md` /
   `about/linux.md`，uitk 直接读文件渲染），写入时保持中文、无 emoji。
   **只写用户可感知的技术变更**（功能/修复/行为/性能/兼容面）；纯开发过程事务——
   换行符与编码归一、git 属性/钩子、CI 与打包脚本调整、目录重组、代码挪动等
   不改变产品行为的内容——**禁止写入更新日志，也禁止写进 README**。

1. **一个功能只有一条规范实现路径**。禁止"功能 ABC 三种都能用"的设计——多套平行实现等于高维护成本。新增功能有多个可行做法时，只保留一种并写进文档，其余不进入代码。
2. **先扩展，再新建**。开新方法 / 新类 / 新文件之前，先搞清楚已有方法能否扩展：优先 改已有函数/类 → 加参数/加配置 → 复用既有抽象；确认确实无法扩展才允许新建，并在更新日志（`about/changelog.md`）说明为何不能扩展。
3. **被替代的实现不保留平行代码**。如 Linux 的 PortAudio/GStreamer/JACK、旧虚拟麦克风架构等已弃用方案，直接删除，不留"备选"。
   - **例外：配置键占位不删**。`config_manager.py` / `device_api.py` 里按接口后缀写全的
     设备键（如 `input_device_wasapi` / `input_device_alsa` / …，共 10 接口 × 4 键）属于
     **跨平台共享的占位配置**，即使当前平台实际只用其中一个本地接口（Linux 只原生 PipeWire、
     Windows 只 WASAPI+MME、macOS 只 Core Audio），其余键也**保留不删**——它们不影响运行、
     是强配置结构的一部分，独立于本条的"被替代实现删除"规则；若日后清理，视作待办（TODO）
     而非本次改动目标。
4. **改动前先读对应模块，尊重既有设计意图**；删除功能需在更新日志（`about/changelog.md`）记录。
5. **legacy 历史快照冻结**。`legacy-v2026.08.20.1943/`（提交 c5972ae 的快照，音频链条化重构前的最后一个版本）是**只读历史参考**：
   禁止修改、更新、删除其中任何文件；也不得让它参与构建、CI、打包、测试或任何全局性批量改动
   （格式化、换行符归一、重命名、依赖升级等一律绕过该目录）。需要对照旧实现时只读查阅，
   修复/新功能永远改主线代码，不回写快照。
6. **发版 tag 与 release 描述**。release 描述由 `tools/automation/release_notes.sh` 生成：
   它用 `git describe --match` 按 tag 家族前缀（主线 `v*` / Lite `lite-v*`）取「上一个 tag」，
   两类 tag 不再互相遮蔽。但 **CI 失败留下、从未生成 release 的 tag 必须删除**
   （`git tag -d <tag> && git push origin :refs/tags/<tag>`）——否则它仍是同家族下一个 tag 的
   「上一个 tag」，会把 release 的提交记录截断。详见 skill `purevox-ci-release`。

**本项目的单一实现路径（强制执行）**：

- 音频引擎为**纯 Python 组件化管线**（`pvengine/` 包）：Stage 接口（process/reset/release）
  是组件唯一契约，组件按 active_modes 声明生效模式，可随意增删替换；所有调用方直接用 pvengine。
- Linux 音频采集/输出走 **pipewire-pulse 兼容层**（`pvplatform/audio/pwpipe_client.py`，
  自研 ctypes 绑定直调系统 libpulse）。ALSA 备选接口已整体移除（2026-08-22 纯 py 迁移）。
- 虚拟麦克风（Linux）= 单一生产者 + 双出口，全部健康，详见 skill `purevox-architecture`：
  ① 单声道 null-sink `purevox_out`（唯一写入口）；② 内置 monitor
  `purevox_out.monitor`（宽口径源）+ 非 monitor 真源 `purevox_mic`
  （`module-remap-source` 把 monitor 重映射而来，供 OBS 等"只列真源"软件）。
  不用 pw-loopback。**禁建第二路源用 `module-null-sink media.class=Audio/Source/Virtual`
  ——实测会把 pipewire-pulse 协议搞坏（pactl 报协议错误、plasma-pa context kaput、
  系统托盘清空，仅重启 pipewire-pulse 恢复）**，健康方案是 module-remap-source
- 音频格式一律 **F32 单声道 48kHz**（PipeWire 负责重采样与声道转换，模型永远拿 48k 单声道）
- 设备枚举只用 `pw-dump` 标准 introspection（`pvplatform.audio.pwpipe_client`）

---

## 命名规范

### 品牌名
**品牌名统一为 `PureVox`**（P、V 大写）。所有用户可见文本——窗口标题、UI 文案、日志、关于对话框、README、更新日志、菜单——一律用 `PureVox`，禁止 `Purevox` / `purevox` / `PUREVOX` 等变体。

### 代码内部标识符
- 含品牌名的 Python 类/标识符统一 `PureVox...`（如 `PureVoxServer`）
- 其它标识符遵循工程约定第 4 条（Python snake_case、C++ PascalCase、Kotlin camelCase）

### 平台强制小写（非品牌变体，勿改）
各处用户可见/系统标识中的 `Purevox` 变体已统一为 `PureVox`（注册表 Run 键、防火墙规则名、单实例 Mutex 名、发行产物 `PureVox.exe`/`PureVoxMic.apk`、WakeLock `PureVoxMic:AudioWakeLock`、settings.gradle rootProject、README/手册/CSS 注释）。以下标识属**平台/协议强制小写**，改小写会破坏功能或违背平台惯例：
- Android 包名 `com.purevox.mic`（Java 包名惯例 + JNI 函数名 `Java_com_purevox_mic_*` 必须与包名逐字符匹配，含 `namespace`/`applicationId`/布局类引用）
- mDNS 服务类型 `_purevox._tcp.local.`（DNS SRV 按 RFC 小写约定）
- 用户数据目录 `~/.purevox/`、日志名 `purevox_*.log`、CA 证书 `purevox-ca.crt`（POSIX 小写路径惯例）
- 模型代号 `purevox9`（内部模型代号）
- 浏览器 localStorage key `purevox_mic_id` / `purevox_theme`
- JNI/CMake 内部名（`purevox_opus_jni`、opus_jni.c 的 native 函数，随包名）

---

## 工程约定

1. **输入自适应、输出强制 48kHz（Windows；Linux 由 PipeWire 统一转 48k）** —
   本地输入（主输入 / AEC far=mic / 回环）按设备原生采样率/声道打开，
   下混单声道后经 pvengine.Resampler 转 48k，启动不拦截；输出端启动前逐设备
   检测，失败弹框阻止，不做重采样或半双工回退。
   - **Windows 下 WASAPI 严格、MME 宽松是刻意的，勿"修"**（2026-08-13 实测，
     2026-09-24 起仅针对输出端）：
     WASAPI 共享模式锁死设备 MixFormat，MixFormat=44.1k 的设备请求 48k 即
     `paInvalidSampleRate (-9997)` 弹框阻止——这是对的，硬上会在建流时失败；
     MME 是 WDM 旧接口，驱动内部自动重采样，44.1k 硬件也能以 48k 打开并正常出声
     （PureVox 侧始终处理 48k，转换由 MME 驱动完成，合规），所以 gate 对 MME
     天然放行不弹框。两者行为差异不是 bug，不要给 MME 加严格 48k 限制。
     判定依据：设备 `defaultSampleRate=44100` 时 WASAPI 弹框、MME 正常。
2. **10ms hop 规约（全局统一时间粒度）** — 所有数据面一律按 10ms hop 前进：
   `hop = SAMPLE_RATE // 100`（48kHz → 480 样本；NFFT = 2×hop = 960），**按时间派生
   而非固定样本数**，未来多采样率/重采样时规约不变。202609 模型三件套契约与此一致
   （波形 hop 进出、STFT 在模型图内、enh_hop 滞后 1 hop）。落点清单：
   引擎 Stage 进出帧（`pvengine.context.HOP_LENGTH`，`process()` 严格校验 hop 长度）、
   平台回调块（PipeWire 桥接 `pwpipe_client.HOP`、Windows `frames_per_buffer`、
   媒体会话 `_HOP`）、桥接 FIFO 分块、网络 Opus 帧（480 样本=10ms）、Android/浏览器
   采集帧。缓冲水位（网络 acc、输出环、loopback 缓冲）取 hop 整数倍。任何新代码
   不得引入与 10ms 网格错位的固定样本块（1024/2048 等）——频谱可视化、流式解码、
   浏览器采集等一切旁路同样遵守，变换/重采样输出同样按 10ms 粒度切片。
   FFT/OLA 窗长恒为 2×hop（NFFT=960，COLA/无损重构要求，随 hop 派生，非豁免）。
3. **配置 key 按接口加后缀** — 设备键为 `<方向>_device_<接口后缀>` 与 `aec_far_sink_<接口后缀>`（如 `input_device_wasapi` / `input_device_mme` / `input_device_pulse` / `aec_far_sink_pulse`），后缀表见 `device_api.API_CONFIG_SUFFIX`；`config_manager.py` 的 `ConfigDefaults` 与 `_KEY_ORDER` 把全部接口的键**显式写全**（不做动态生成，阅读直观）；不用 `WASAPI_` 前缀，也不留无后缀的通用设备键。monitor（监听）与 AEC far 各存各的键。
3a. **推理后端（2026-08-22 起）** — 纯 Python 引擎用 onnxruntime Python 包，
   CPU 内核 dispatch 由 onnxruntime 运行时自动完成，禁止再做 AVX/SSE/NPU
   探测或编译参数干预（后端探测与恒值兼容报告接口已删除）。
4. **命名** — Python: snake_case 方法和变量；C++: snake_case 方法和 PascalCase 类；Kotlin: camelCase。
5. **错误处理** — 内部用 `try/except` + `_module_log()` 记录，不冒泡到 UI 线程；Tk UI 用 `messagebox` 提示。
6. **日志** — 统一 `logger.py` 的 `Logger` 类，层级 `dev`/`msg`/`warn`/`err`。
7. **DSP 全部收敛在 `pvengine/`** — numpy/scipy/onnxruntime 只允许出现在 pvengine 包内
   （组件 + dsp 基础件）；GUI 层（uitk）与平台层（pvplatform）不做信号处理，
   仅搬运 `List[float]` / numpy 帧。新增音频功能 = 新增一个 Stage 组件，不改管线骨架。
8. **Android 主题跟随系统** — `Theme.MaterialComponents.DayNight.NoActionBar`，亮色/深色自动切换。
9. **品牌拼写规约** — 品牌名一律 `PureVox`；`purevox` 全小写仅限平台/协议强制标识（见命名规范），改大小写视为破坏行为。
10. **许可证头** — 每个源码文件顶部必须带 GPL-3.0 版权头 + 模型声明 + `SPDX-License-Identifier: GPL-3.0-or-later`（照抄 `audio_processor.py` 顶部，按 `#`/`//` 注释风格替换）；新增文件也必须带。
11. **README 双语约定** — 默认中文 `README.md`，英文单独 `README_EN.md`；改文件名/平台结构/打包命令时两处必须同步，不得改名或删除。
12. **弹框集中在 `uitk/dialogs.py`** — 桌面端独立弹框（关于/EQ 编辑器/TSE 录音等）一律放
    `uitk/dialogs.py`，入口函数走 `open_*` / `show_*` 命名；不得在仓库根重建 `dialog_*.py` 平行实现。

---

## 注意事项

- **AEC 行级采集**: AEC 是 input 种节点（`echo_cancel` 行：一行一路 mic + far 二选一），far=扬声器走回环采集（Linux `PwBridge.open_far` 监听 monitor 源，会话内创建/销毁；Windows WASAPI loopback，共享模式**必须用引擎 MixFormat**），far=麦克风走 `MicCapture` 专用采集（不进混音）；far 样本带采集时间戳入行内 `GridHistory`（48k 时间戳网格），模型 far 输入直接取「mic 时刻 − far_delay」的历史段直达 `AecRow`（`pvengine/aec_row.py`，会话多行共享，会话内 cache 独立；时间原点由外部时钟保证，无隐藏缓冲；far 历史不足时直通 mic），不经过任何 fx。行配置变更走重启（与输入行一致，无运行时热切换）。
- **播放时钟域（2026-09 重构，勿回退）**: 设备回调是唯一主时钟；全部输出路
  （主输出/额外输出/网络输出/媒体从设备）各持一个 `pvengine.dsp.playback.PlaybackSink`
  （PI 伺服 ASRC ±3% + 预热 + 欠载静音重同步 + 封顶丢最旧），速率差/调度抖动
  由 sink 消化。**禁止在任何回调里写缓冲策略**（垫零/丢帧/复用上一帧/手写
  重采样均为平行实现）；播放正确性只在 playback.py 一处，合成测试见
  `tests/test_playback_sink.py`（CI 冒烟运行）。
- **网络模式缓冲**（未做低延迟优化，目标以稳为主，不追求最小延迟；水位全部按
  `HOP_LENGTH`=10ms 派生，ms 数即准确值）:
  - `_network_reader acc`: 目标 `HOP_LENGTH*5` (50ms)，硬顶 `HOP_LENGTH*8` (80ms，突发兜底截断)
  - 速率补偿: 稳态漂移由 PlaybackSink 伺服连续消化，acc 侧不再做 drop/pad
- **强配置（无迁移）**: `ConfigManager.load_config` 不做旧配置迁移，只保留已知键；
  旧 `WASAPI_*` / 通用设备键一律丢弃回退默认。设备键为带接口后缀的
  `<方向>_device_<接口后缀>`（如 `input_device_wasapi`、`input_device_mme`）。
- **设备列表刷新单一入口**：Tk 走 `MainWindowTk.refresh_devices()`——后台线程枚举，
  严禁 UI 线程同步枚举。
  触发点仅两个：程序启动、点击「启动/停止音频处理」。运行中引擎占着
  PyAudio，扫描会失败——**禁止**在下拉展开（`DarkCombo.on_open` 已随此决策移除）、
  弹框回调等其它时机触发枚举。
  新增触发点必须接到同一入口，禁止自建第二套枚举刷新逻辑。

### 长时间运行稳定性观察（2026-08-10 走查 + 2026-08-22 纯 py 迁移后复核）

- **viz 内存隐患已根治（2026-08-22）**：旧 C 版 `process_pipeline` 无条件向 viz 缓冲
  追加且只增不减（~1.4GB/小时）。纯 py 版 viz 改为 `BufferTapStage`：有界上限丢最旧 +
  仅在 `process_pipeline` 内临时启用，本地路径零开销，泄漏不可能再发生。
- **无数值溢出/延迟累积（安全）**：环形缓冲游标单调递增、水位阈值夹牢；AGC/EQ/压缩器
  状态皆为有界信号值。网络模式 acc 硬顶 80ms；各输出 sink 水位封顶 300ms。
- **播放时钟域已收敛（2026-09 重构）**：此前 5 条播放路径 4 种时钟策略
  （全双工内联处理/主输出帧长硬对齐/额外输出手写 ASRC/网络 drop+pad/Linux
  无节奏 push），速率差反复成病；现全部收敛到后端哑插件 + 唯一
  PlaybackSink（合成测试可验证：±2% 速率差、抖动、断流、突发均不连续有界）。
- **事件型弱点（继承自旧架构，待办）**：libpulse 流无 core error/lost 监听与
  自动重连；运行中 USB 拔插/PipeWire 重启 → 对应流失败、桥接静默失效
  （统一循环 ~2s 健康探测会退出线程走会话重启路径，但无流级自动恢复）。
  自动重连留作 TODO。

---

## 许可证

- 源码 **GPL-3.0**（SPDX: `GPL-3.0-or-later`），见 `LICENSE`
- 内置 AI 模型（`*.onnx`）**不随 GPL 授权**，归 a2heng 所有，禁止提取用于其他项目，仅随 PureVox 经授权使用 → 见 `MODEL-LICENSE.md`
- 作者另有 MIT 模型仓库可自由使用：`lightweight-denoise-48k` / `lightweight-aec-48k`（README 已写）
