---
name: purevox-architecture
description: Use when navigating PureVox architecture or touching Linux audio plumbing — 桌面模块速查 (run_tk/uitk/session_plan/audio_processor/pvengine/pvplatform/server/html)、Linux pipewire-pulse + ctypes libpulse 桥与虚拟麦克风 purevox_out/purevox_mic、pw-dump 设备枚举、Android 模块、WSS 网络推流协议与 480 帧。这是 AGENTS.md「架构」小节的按需展开。
---

# PureVox 架构速查

> 本文件是 `AGENTS.md`「架构」小节的完整版（拆分出来以便按需加载）。
> **顶层设计与规范见 `DESIGN.md`**（分层架构/节点模型/数据流不变量/SessionPlan 契约/
> 扩展指南）。本节是模块速查；实现与 DESIGN.md 冲突时以 DESIGN.md 为准。

### 桌面端 (Python)

| 模块 | 职责 |
|---|---|
| `run_tk.py` | 单实例锁、启动入口，导入 `uitk.main_window.MainWindowTk` |
| `uitk/` | 主 UI（纯标准库 Tkinter）——**单列节点面板**：顶部单一工具条（启动/退出 · 添加节点▾ · 清空 · 设置▾[快捷键/自动运行/开机自启/系统声音/虚拟声卡/关于]）+ PluginPanel（输入/处理/输出/可视化全部为可增删排序的节点行；排序走**拖拽手柄**）；EQ 曲线编辑器 / TSE 参考录音 / 关于页（文档标签）都在 `uitk/dialogs.py`；VB-CABLE 状态卡内嵌在虚拟输出行。设备选择在 input/output 节点行内（Linux 存 node.name）。**外观为星露谷像素浅色主题**（theme.py 单份令牌定义），无明暗切换 |
| `about_content.py` + `about/` | **关于页文本（无 GUI 依赖的单一来源）**——`about/changelog.md` 更新日志、`about/windows.md`、`about/linux.md` 使用手册（uitk 直接读文件渲染）；`about_content.py` 只存元数据（应用信息/URLS/LIBS）与介绍页、许可证页文本，打包须随包携带 `about/` |
| `session_plan.py` | **L3 会话层**——`SessionPlan.from_chain(chain_cfg)` 纯函数：链文档 → 校验后的可执行计划（inputs/outputs/remote_url/viz/fx_chain/aec_rows/aec_far_mics/loopbacks/problems/warnings）。UI 启动流程只消费计划，不做内联解析 |
| `audio_processor.py` | 核心音频线程 —— `AudioThread`(**统一处理循环** read→process→sinks.write；本地/网络同一循环)、每输出一个 `PlaybackSink`(pvengine)、后端装配(`_create_stream`：Linux=PwBridge / Windows=PaBridge，哑传输)、行级 AEC（`echo_cancel` 输入行：一路一路装配 far 采集 + `AecRow`）+ 回环输入行（`loopback`，时间戳网格历史逐 hop 出队进混音）、`SpeakerCapture`(回环采集)/`MicCapture`(麦克风专用采集)、`RingBuffer`、设备枚举、TSE 参考录音工具(`_recorder`/`load_tse_reference`/`_wsola_time_stretch`) |
| `pvengine/` | **纯 Python 组件化音频引擎**——Stage 接口（process/reset/release）是唯一契约；`components/`(denoise/tse/gain/eq/vad/agc/compressor/clip/recorder/tap) 每文件一个组件、按 active_modes 声明生效模式；`aec_row.py`(行级 AEC：一行一路，far 与 mic 按外部时钟配对——far 入 GridHistory，取「mic 时刻−far_delay」的回声源直达，会话多行共享)；`dsp/`(窗/环形缓冲/重采样/**PlaybackSink 跨时钟域播放**/**GridHistory 时间戳网格配对（外部时钟 QPC/perf）**/Mel 频谱/ONNX 会话) 可独立复用；`pipeline.py` 按序执行+模式旁路；`processor.py` 是 AudioProcessor 门面。模型：202609 三件套（denoise/aec/tse，波形 hop [1,480] 进出、STFT 在模型图内、enh_hop 滞后 1 hop），全部 numpy + onnxruntime 实现 |
| `pvplatform/` | 平台抽象层 —— `audio/`(SpeakerCapture 三端[回环采集]/MicCapture[麦克风专用采集]、device_api、backends[后端注册表]、pwpipe_client[ctypes libpulse 桥，read_each + 多路 far 专用流]、pa_backend[Windows PortAudio 桥]、media_session[miniaudio 纯媒体会话]、_libpulse[libpulse 最小绑定])、`system/`(单实例/自启动/防火墙/虚拟麦克风，win+posix) |
| `server/` | 远程麦克风 HTTPS/WSS 服务器 —— `https_server.py`、`audio_bridge.py`(RemoteAudioSource)、`opus_codec.py`、`mdns_publisher.py`、`tls_manager.py` |
| `config_manager.py` | JSON 配置读写（强配置，无迁移）；api_type 平台感知默认值、设备键按接口后缀（`<方向>_device_<接口后缀>` / `aec_far_sink_<接口后缀>`，全部接口显式写全） |
| `model_config.py` | ONNX 模型文件名常量 |
| `html/` | 浏览器端远程推流页面 —— `index.html`、`app.js`、`audio-capture.js`、`pcm-worklet.js`（AudioWorklet 采集恒 480 帧切分）、`ws-client.js`、Opus WASM 编码器 |
| `build_win.ps1` / `pack_deb.sh` / `pack_rpm.sh` / `pack_appimage.sh` | Windows 产物目录打包（PyInstaller，CI 上传自动压缩）/ Linux deb / rpm / AppImage 打包。全部产物为纯 Python（依赖随内嵌 python312 或系统环境携带），无任何自编译二进制 |

