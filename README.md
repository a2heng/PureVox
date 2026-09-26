# PureVox

实时 AI 音频降噪工具 —— 降噪 / 目标说话人提取 / 回声消除，支持本地麦克风与远程网络推流。

中文 | [English](README_EN.md)

## 文档

- 📖 使用说明与更新日志均内置于软件，菜单「关于」查看（软件介绍 / Windows / Linux 使用说明 / 更新日志 / 许可证）

## 功能特性

- 🎤 实时 AI 降噪（48kHz，模型按需加载）
- 🗣️ TSE 目标说话人提取（录制参考语音后从背景中分离目标人声）
- 🔊 AEC 回声消除
- 🎛️ 均衡器（EQ）：10 段（1 倍频程）/ 31 段（1/3 倍频程）/ 61 段（1/6 倍频程）
- 📊 AGC 自动增益控制 / VAD 静音检测
- 📱 远程麦克风：手机浏览器 / Android APK 经局域网推流到 PC 处理
- 🖥️ Windows（WASAPI 默认 / MME 备选）与 Linux（pipewire-pulse 兼容层 + libpulse）双平台

## 环境要求

| 平台 | 要求 |
|---|---|
| Windows | Windows 10/11，Python 3.12+ |
| Linux | Python 3.12+，PipeWire + pipewire-pulse + libpulse（libopus 可选） |

