# BUILD — 构建、运行、验证（GPUI）

> Tauri / WebView2 已整体移除。界面是 **GPUI + GPUI Kit**（纯 Rust），
> 本文件是唯一的构建入口；旧 Tauri 文档只在 git 历史里。

## 1. 环境

- Windows 10/11 + VS Build Tools（MSVC）+ Rust stable（`1.95+`，本机 1.99）+ Python 3。
- Linux 构建依赖（Ubuntu 24.04 实测，与 GPUI Kit 安装文档一致）：
  ```bash
  sudo apt install -y gcc g++ clang libfontconfig-dev libwayland-dev \
    libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev \
    vulkan-validationlayers libvulkan1 pkg-config file
  ```
- ONNX Runtime 不走下载：`~/.cargo/config.toml` 里已有（不进仓库）：
  ```toml
  [env]
  ORT_LIB_LOCATION = "C:/Users/<你>/.local/ort-win"
  ORT_PREFER_DYNAMIC_LINK = "1"
  ORT_SKIP_DOWNLOAD = "1"

  [target.x86_64-pc-windows-msvc]
  rustflags = ["-C", "link-arg=DirectML.lib"]
  ```
- Android：`D:/AndroidDev/jdk` + `D:/AndroidDev/sdk`（见 `android/README.md`）。

## 2. 开发构建

```bash
export PATH="/c/Users/<你>/.cargo/bin:$PATH"
cd /d/PureVox

cargo build --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt   --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml --bin purevox
```

产物：`src-tauri/target/debug/purevox.exe`。

## 3. 运行（隔离实例）

```bash
export PV_HOME="C:/Users/<你>/AppData/Local/Temp/pv-mix"
mkdir -p "$PV_HOME/.purevox"
USERPROFILE="$PV_HOME" HOME="$PV_HOME" \
  PUREVOX_DEBUG_PORT=47823 \
  PUREVOX_MODEL_DIR="D:/PureVox/models" \
  ./src-tauri/target/debug/purevox.exe
```

验证钩子：`PUREVOX_MIX_AUTOSTART=1` 启动后自动开引擎；`PUREVOX_AUTONET=1` 自动开网络；
`PUREVOX_AUTODEBUG=1` 自动打开浏览器调试面板。

## 4. 测试

- Rust 单测 / 集成：`cargo test --bin purevox`（含协议、时钟、混音、延迟分析）。
- 采样热路径基准（重采样 + 交织拆分，真实实现原样引入）：
  `cargo run --release --example resampler_bench`（输出 ns/块 与单核占用）。
- 端点能力（需要真实声卡）：`wasapi_caps` / `wasapi_lowlat` 两个 `#[ignore]` 测试。
- 界面自动化：见 `.opencode/skills/purevox-testing/SKILL.md`（隔离 profile + HTTP 读数 +
  PrintWindow 截图；GPUI 窗口同样适用）。
- **门禁只有一份实现**：`python tools/automation/check.py`（fmt / clippy / build / test /
  version / bundle）。CI 的 `tests.yml` 与本地跑同一条命令，不要另写一套脚本。
- UI 设计取值先看 `UI.md` 的「设计系统」；界面原语在 `src-tauri/src/ui/widgets.rs`，
  新页面不要复制按钮/字段样式。
- Android：`cd android && ./gradlew :app:compileDebugKotlin`（或 `assembleDebug`）。

## 5. 打包

**Windows 安装包用 NSIS**（Tauri 同款形状：按用户安装、开始菜单/桌面快捷方式、卸载项）：

```bash
python tools/automation/package_windows.py          # cargo build --release + NSIS
python tools/automation/package_windows.py --no-build
```

产物：`dist/PureVox_<版本>_x64-setup.exe`（需要 `makensis`：`choco install nsis`）。
Linux 的 AppImage / deb / rpm 流水线待恢复。

`RELEASE.md` 里的版本号与产物命名规则仍然有效，打包命令部分待重写。

## 6. 常见坑

- 运行中的 exe 会锁住 `target/debug/purevox.exe`：用独立 `CARGO_TARGET_DIR` 构建。
- 改完界面直接跑旧二进制是没用的（GPUI 原生，没有前端热更新）；重新 `cargo build`。
- 模型目录：开发态用 `PUREVOX_MODEL_DIR` 指向仓库 `models/`；打包后放 exe 同级 `models/`。
- 调试接口固定 `47821`，隔离实例用 `PUREVOX_DEBUG_PORT` 改端口。
