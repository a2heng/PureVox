---
name: purevox-testing
description: PureVox 无人值守 / 不改用户环境的验证手法（**以 Windows + Git Bash 为主**）：隔离 profile 起 app、HTTP 调试接口读数、PrintWindow 截图（Windows only）、无头 Chrome 截调试面板、TLS WebSocket 重连压测、Android adb/UIAutomator 验收，以及收尾纪律。改音频/网络/界面/安卓端或要「跑一遍确认」时使用。
---

# PureVox 验证手册（agent 用，Windows + Git Bash 为主）

> **平台说明**：本手册的命令默认在 **Windows 本机 + Git Bash** 上跑（这是本仓库 agent 的日常环境）。
> 标注 **Windows only** 的部分（`PrintWindow` 截图、`Stop-Process`、PowerShell 5.1 编码坑）在 Linux/macOS
> 不适用；Linux 用 `tools/automation/ui.py`（AT-SPI）+ 系统截图，macOS 用 `screencapture`。
> 构建 / HTTP 读数 / 重连脚本 / adb 这些部分三平台通用（路径按平台换）。

> 目标：**一次跑完、自己收尾、不碰用户的环境与音量**。所有结论从 `/debug` 或结构化 JSON 来，
> 不靠听、不靠肉眼猜；截图只证明布局，不当测量依据。

## 0. 铁律

1. **不模拟键鼠点界面**（Windows 会把按键送进别的窗口）。要驱动界面用：
   - 安卓：`adb shell input tap` + `uiautomator dump`；
   - 桌面：Tauri 命令 / HTTP 调试接口 / Windows UIAutomation（见 `TAURI3.md` §3.6）。
2. **不碰 `~/.purevox`**：用临时 `USERPROFILE` / `HOME` 起隔离实例。
3. **自己起的进程自己杀**；命令带超时；不许把运行中的 app 留给用户。
4. **不改系统音量 / 静音**；测试音输出到 VB-CABLE 等虚拟设备，别外放。
5. 端口冲突先查：`netstat -ano | findstr :47821`、`:59123`；调试 HTTP 固定 47821，
   网络服务端口可改（临时 profile 的 `settings.json` 里设一个空闲端口）。
6. 运行中的 `purevox.exe` 会锁住 `target/debug/purevox.exe`。**别杀用户的进程**，
   用独立 `CARGO_TARGET_DIR` 构建（见 §1）。

## 1. 构建（不覆盖正在跑的 exe；三平台通用）

```bash
# Git Bash；临时目录按机器选
export PV_TARGET="$LOCALAPPDATA/Temp/pv-target"   # 或 C:/Users/<你>/AppData/Local/Temp/pv-target
CARGO_TARGET_DIR="$PV_TARGET" cargo build --manifest-path src-tauri/Cargo.toml
# 产物：$PV_TARGET/debug/purevox.exe
```

- 依赖已有缓存时增量构建约 5~10 s；新依赖首次会久一些。
- 前端资源（`src-tauri/ui/*`）是**编译期嵌入**的：改完 JS/CSS 必须重新 `cargo build`
  （必要时 `touch src-tauri/src/main.rs` 强制重编），否则跑的仍是旧界面。
- Linux 门禁用 WSL `Ubuntu-24.04`；本机 Windows 的 cargo 只编 Windows。

## 2. 隔离启动 + 调试钩子

```bash
export PV_HOME="C:/Users/<你>/AppData/Local/Temp/pv-home"
mkdir -p "$PV_HOME/.purevox"
# 需要特定计划就写 session.json；不写就是默认一列
USERPROFILE="$PV_HOME" HOME="$PV_HOME" \
  PUREVOX_AUTOCALIB_MS=600000 \        # 自动启动会话并保持 10 分钟（之后才会尝试校准）
  "$PV_TARGET/debug/purevox.exe" >"$PV_HOME/run.log" 2>&1 &
```

钩子（都只在设了环境变量时生效，release 同样存在，方便复现）：

| 变量 | 作用 |
| --- | --- |
| `PUREVOX_AUTONET=1` | 启动后自动开网络服务（手机/网页客户端可连） |
| `PUREVOX_AUTOCALIB_MS=<ms>` | 自动 `start()` 会话，`<ms>` 后跑一次 AEC 校准；**设大值 = 只为把引擎跑起来** |
| `PUREVOX_AUTODEBUG=1` | 自动打开系统浏览器调试面板（本机 HTTP 47821） |

读状态（中文用 Python/json，不用 PowerShell 解码）：

```bash
curl -s --max-time 5 http://127.0.0.1:47821/debug > /tmp/dbg.json
python -c "import json;d=json.load(open('/tmp/dbg.json',encoding='utf-8'));print(d['app']['pid'],d['audio']['verdict'])"
curl -s http://127.0.0.1:47821/debug/viz | python -c "import sys,json;print(json.load(sys.stdin)['viz']['frames'])"
```

常用端点：`/debug`（汇总）、`/debug/audio`、`/debug/devices`、`/debug/calib`、`/debug/viz`、
`/debug/net`（在汇总里）、`/debug/ui`（调试面板页面）。

## 3. 截图（不抢焦点）

- **主窗口（Windows only）**：用本 skill 的 `scripts/shot-window.ps1`（`PrintWindow` +
  `PW_RENDERFULLCONTENT`，DPI-aware）。它在窗口被别的程序盖住时也能拿到内容，**不要**用
  `SetForegroundWindow`+`CopyFromScreen`（Windows 会拒绝后台抢焦点，结果截到的是浏览器）。
  Linux 用 `tools/automation/ui.py` 驱动 + `gnome-screenshot`/`import`；macOS 用 `screencapture -l <windowid>`。