### Linux 音频架构（pipewire-pulse 兼容层 + 自研 ctypes libpulse 绑定，强制）

数据流（本地）：麦克风源 → libpulse 录制流（读回调→输入环）→ pvengine 降噪 → 每输出 PlaybackSink → libpulse 播放流（写回调按设备时钟 pull）→ `purevox_out`（虚拟麦克风 sink）
监听：独立录制流指向扬声器 monitor 源（同一路降噪音频）
AEC far-end：独立录制流指向 `far_sink.monitor`（扬声器）或麦克风真源（far=mic），一行一路，会话内创建/销毁，恒 48kHz 单声道；`loopback` 回环输入行与 AEC far=扬声器继承同一套回环采集机制

- 实现：`pvplatform/audio/_libpulse.py`（系统 libpulse 的最小 ctypes 绑定，
  `pa_threaded_mainloop` + pa_stream 读写回调）+ `pwpipe_client.py` 的
  `PwBridge`。**不用 pulsectl**——其流式 API（connect_recording 等）在
  PyPI 全版本中不存在（旧代码调的是未记录 fork，干净安装必断）。
  无任何自编译二进制。
- 时钟模型：**设备回调是唯一主时钟**。播放 = libpulse 写回调(nbytes) →
  `out_pull[i](n)`（PlaybackSink.pull，速率差由 sink 伺服消化）→ 写流；
  录制 = 读回调 → 各输入独立环形缓冲（200ms）→ 引擎线程 read(hop) 混合。
  桥内零缓冲策略，播放正确性只在 `pvengine/dsp/playback.py`。
- 格式协商 **F32 单声道 48000Hz**：PipeWire 内置重采样 + 声道转换，模型永远拿 48k 单声道，
  输出自动上混到目标设备声道数