> **⚠️ Windows 7 支持已终止**：自 Python 3.13 起不再支持 Win7；`v2026.08.14.1643` 是最后支持 Win7 的版本，需继续在 Win7 使用请下载 [此 tag 的 Windows 产物](https://github.com/a2heng/PureVox/releases/tag/v2026.08.14.1643) 并停用更新。

## 快速开始

### 内嵌 Python 3.12（推荐，独立于系统环境）

项目可自带一份独立的 Python 3.12，与系统 Python 完全隔离，不会互相影响。
引导脚本按需下载官方预编译 CPython 包（Windows 走 NuGet 完整包；Linux 下载
python-build-standalone install_only 包，解压即用、不编译）。产物都放在 `packages/` 下。

```bash
# Linux：
./bootstrap_python312.sh          # -> packages/python312（自包含），并安装依赖
./py312 run_tk.py            # 启动

# Windows（PowerShell，NuGet 下载预编译）
powershell -ExecutionPolicy Bypass -File bootstrap_python312.ps1   # -> packages\python312w
# 之后 build_win.ps1 打包会自动使用 packages\python312w\python.exe，独立于系统 Python
```

若不想用内嵌解释器，也可直接使用系统 Python 3.12+：

```bash
# Windows 系统 Python：
pip install -r requirements-win.txt

python run_tk.py
```

### Linux（AOSC / 其它发行版）

```bash
# 系统级依赖（例：AOSC；Debian/Ubuntu: apt install pipewire pipewire-pulse libpulse0 libopus0）
sudo oma install -y python3 pipewire

# 内嵌 3.12 方式（推荐，见上）：
./bootstrap_python312.sh
./py312 run_tk.py

# 或用系统 python3 直接运行：
pip install --user -r requirements-linux.txt
python3 run_tk.py
```

Linux 音频走 pipewire-pulse 兼容层（自研 ctypes 绑定直调系统 libpulse），
格式协商 F32 单声道 48000Hz，重采样与声道转换由 PipeWire 负责，无任何自编译二进制。
虚拟麦克风 = 单声道 null-sink `purevox_out`（唯一写入口）+ 两个出口：
monitor 源 **"PureVox 虚拟麦克风"** 与重映射真源 **"PureVox mic"**（供 OBS 等只列真源的软件）。
AEC 远端采集（回声参考）与回环输入同样走该兼容层（监听扬声器 monitor 源）。

### Windows 远程麦克风附加组件

远程麦克风功能的 Opus 解码库已随仓库提供（`server/opus.dll`，预编译 libopus，BSD），
只需自行安装 **VB-CABLE** 虚拟声卡：

- 从 [vb-audio.com/Cable](https://vb-audio.com/Cable/) 下载 `VBCABLE_Setup_x64.exe`
  （或直接下载[官方驱动包](https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip)），
  双击运行安装程序，按提示完成即可。检测到未安装时，PureVox 会在音频输出行内显示安装指引
  （含[安装视频教程](https://www.bilibili.com/video/BV1i2bazGEKe/)）。

## 打包

### Windows（产物目录，CI 打包上传）

```powershell
powershell -ExecutionPolicy Bypass -File build_win.ps1   # 产出 dist/PureVox/（PyInstaller one-folder 产物）
```

脚本包含完整流程：PyInstaller one-folder 打包（自动使用 packages\python312w\python.exe）→ tcl/tk 与无用模块清理 → 拷贝文档。
Windows CI 会执行同样流程，上传 `dist/PureVox/` 目录（`actions/upload-artifact` 自动压缩为 zip）。

### Linux（deb / rpm / AppImage）

```bash
bash pack_deb.sh        # 产出 dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.deb，内含源码+模型+html
bash pack_rpm.sh        # 产出 dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.rpm（Fedora/RHEL，同样捆绑内嵌 Python3.12）
bash pack_appimage.sh   # 产出 dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.AppImage（捆绑内嵌 Python3.12）
```

| 产物 | 命名规则 |
|---|---|
| Windows（CI 上传目录自动压缩） | `PureVox-Windows-x64-<date>-release` |
| Linux deb/rpm/AppImage | `PureVox-Linux-x64-<date>-release.<deb\|rpm\|AppImage>` |
| Android APK | `PureVox-Android-arm64-<date>-debug.apk` |

时间戳 `<date>` = `yyyy-MM-dd-HHmm`（文件名）；产物体内版本字段 = `yyyy.MM.dd.HHmm`
（如 `2026.08.10.1517`），**由 tag 名推导**（`v2026.08.10.1517` → `2026.08.10.1517`），
deb/rpm/setup.py 版本号同源一致，不随 job 运行时刻漂移。

发版：推送 tag `v<yyyy.MM.dd.HHmm>`（如 `v2026.08.10.1517`）即自动触发 CI，release job
会创建 GitHub Release 并把全部产物（deb / rpm / AppImage / Windows zip / APK）attach 上去。

### Android APK

```bash
cd android
./gradlew assembleDebug    # 输出 android/app/build/outputs/apk/debug/app-debug.apk
```

环境要求：JDK 17、Android SDK platform 34、NDK r27。首次编译需将 Opus 源码放到
`android/opus-src/`（[xiph/opus v1.5.2](https://github.com/xiph/opus/archive/refs/tags/v1.5.2.zip)）。

## 远程麦克风

手机 / 浏览器 → WSS(Opus) → PC 服务器 → AI 处理链路 → 扬声器 / 虚拟麦克风

```
手机 → https://<PC的IP>:59123（mDNS 广播 _purevox._tcp.local.）→ 降噪 → 输出
```

- 浏览器：手机与 PC 同局域网，访问 `https://<PC的IP>:59123`，信任自签名证书后点麦克风推流
- APK：打开自动搜索局域网服务器，发现即自动连接推流
- 客户端消息：`{"type":"audio","data":"<base64 opus>","seq":N}`，服务器回 `{"type":"ack","seq":N}`
- 帧大小 480 samples（10ms @48kHz），与 Opus 编码器、引擎 hop 对齐

## 项目结构

```
run_tk.py                 # 启动入口（单实例锁 + Tk 主窗口）
uitk/                     # 桌面 UI（纯标准库 Tkinter）：节点面板、EQ 编辑器、关于页
about_content.py          # 关于页元数据/介绍/许可证文本（更新日志与手册在 about/*.md）
audio_processor.py        # 核心音频线程（采集/播放/网络循环）+ TSE 参考录音工具
pvengine/                 # 纯 Python 组件化音频引擎（numpy + scipy + onnxruntime）
pvplatform/               # 平台抽象：audio/（设备枚举、SpeakerCapture）、system/（单实例/虚拟麦克风）
config_manager.py         # JSON 配置（强配置，按接口隔离设备键）
model_config.py           # ONNX 模型文件名常量
server/                   # 远程麦克风 HTTPS/WSS 服务端（aiohttp + Opus + mDNS + TLS）
html/                     # 浏览器推流前端（AudioWorklet + Opus WASM）
purevox-web/              # Lite 两档的浏览器版（ONNX Runtime Web；页面瘦、重资源按 URL 缓存）
android/                  # Android 客户端（Kotlin + OkHttp + Opus JNI）
pack_deb.sh               # Linux deb 打包
pack_rpm.sh               # Linux rpm 打包（Fedora/RHEL）
pack_appimage.sh          # Linux AppImage 打包（捆绑内嵌 Python 3.12）
build_win.ps1             # Windows 打包（PyInstaller 产物目录）
bootstrap_python312.sh / .ps1  # 内嵌 Python 3.12 引导（Linux 下载预编译包，Windows 拉 NuGet）
```

## 技术栈

| 组件 | 技术 |
|---|---|
| 桌面 GUI | Python 标准库 Tkinter（uitk，星露谷像素浅色主题） |
| 音频处理 | 纯 Python 引擎 pvengine（numpy + scipy + onnxruntime） |
| Linux 音频 | PipeWire（pipewire-pulse 兼容层，自研 ctypes 绑定直调系统 libpulse） |
| Windows 音频 | WASAPI（默认）/ MME 备选；独立输入/输出流 + 每输出 PlaybackSink |
| 服务端 | Python aiohttp + zeroconf + cryptography |
| 音频编码 | Opus（PC: opuslib，APK: NDK 编译，Web: WASM） |
| Android | Kotlin + OkHttp + NsdManager + AudioRecord |

## 许可证

- **源代码**：[GPL-3.0](LICENSE)（GNU General Public License v3.0 or later）
- **内置 AI 模型**：不随 GPL 授权，归作者 a2heng 所有，禁止提取用于其他项目，
  仅可在 PureVox 内经授权使用 —— 详见 [MODEL-LICENSE.md](MODEL-LICENSE.md)

作者的 MIT 开源模型仓库（较早版本，可自由使用）：

- <https://github.com/a2heng/lightweight-denoise-48k>
- <https://github.com/a2heng/lightweight-aec-48k>

第三方组件（ONNX Runtime、Opus、NumPy 等）使用各自许可证，
见 [LICENSE-THIRD-PARTY.txt](LICENSE-THIRD-PARTY.txt)。

## 联系方式

- GitHub: <https://a2heng.github.io/>
- 哔哩哔哩: <https://space.bilibili.com/10850943>
