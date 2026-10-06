---
name: purevox-build
description: Use when running, bootstrapping, building, or packaging PureVox desktop/Android — bootstrap_python312.sh/.ps1、run_tk.py、build_win.ps1、pack_deb.sh / pack_rpm.sh / pack_appimage.sh、Android gradlew assembleDebug、内嵌 Python 3.12、deb/rpm/AppImage 产物布局、.ps1 纯 ASCII 约束。这是 AGENTS.md「运行 / 构建」小节的按需展开。
---

# PureVox 运行 / 构建 / 打包

> 本文件是 `AGENTS.md`「运行 / 构建」小节的完整版（拆分出来以便按需加载）。
> 跨模块硬准则（10ms hop、48kHz、单一实现路径等）始终见 `AGENTS.md`。

**内嵌 Python 3.12（推荐，独立于系统环境）**：本项目可自带独立 Python 3.12，
与系统 Python 完全隔离。Windows 走 NuGet 下载预编译包；Linux 源码由引导脚本
**按需下载官方 CPython@v3.12.11 tarball**（一次性，缓存于 `~/.cache/purevox`，
可用 `PUREVOX_CPYTHON_TARBALL` 指定离线包）后 out-of-tree 一次性编译。
产物统一放 `packages/`。

- `./bootstrap_python312.sh`（Linux，幂等）→ 生成自包含 `packages/python312/` + 装依赖
  （**不编译**：下载 python-build-standalone 预编译 CPython install_only 包，
  版本锁定 cpython-3.12.14+20260814，可用 `PUREVOX_CPYTHON_TARBALL` 指定离线包）
