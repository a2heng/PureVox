---
name: purevox-linux
description: Use when working on PureVox's Linux runtime, host dependencies, or distro packaging — libpulse/pipewire-pulse/libopus 宿主依赖、deb Depends / rpm Requires、stage_payload.sh 清单、requirements-linux.txt、compileall_srcs.txt、AOSC/Ubuntu 安装命令、.venv / verify-local.sh / PUREVOX_TEST_VIRTUAL_MIC 本地验证、pipewire-pulse 排障。Linux 音频内部管线见 purevox-architecture。
---

# PureVox Linux 适配

> Linux 侧只做「宿主依赖 + 打包 + 验证 + 排障」。音频数据流/虚拟麦克风/libpulse 桥的内部实现
> 见 `purevox-architecture` skill；跨模块硬准则见 `AGENTS.md`。

## 1. 运行期宿主依赖（2026-08-22 纯 py 迁移后，不再编译任何扩展）

| 依赖 | 用途 | 加载方式 |
|---|---|---|
| `libpulse0` | PulseAudio **客户端库**（录制/播放/虚拟声卡控制的核心） | `ctypes.util.find_library("libpulse")`（`pvplatform/audio/_libpulse.py:93`） |
| `pipewire` + `pipewire-pulse` | 提供 PulseAudio 协议 socket + PipeWire 图 | libpulse 连接 `$XDG_RUNTIME_DIR/pulse/native` |
| `libopus0` | opuslib 解码网络推流 | `find_library('opus')`；缺则 `OPUS_AVAILABLE=False` 优雅降级 |
| `pw-dump` / `pactl` / `pw-cli` | 设备枚举、虚拟麦克风创建 | 子进程（来自 `pipewire-bin` / `pulseaudio-utils`） |

- **不需要** gcc / pkgconf / `libpipewire-0.3-devel`：没有编译步骤，禁止再向用户/文档推荐开发包。
- **服务是 `pipewire-pulse` 而不是 `pipewire`**：排障命令 `systemctl --user status pipewire-pulse`。
- **ALSA 接口已整体移除**：不要再用 `alsamixer` / `plughw` / `pulse:` 指导排障。

## 2. 打包适配（deb / rpm / AppImage 共用同一份 payload 清单）

- 文件清单唯一定义在 **`tools/automation/stage_payload.sh`**，`pack_deb.sh` / `pack_rpm.sh` /
  `pack_appimage.sh` 全部调用它（保证三种格式文件集一致）。**在仓库根新增任何顶层 `.py` 模块，
  必须同步登记进该脚本**，否则 Linux 包缺文件 → 运行时 `ImportError`。
  - `qr_tk.py` 已登记（注意 `uitk/main_window.py:1509` 是**惰性 `from qr_tk import ...`
    且调用处无 try/except**，一旦漏登记，网络输入行一渲染就 `ImportError`）。
- `requirements-linux.txt` 必须列全运行期 import。**跨平台共享模块用到的第三方库不能只写在
  `requirements-win.txt`**（`qr_tk.py` 需要 `qrcode`，已加入 linux 依赖）。
- `tools/automation/compileall_srcs.txt` 是 CI 语法门清单，也须含根目录 py 文件
  （`qr_tk.py`、`logger.py` 已登记）。
- **包依赖声明须覆盖第 1 节**（已声明）：
  - deb `Depends: pipewire, pipewire-pulse, libpulse0, libopus0`。注意 `pipewire` 包本身
    ***不会*拉入 `pipewire-pulse`，也不依赖 `libpulse0`**（它只依赖 libpipewire-0.3-modules /
    pipewire-bin / init-system-helpers），二者必须显式声明。
  - rpm `Requires: pipewire, pipewire-pulseaudio, pulseaudio-libs, opus`。

## 3. 本地开发 / 兼容性验证（Ubuntu）

仓库根 `.venv`（gitignore）装 `requirements-linux.txt`；重建：

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements-linux.txt
.venv/bin/python tools/automation/smoke.py     # 引擎冒烟：导入 + 加载模型跑一帧
.venv/bin/python tests/run_all.py              # 全套（纯函数/合成 DSP/传输/设备）
```

- 实测 **Ubuntu 24.04.5 + Python 3.12.3**：smoke 与 `tests/run_all.py` 全绿，
  含真实设备枚举（`pw-dump` 5 输入/6 输出）、`SpeakerCapture`、`PwBridge`/`MediaSession`。
- 虚拟声卡真机测试需 PipeWire 运行 + `pactl`，并设 **`PUREVOX_TEST_VIRTUAL_MIC=1`**；
  否则 `test_devices.py` 只做「优雅降级」断言（`ready=False`）。
- `bash verify-local.sh smoke|deb|appimage|all` 是本地 CI 镜像（自动装 sysdeps，必要时调
  `bootstrap_python312.sh` 下预编译内嵌 3.12）。

## 4. 发行版安装命令

- AOSC：`sudo oma install -y python3 pipewire`（按第 1 节补 libpulse / pipewire-pulse / libopus）
- Ubuntu/Debian：`sudo apt install pipewire pipewire-pulse libpulse0 libopus0`
- 文档里遗留的 `gcc pkgconf libpipewire-0.3-devel`、`pip install -r requirements.txt` 均已失效
  （纯 py 不编译；且仓库只有 `requirements-linux.txt` / `requirements-win.txt`）。

## 5. 排障

- 虚拟麦克风 = 单生产者 `purevox_out` + 双出口（monitor / `purevox_mic` remap）；**禁**用
  `module-null-sink media.class=Audio/Source/Virtual` 建第二路源，**禁**用重启 `pipewire-pulse`
  去"修托盘"——细节和原因见 `purevox-architecture`。
- 流级自动重连尚未实现（USB 拔插/PipeWire 重启后需重启会话），属已知 TODO。
