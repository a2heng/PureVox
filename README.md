# PureVox

[English](README_EN.md)

实时 AI 麦克风降噪 / 目标说话人提取 / 回声消除桌面应用。把麦克风、系统声音、手机、任意输入源
按列串起来处理，再送到扬声器、虚拟声卡或手机——用一条可编辑的音频链路替代「几个软件 + 一堆
驱动」的拼装。

## 功能

- **AI 降噪**：ONNX 流式模型，10 ms 一跳（48 kHz / 480 样本），单线程推理，实测 ~1.8–6 ms/跳。
- **目标说话人提取（TSE）**：录 10 秒参考音，只保留这个人的声音。
- **回声消除（AEC）**：消除麦克风里混进来的扬声器声音（网课 / 直播 / 免提通话），带延时校准
  与两端电平配平。
- **OBS 音频滤镜**：噪声门、压缩器、限制器、扩展器、向上压缩器、3 段均衡器、极性反转，
  参数范围 / 默认值 / 步进与 OBS 一致。
- **多列并行链路**：每列「输入 → 若干处理行 → 输出」，列可增删、行可拖动、行/列都能单独挂起。
- **设备池**：同一只设备被多列共用只打开一次；多个输入源由软件混音相加，不再报 busy。
- **输入 / 输出种类**：麦克风、系统声音回环、手机（网络麦克风 / 扬声器）、测试音、
  音频文件（音效板 / 音乐播放器）、任意输出设备。
- **虚拟麦克风**：Windows 走 VB-CABLE，Linux 自建 `purevox_mic`，OBS / 浏览器 / 会议软件直接选它。
- **手机当麦克风 / 扬声器**：局域网 HTTPS + Opus（网页版或 Android APK），另有可选的远程输入
  （打字 / 键盘，**默认关闭**）。
- **常驻托盘**：启动/停止、显隐窗口、退出；最小化与关闭都收到托盘；全局热键；中英文界面。
- **调试优先**：顶栏调试面板与只读 HTTP 接口 `http://127.0.0.1:47821/debug` 同源同字段
  （系统指标、每路流的速率/缓冲/欠载/丢弃、模型推理耗时、频谱波形）。

## 状态

功能已完整（不再是迁移占位版）：输入采集与重采样、输出与时钟伺服、降噪 / TSE / AEC、
OBS 滤镜、设备池与混音、网络（含 Android 客户端）、虚拟驱动、媒体行、更新与打包都已落地。

- 下载：见 [Releases](https://github.com/a2heng/PureVox/releases)（版本号 = 年.月.日）。
- 旧实现（Python + Tkinter）：`legacy-v2026.09.30.1944/`，只读归档，不参与构建。
- 设计： [DESIGN.md](DESIGN.md)；平台与工具链现状：[TAURI3.md](TAURI3.md)；协作约定：[AGENTS.md](AGENTS.md)。

## 快速开始

```bash
# 开发运行（Windows / Linux 均可）
cd src-tauri && cargo tauri dev

# 出安装包：Windows = NSIS exe；Linux = deb / rpm / AppImage
cd src-tauri && cargo tauri build
```

首次构建需要联网：Rust 依赖、`onnxruntime`（`ort` 的 `download-binaries`）、以及 Tauri 的打包工具。
Linux 还需要系统依赖（见下）。

## 依赖

**运行期**

| 平台 | 需要 | 说明 |
| --- | --- | --- |
| Windows | WebView2 运行时 | Win10/11 一般自带；缺失时系统会提示安装 |
| Windows | VB-CABLE（可选） | 想把它当虚拟麦克风给 OBS / 会议软件用时安装 |
| Windows | `opus.dll` | 随包分发，网络音频用；不装也能跑其它功能 |
| Linux | WebKitGTK 4.1、GTK3、ALSA | 界面与音频 I/O |
| Linux | PipeWire 或 PulseAudio | 设备枚举与采集（`pactl` / `parec` / `pw-dump`） |
| Linux | `libopus0`（可选） | 网络音频；缺失时该功能给出明确原因，其余正常 |
| Linux | `pkexec`（deb/rpm 更新时） | 图形授权后交给包管理器安装 |
| Android | Android 8.0+（API 26） | 网页版客户端无需安装，浏览器打开电脑地址即可 |

**构建期**

- Rust（`rust-version = "1.95"`）+ `tauri-cli` 3.0 alpha（`cargo install tauri-cli --version 3.0.0-alpha.5`，本仓库钉这个版本）；Linux 需 `libwebkit2gtk-4.1-dev libssl-dev
  libxdo-dev librsvg2-dev libgtk-3-dev libasound2-dev pkg-config file rpm`。
- Node.js 24（`tsc` + i18n 门禁）与 Python 3（`tools/automation/` 的构建/发版脚本）。
- `onnxruntime` 由 `ort` 在构建时下载；Linux 也可链接系统/自备的 `libonnxruntime.so`
  （见 `TAURI3.md` 的 `ORT_*` 环境变量说明）。

## 模型与授权

- 源码：[GPL-3.0-or-later](LICENSE)，第三方库声明见 [LICENSE-THIRD-PARTY.txt](LICENSE-THIRD-PARTY.txt)。
- 内置模型（`models/*.onnx`）**不随 GPL 授权**，仅可随 PureVox 使用，见 [MODEL-LICENSE.md](MODEL-LICENSE.md)。
- 想自己训练 / 换成 MIT 授权的模型：
  [lightweight-denoise-48k](https://github.com/a2heng/lightweight-denoise-48k)（降噪）、
  [lightweight-aec-48k](https://github.com/a2heng/lightweight-aec-48k)（回声消除）。
  两者与内置模型同一套 10 ms / 48 kHz 流式契约，可直接替换 `models/` 下的文件。

Copyright (C) 2024-2026 a2heng <752848283@qq.com>