- **第二个 WebView2 窗口是废的**（页面不导航、整窗纯白）：不要试图截它，调试面板走浏览器。
- **调试面板**：无头 Chrome 直接截 HTTP 页面，无需开窗口：

```bash
CHROME="$LOCALAPPDATA/Google/Chrome/Application/chrome.exe"
"$CHROME" --headless=new --disable-gpu --hide-scrollbars \
  --window-size=1600,1400 --virtual-time-budget=6000 \
  --screenshot="$LOCALAPPDATA/Temp/debug-page.png" \
  http://127.0.0.1:47821/debug/ui
```

拿到 PNG 再读图（模型能直接看）。截图只用于确认布局/渲染，**不用于取数**。

## 4. 数值验收（音频 / AGC / EQ / 可视化）

- **要真设备**的测量默认不编译：`cargo test --features hw-tests clock_probe -- --nocapture`。
- 用 `tone` 源 + 两个 `viz` 行把「处理前 / 处理后」夹住，读 `/debug/viz` 的 `peak_dbfs` /
  `rms_dbfs`，比用耳朵可靠：

```json
{"columns":[{"rows":[
 {"kind":"input","ptype":"tone","device":null,"enabled":true,"params":{}},
 {"kind":"process","ptype":"viz","device":null,"enabled":true,"params":{"mode":"vu"}},
 {"kind":"process","ptype":"agc","device":null,"enabled":true,"params":{"target_db":"-30","speed":"0.002","floor_db":"-50"}},
 {"kind":"process","ptype":"viz","device":null,"enabled":true,"params":{"mode":"vu"}},
 {"kind":"output","ptype":"audio_output","device":"wasapi:{...VB-CABLE...}","enabled":true,"params":{}}
]}],"enabled":true}
```

- 测试音默认 −20 dBFS；AGC 的 `target_db` 是 **RMS 目标**（正弦峰值 ≈ 目标 +3 dB）。
- EQ 曲线用 `eq_curve` 命令 / 点 `/debug` 看 `engine_info` 的 row detail，别在前端另算。
- 输出一律指向 VB-CABLE 等虚拟设备，**别外放**。

## 5. 网络 / 重连压测

用 `scripts/ws-reconnect.py <port> <rounds>`：
- 真 TLS + WebSocket 握手（忽略自签证书），发 `hello(proto=3)`、`sub`，然后**直接断 TCP**
  （模拟手机「重新连接」），重复 N 次；
- 同时看两件事：**进程还活着**、`run.log` 里没有 `panicked`；再抽查 `ready.target_ms`
  是否等于电脑当前环深度（重连不该退回 20）。

```bash
python .opencode/skills/purevox-testing/scripts/ws-reconnect.py 59123 20
grep -c panicked "$PV_HOME/run.log"
```

## 6. Android（Windows 本机已连 USB；adb 命令跨平台）

```bash
export JAVA_HOME='D:/AndroidDev/jdk/jdk-17.0.13+11'
export ANDROID_HOME='D:/AndroidDev/sdk'
export ANDROID_SDK_ROOT="$ANDROID_HOME"
cd android && ./gradlew :app:assembleDebug
adb install -r app/build/outputs/apk/debug/PureVox-*-debug-android-arm64.apk
adb shell am force-stop com.purevox.mic.debug
adb shell am start -n com.purevox.mic.debug/com.purevox.mic.MainActivity
```

- **Git Bash 会把 `/sdcard/...` 改写成 `C:/Program Files/Git/...`**：调 adb 前加
  `MSYS_NO_PATHCONV=1`，否则 `pull`/`cat` 全部找不到文件。
- UI 树：`MSYS_NO_PATHCONV=1 adb shell uiautomator dump /sdcard/ui.xml`，再
  `MSYS_NO_PATHCONV=1 adb pull /sdcard/ui.xml "$TEMP/ui.xml"`，按 `resource-id` /
  `content-desc` / `bounds` 找控件；点击用 `adb shell input tap x y`（中心点）。
- 扫码：点 `btnScan` 后 dump 一次，确认相机权限弹窗或 `PvScanActivity` 出现；
  `adb logcat -d | grep -i purevox` 看异常。
- 清单验收：`aapt2 dump xmltree <apk> --file AndroidManifest.xml | grep -E 'CAMERA|CaptureActivity'`。
- 重连 / 扫码切电脑后**连接被旧回调关掉**是曾经的真 bug：验收时重复「扫码两次 / 重连两次」，
  断言仍处于连接态（旧连接回调必须按 client 身份丢弃）。

## 7. 门禁（提交前）

```bash
python tools/automation/check.py            # fmt/clippy/build/test/ui/version/bundle
# 或单项：python tools/automation/check.py -Gate clippy
node tools/automation/i18n_lint.js          # 0 error / 0 warning
cd src-tauri/ui && tsc -p jsconfig.json --noEmit
```

- `check.py` 的 `clippy` 会先碰 crate 根强制重编；看到 `Fresh` 不代表刚编过。
- Android 不参与 `check.py`；改 Kotlin/隐私清单时另外 `assembleDebug`。
- 提交前确认 `git status` 里没有临时 profile / PNG / APK。

## 8. 收尾（必须）

```bash
powershell -NoProfile -Command "Get-Process purevox -ErrorAction SilentlyContinue | Where-Object {$_.Path -like '*pv-target*'} | Stop-Process -Force"
adb shell am force-stop com.purevox.mic.debug   # 只在测试包上
```

- 只杀自己起的（路径/临时 profile 判定），**不要**动用户正在跑的 `purevox.exe`。
- 删掉临时 `run.log` / PNG / `session.json`（整个临时 profile 可留作复现，但别塞进仓库）。
