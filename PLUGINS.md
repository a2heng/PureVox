# PLUGINS — 节点清单与处理组件

> 从 `ARCHITECTURE.md` §4 拆出。AEC/TSE/媒体/OBS 滤镜/31 段 EQ/AGC/可视化的实现约定都在这里。

## 4. 节点清单

| 类别 | ptype（示例） | 说明 |
| --- | --- | --- |
| 输入 | `audio_input` / `loopback` / `remote_mic` / `echo_cancel` | 源；同列多个混合 |
| 处理 | `denoise_*` / `gain` / `noise_gate` / `compressor` / `limiter` / `expander` / `upward_compressor` / `eq31` / `agc` / `invert` / `soundpad` / `player` | Stage；就地处理 |
| 输出 | `audio_output` / `remote_speaker` | 汇；位置 tap，送往设备或网络 |
| 可视化 | `viz` | **处理行**；只读 tap，原样放行（VU / 频谱，见 §3.6） |

- **可视化插件（`viz`，2026-10-27）**：挂在链路任意位置，显示**该位置**的电平与频谱。
  设计取自 legacy `uitk/viz.py`（用户指定）：分段 LED 电平条（绿 -60..-20 / 黄 -20..-9 /
  红 -9..0，0.1 s 快攻、90 dB/s 慢放、峰值保持 10 s 后 20 dB/s 回落）+ 对数分段频谱柱
  （底槽连续、三色按 dB 分区、无网格）。新实现：FFT 960（2×hop）在列线程每 hop 做一次，
  64 段对数频率由 `dsp::bands` 统一映射；`mode`（电平 + 频谱 / 电平 / 频谱）只是**怎么画**，
  改它不重建会话（`persist_plan`）。测量值进 `VizHub`，界面 10 Hz 轮询 `viz_snapshot`，
  `/debug` 的 `viz` 段与 `GET /debug/viz` 是同一份。

- **OBS 音频滤镜（参考 `~/obs-studio/plugins/obs-filters`）**：已实现
  Audio Gain / Noise Gate / Compressor / Limiter / Expander / Upward Compressor /
  Invert Polarity；参数默认值、范围、步进与 OBS 一致（`registry.rs` + `components/obs_dsp.rs`）。
  3 段 EQ 已被 **31 段均衡 `eq31`** 取代（`biquad` crate，ISO 1/3 倍频程 + 低切/高切 +
  曲线 + 6+6 预设，`ui/eq.js`）；另有 **`agc`**（`dagc` crate，三个热参数）。
  两者都是处理行，曲线/参数走热参数，不重建会话。
  不包含 Noise Suppression（由 AI 降噪替代）、VST 2.x（需要 VST 宿主，未引入）、以及 Audio
  Monitor / Pan / Sync Offset（它们是 OBS 的源属性/路由；PureVox 的输出行承担监听）。
- **AEC 行**：一个输入行 =「一路 mic + 一路 far」，mic 在行内先过 AEC 再进列；
  far 参考只给模型用，**不进信号路径**；历史不足则直通 mic。
- **桌面声音（`loopback` 输入行）**：捕获某只输出设备正在播放的系统混音作为源
  （复用 AEC 的远端回环实现：Windows = WASAPI loopback，Linux = PipeWire monitor）。
  `device` = `loopback`（系统默认输出）或 `loopback:<端点ID｜sink 名>`；带行内增益
  `volume_db`（-30~6 dB，默认 -6 dB，系统混音偏响）。
- **媒体行（`soundpad` / `player`）是处理行，不是输入行**：无采集设备，在自身链位置
  把媒体样本与信号相加——挂链尾（默认）= 后级直通，媒体不经上游处理；拖到降噪之前
  则参与处理（与 legacy 链注入语义一致，也符合 §3.2 的位置 tap 语义）。
  - `soundpad`（音效板）：音效整段入内存（都是短音效，起播零延迟），起播/结束/
    手动停止/重播一律 hop 内线性淡变（杜绝咔哒，**首样本恒为 0**）；重播同一音效 = 旧声部
    淡出、新声部从**开头**起播；音量**每个音效一个**（±10 dB），拖动实时生效。
    每个音效可绑**一个全局热键**（录键 + 清除），热键**存在计划里**（`pads` 参数的
    `hotkey` 字段）——跟着音效走，增删音效即重注册，不另设一份设置。
  - `player`（音乐播放器）：流式解码（后台线程 → 有界环形缓冲 → 音频线程每 hop 取
    480 样本，内存恒定），曲目视作首尾相连的**环流**（EOF 回卷续播）；播放/暂停/seek/
    音量；位置经 `resume_sec` 持久化（暂停/拖动/退出即写）。
  - **媒体库**：导入时一律归一为 48 kHz 单声道 16-bit WAV，存 `~/.purevox/soundpad/`
    （音效）或 `~/.purevox/music/`（曲目）；文件名 `<原名>-<sha1 前 8 位>.wav`
    （同内容幂等复用）；配置只引用库内路径（不依赖外部原始文件）。
  - **控制面**：播放/停止/seek/音量是**运行时命令**（不是计划参数），经全局
    `MediaHub` 按 (列, 行) 寻址下发（与 `NetHub` 同款单例）；音量等热参数同时经
    `persist_plan` 落盘**不重建会话**；增删音效 / 换曲 = 结构性变更 → `apply_plan` 重建。
- **网络行**（本轮新增）：
  - `remote_mic` = 手机麦克风当电脑麦克风。**不需要虚拟声卡驱动**：网络流直接成为引擎的
    「源」，因此能进降噪 / TSE / AEC 全链。全会话只允许一个（全局单例）。
    自带去回声开关（`params.aec`）：勾上后近端 = 网络入站、远端 = `far_device`，
    与独立 `echo_cancel` 行是**同一个 `AecRow` / 同一个内置模型**（`MODEL_AEC`，
    计划里不存 `model`、界面不给选），校准与报告也同一套（§4.1.11）。
  - `remote_speaker` = 手机扬声器当电脑音箱。按 §3.2 位置 tap 取信号，允许多个（都收到同一份）；
    该信号经 Opus 编码后广播给所有已订阅客户端。
  - 协议、缓冲与安全边界见 §4.1。