- `./bootstrap_python312.ps1`（Windows）→ 生成 `packages\python312w\`（NuGet 完整版，含头文件/链接库）
- 内嵌解释器与系统 Python 互相独立；`packages/python312*`、`.py312-src/` 不进版本库（gitignore）
- **不再使用 git 子模块、也不再源码编译**（2026-08-25 改预编译）：bootstrap 下载
  python-build-standalone install_only 包解压即用；CI 缓存 key 固定为 pbs 包版本号；
  Linux job 系统依赖不再需要 libssl-dev/libffi-dev/zlib1g-dev/build-essential

### Windows (PowerShell)

```powershell
chcp 65001
# 方式一（内嵌 3.12，推荐）：
powershell -ExecutionPolicy Bypass -File bootstrap_python312.ps1
# 方式二（系统 Python）：pip install -r requirements-win.txt
python run_tk.py
powershell -ExecutionPolicy Bypass -File build_win.ps1   # 打包产物目录 dist/PureVox/（自动用 packages\python312w\python.exe）
```

**`.ps1` 脚本必须纯 ASCII（英文）**：`build_win.ps1` / `bootstrap_python312.ps1`
不含中文/非 ASCII/BOM。Windows PowerShell 5.1 对无 BOM 的 UTF-8 脚本按 ANSI
(cp1252/GBK) 误读导致语法错误（`chcp 65001` 只在本机掩盖）；脚本须引用中文
文件名时用通配符（`*.html`）匹配，不写字面量。

### Linux

依赖因发行版而异（参考 `.github/workflows/release.yml` 与 README）。AOSC 示例：

```bash
sudo oma install -y python3 pipewire
# 内嵌 3.12（推荐）：
./bootstrap_python312.sh
./py312 run_tk.py
bash pack_deb.sh                              # deb → dist/PureVox-Linux-x64-<date>-release.deb
bash pack_rpm.sh                              # rpm → dist/PureVox-Linux-x64-<date>-release.rpm
bash pack_appimage.sh                         # AppImage → dist/PureVox-Linux-x64-<date>-release.AppImage
```

deb 布局：`/opt/purevox/` 放全部源码+模型+html+捆绑的内嵌 `python312`（含 numpy/onnxruntime/scipy 等
全部 pip 依赖，无任何自编译 .so）；`/usr/bin/purevox` 启动脚本直接 exec。
rpm（pack_rpm.sh）与 AppImage 是同一实现路径，同样捆绑内嵌 python312，Requires/无系统 Python 依赖。
`/usr/share/applications/purevox.desktop` + hicolor 图标。Depends 只留 pipewire。
Linux 输入/输出/设备枚举/AEC 全走 pipewire-pulse（libpulse 绑定桥）；opuslib 缺失时 `pip install --user`。

### Web 瘦页面（Lite 两档的浏览器版，面向 GitHub Pages）

```bash
python purevox-web/build/build_web.py            # 打包（默认 202609b 中号）
python purevox-web/build/build_web.py --model a  # 换档：a 小号 / b 中号 / c 大号
python purevox-web/build/build_web.py --base-url https://cdn.example.com/pv/assets   # 重资源走 CDN
python purevox-web/build/build_web.py --list-models
python purevox-web/serve.py --open               # 本地 HTTPS 伺服（默认 59124）
```

- 产物 `purevox-web/dist/`：`index.html`、`mic.html`（= lite_mic 物理设备）、
  `out.html`（lite_net 输出端：WS 订阅 + 降噪 + 播放）、`in.html`（lite_net 输入端：
  麦克风 + WS 推送，不降噪）、`assets/ort/*`（ONNX Runtime Web 运行时）、
  `assets/models/*.onnx`（降噪模型）、`build.json`（清单：版本/字节数/sha256）。
  第三方运行时另存 `purevox-web/vendor/`（按 `purevox-web/build/ort.lock.json`
  下载并校验 sha512）。**产物与 vendor 都不入版本库**（见 .gitignore），
  入库的只有 `src/` `workers/` `build/` `serve.py`。
- **瘦身口径**：页面本体只含应用代码（约 95~100KB）；ORT 运行时与模型
  按 URL 加载，**三个页面共用同一份**（谁先加载谁填缓存）。URL 稳定、不带版本
  查询串——内容指纹在 `build.json` 里而不塞 URL，这样重资源能被浏览器/CDN 长期
  缓存（URL 不变即不失效）。别再把 20MB 级资源 base64 内联：凭空多 33% 体积，
  且每次改页面都要重传。
- ⚠️ **必须走安全上下文**：浏览器只在安全上下文（HTTPS 或 localhost）里给麦克风
  权限，所以 `file://` 双击打不开。用 `serve.py` 起 HTTPS
  （重资源发 `max-age=31536000, immutable`，页面发 `no-cache`），或部署到
  GitHub Pages。证书复用桌面端那份 PureVox Local CA（`~/.purevox/ca/`），
  全产品只需信任一次；首次访问在证书警告里点「继续前往」。
- Net 版网络链路 = **`serve.py` 当纯服务器 + 两个独立页面**（浏览器不能监听端口，
  跨设备必须有个进程 listen）：`GET /api/lan` 报网卡列表（浏览器无 API 可枚举
  本机局域网 IP，只能服务端给），`WS /ws/up` 收输入端推上来的 Int16 帧，
  `WS /ws/down` 原样转给输出端。**服务端不解码、不降噪、不碰音频**，
  只倒字节；降噪只在输出页做（与 Mic 页同一条 Pipeline + 同一个模型）。
  音频格式 Int16 / 单声道 / 48kHz / 每帧 480 样本（10ms）。
  输出页选局域网 IP → 页内二维码即输入页地址（`15-qr.js` 自带编码器，
  无第三方依赖），手机扫码后两边各点一次启动即连通，零手工交换。
- 跨站部署（github.io）：页面与中继不同源时用 `?srv=host:port` 指定中继
  （进页面即记住，输入框可改；scheme 缺省跟页面走）；跨站时二维码自动出
  「本站 in.html + `?srv=`」形式。`serve.py` 已带 CORS 与 Private Network
  Access 预检头；手机真机仍需先信任自签证书且同网段。
  本地模拟部署后：中继跑 59124，另起一个 `serve.py`（纯当静态）跑 59127，
  打开 `https://127.0.0.1:59127/out.html?srv=127.0.0.1:59124` 即跨 origin。
- 部署范围：io 上只实装 Mic 页。Net 版（in/out）最终未上 io——浏览器页面
  开不了系统级服务端口，纯静态站上服务端—客户端直连做不到；本地 `serve.py`
  起服务实测可用，代码保留（`serve.py` 的中继 + `40-in/out-app.js`）。

### Android

```powershell
$env:ANDROID_HOME = "D:\Android\Sdk"; $env:ANDROID_SDK_ROOT = "D:\Android\Sdk"
$env:ANDROID_NDK_HOME = "D:\Android\Sdk\ndk\27.0.12077973"
cd android
.\gradlew.bat assembleDebug    # 输出 android/app/build/outputs/apk/debug/
.\gradlew.bat installDebug     # 安装到设备
```

要求：JDK 17、SDK platform 34、NDK 27、CMake 3.22.1。首次编译需 Opus 源码放到
`android/opus-src/`（gitignore，JNI CMake 引用该路径）。
