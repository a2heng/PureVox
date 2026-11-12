# PureVox

[English](README_EN.md)

实时 AI 麦克风降噪 / 目标说话人提取 / 回声消除桌面应用，同时是一台**多设备混音 / 路由台**：
把麦克风、系统声音、手机、任意输入源接成输入条，经过各自的 AI 处理链，再按用途送往
扬声器（监听）、虚拟声卡（会议 / 直播）或手机（网络监听）——不是 VoiceMeeter 的复刻，
而是把 AI 处理与低延迟放在第一位的路由器。

## 功能

- **AI 降噪**：ONNX 流式模型，10 ms 一跳（48 kHz / 480 样本），单线程推理，实测 ~0.3 ms/跳（降噪小 0.28 / 中 0.30 / 大 0.51）
- **目标说话人提取（TSE）**：录 10 秒参考音，只保留这个人的声音。
- **回声消除（AEC）**：消除麦克风里混进来的扬声器声音（网课 / 直播 / 免提通话），带延时校准
  与两端电平配平。
- **OBS 音频滤镜**：噪声门、压缩器、限制器、扩展器、向上压缩器、3 段均衡器、极性反转，
  参数范围 / 默认值 / 步进与 OBS 一致。
- **31 段 EQ / 可视化**：检查器里拖 31 根柱子（±24 dB、预设、低切/高切），可视化节点画 VU 与 64 段频谱。
- **多设备混音台（GPUI 原生节点图）**：顶部一排输入、中间每个输入的处理节点、底部一排输出；
  拖动节点到输出即接通（连线就是路由），处理节点可拖动换序。总线按用途叫
  「监听 / 虚拟麦克风 / 手机」，不用 A1/A2。输入与总线都能增删。
- **设备池**：同一只设备被多列共用只打开一次；多个输入源由软件混音相加，不再报 busy。
- **输入 / 输出种类**：麦克风、系统声音回环、手机（网络麦克风 / 扬声器）、测试音、
  音频文件（音效板 / 音乐播放器）、任意输出设备。
- **虚拟麦克风**：Windows 走 VB-CABLE，Linux 自建 `purevox_mic`，OBS / 浏览器 / 会议软件直接选它。
- **手机当麦克风 / 扬声器**：局域网 HTTPS + Opus（网页版或 Android APK）。
  只传音频：手机端只有麦克风上行 / 扬声器下行，不做远程打字或按键注入。
- **全局热键 / 提示音 / 开机自启**：设置页可开关、录制启停热键；音效板每个音效单独录制热键；启动/停止提示音可选预设并试听。
- **系统托盘**：最小化（-）和关闭（×）都收进托盘；托盘菜单可显示/隐藏、启停引擎、退出。
- **界面语言**：中文 / English 切换在设置页（缺译条目回退中文）。
- **调试优先**：独立调试窗口与只读 HTTP 接口 `http://127.0.0.1:47821/debug` 同源同字段
  （系统指标、每路流的速率/缓冲/欠载/丢弃、模型推理耗时、频谱波形）。

## 状态

音频引擎完整：输入采集与重采样、输出与时钟伺服、降噪 / TSE / AEC、OBS 滤镜、设备池与混音、
网络（含 Android 客户端）、虚拟驱动、媒体行。界面已从 Tauri/WebView 迁移到 GPUI 节点画板；
AEC 校准、TSE 参考录制、媒体节点的检查器都已接入；设置页有语言 / 开机自启 / 热键 / 提示音开关。
窗口几何记忆、Linux tar.gz/deb 打包、托盘（Windows 完整、Linux 启停/退出）都已接入。

- 下载：见 [Releases](https://github.com/a2heng/PureVox/releases)（版本号 = 年.月.日）。
- 旧实现（Python + Tkinter）已删除；需要时用 `git log` 查历史。
- 设计： [AGENTS.md](AGENTS.md)；平台与工具链现状：[BUILD.md](BUILD.md)；协作约定：[AGENTS.md](AGENTS.md)。

## 快速开始

```bash
# 开发构建 + 运行
cargo build --manifest-path src-tauri/Cargo.toml
./src-tauri/target/debug/purevox.exe

# 测试 / 门禁
cargo test  --manifest-path src-tauri/Cargo.toml --bin purevox
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

安装包流水线尚未恢复（去掉 Tauri bundler 后需改成 cargo + NSIS/AppImage/deb），
详见 `BUILD.md`。

## 依赖

**运行期**

| 平台 | 需要 | 说明 |
| --- | --- | --- |
| Windows | GPU（DX11/12） | GPUI 用 GPU 绘制界面；核显即可 |
| Windows | VB-CABLE（可选） | 想把它当虚拟麦克风给 OBS / 会议软件用时安装 |
| Windows | `opus.dll` | 随包分发，网络音频用；不装也能跑其它功能 |
| Linux | ALSA | 音频 I/O（GPUI 需要 X11/Wayland 与 GPU） |
| Linux | PipeWire 或 PulseAudio | 设备枚举与采集（`pactl` / `parec` / `pw-dump`） |
| Linux | `libopus0`（可选） | 网络音频；缺失时该功能给出明确原因，其余正常 |
| Linux | `pulseaudio-utils`（`paplay`） | 启动 / 停止提示音；缺失时只少这一声，其余正常 |
| Linux | `nvidia-smi` 或 `amdgpu` sysfs（可选） | 调试面板的 GPU 读数；没有就不显示，不影响音频 |
| Linux | 桌面项（`.desktop`，安装包自带） | **全局热键**需要桌面应用标识：从应用菜单/图标启动才有，终端直跑会被门户拒绝（界面会说明） |
| Linux | `pkexec`（deb/rpm 更新时） | 图形授权后交给包管理器安装 |
| Android | Android 8.0+（API 26） | 网页版客户端无需安装，浏览器打开电脑地址即可 |

**构建期**

- Rust（`rust-version = "1.95"`）；Linux 需 `libasound2-dev pkg-config` 与 GPUI 的
  X11/Wayland/fontconfig/Vulkan 依赖（完整清单见 `BUILD.md`）。
- Python 3（`tools/automation/` 的门禁 / 打包 / 发版脚本）；界面是 Rust，不需要 Node/tsc。
- `onnxruntime` 由 `ort` 在构建时下载；Linux 也可链接系统/自备的 `libonnxruntime.so`
  （见 `BUILD.md` 的 `ORT_*` 环境变量说明）。

## 模型与授权

- 源码：[GPL-3.0-or-later](LICENSE)，第三方库声明见 [LICENSE-THIRD-PARTY.txt](LICENSE-THIRD-PARTY.txt)。
- 内置模型（`models/*.onnx`）**不随 GPL 授权**，仅可随 PureVox 使用，见 [MODEL-LICENSE.md](MODEL-LICENSE.md)。
- 想自己训练 / 换成 MIT 授权的模型：
  [lightweight-denoise-48k](https://github.com/a2heng/lightweight-denoise-48k)（降噪）、
  [lightweight-aec-48k](https://github.com/a2heng/lightweight-aec-48k)（回声消除）。
  ⚠ 这两个仓库是**旧的波形 hop 契约**（STFT 在图内）；内置模型从 2026-10-29 起是**谱进谱出**
  （`(1,2,481)` 复数谱 + 图外 FFT，见 `models/README.md`），所以**不能直接塞进 `models/`**
  —— 得先按同一份契约重新导出（加载时会明确报「不是谱接口」）。

Copyright (C) 2024-2026 a2heng <752848283@qq.com>