- 虚拟麦克风（Linux 虚拟声卡）= **单一生产者 + 双出口**，实现见 `pvplatform/system/_posix.py`：
  - 生产者：单声道 null-sink `purevox_out`（`pw-cli create-node`，唯一写入口，
    `media.class=Audio/Sink`、`audio.position=[MONO]`、`object.linger=true`）。
    PureVox 降噪输出流只写入它。
  - 出口 1 `purevox_out.monitor`（monitor 源，宽口径）；出口 2 `purevox_mic`
    （真源，`module-remap-source` 重映射而来，供 OBS 等"只列真源"软件）。
  - 生命周期全幂等：`virtual_mic_ready()` → `ensure_virtual_mic()` → `remove_virtual_mic()`。
  - **启动不自动创建**：菜单「虚拟声卡」→ Tk 状态面板手动「创建/清理」。
- **禁用/踩坑**（违反任一即弄坏系统托盘/协议）：
  - `pw-loopback`：旧虚拟麦克风架构，已弃用，仅防御性 `pkill` 清残留。
  - `module-null-sink media.class=Audio/Source/Virtual` 建第二路真源：实测把
    **pipewire-pulse 协议状态弄坏**（pactl 报协议错误、plasma-pa context kaput、
    系统托盘清空）。真源必须用 `module-remap-source`。
  - **重启 pipewire-pulse "修托盘"**：plasma-pa 的 libpulse context 变 kaput、托盘清空。
  - remap-source 会强制覆盖 node.description（显示 "Remapped ... source"），set-param 改不掉。
  - **ALSA 备选接口已整体移除（2026-08-22）**：旧混合实现（输入 plughw/pulse:、
    输出经 PipeWire 原生流写 purevox_out）连同 alsa_client.c/pvalsa.py 一并删除；
    单一实现路径 = libpulse 绑定桥。历史踩坑结论（默认 source 抢占回读、snd_pcm_drain 阻塞等）
    不再适用。
- 设备列表（pw-dump）：Linux **按声卡枚举设备**——一个声卡有多个接口各对应真实设备
  （数字麦 Mic1 / 模拟麦 Mic2 / 扬声器 / HDMI 等）。PureVox 自身输入 = Audio/Source 物理
  麦克风（排除 PureVox-* 流、purevox* 虚拟源[对外输出，选它当输入会回授]、error 死节点）。
  **禁止按 api.alsa.path 无 `,dev` 把板载卡接口当"假设备"排除**。输出 = Audio/Sink
  节点（扬声器 + `purevox_out`）
- VU 电平显示**降噪输出峰值**（`_pw_loop` 里取 `out`，勿改成输入 `data`）
- UI 下拉框直接显示节点名（node.name），真实节点名存 userData，读下拉框一律走 `_combo_value()`

### Android 端 (Kotlin)

| 模块 | 职责 |
|---|---|
| `MainActivity.kt` | 主界面 —— 服务器发现、连接、推流控制、VU 显示、调试信息、RTT 追踪 |
| `audio/AudioCapture.kt` | AudioRecord 采集 48kHz/16bit，帧大小 480 (10ms) |
| `audio/OpusEncoder.kt` | JNI 调用 native opus 编码 |
| `network/WsClient.kt` | OkHttp WebSocket 客户端，base64 Opus 推流，ack RTT 追踪 |
| `network/TlsHelper.kt` | 自签名证书信任 |
| `discovery/MdnsDiscovery.kt` / `SubnetScanner.kt` | mDNS 发现 + 子网扫描备用 |
| `service/StreamService.kt` | 前台服务保活 + WakeLock |
| `VuMeterView.kt` | 自定义 VU 表绘制 |

### 网络推流协议

```
浏览器/Android → WSS → Python 服务器 → audio_processor pipeline → 扬声器

客户端 JSON: {"type":"audio","data":"<base64 opus>","seq":N,"timestamp":T}
服务器 ACK:  {"type":"ack","seq":N}
服务器 API:  GET /api/status → {"sample_rate":48000, "active_clients":N, ...}
```

帧大小 480 samples (10ms @48kHz) —— Opus 编码器 (JS WASM / Android JNI) 与 Python 解码器、引擎 hop 对齐。
