# BUILD — 构建、运行、验证（GPUI）

> Tauri / WebView2 已整体移除。界面是 **GPUI + GPUI Kit**（纯 Rust），
> 本文件是唯一的构建入口；旧 Tauri 文档只在 git 历史里。

## 1. 环境

- Windows 10/11 + VS Build Tools（MSVC）+ Rust stable（`1.95+`，本机 1.99）。
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
- 端点能力（需要真实声卡）：`wasapi_caps` / `wasapi_lowlat` 两个 `#[ignore]` 测试。
- 界面自动化：见 `.opencode/skills/purevox-testing/SKILL.md`（隔离 profile + HTTP 读数 +
  PrintWindow 截图；GPUI 窗口同样适用）。
- Android：`cd android && ./gradlew :app:compileDebugKotlin`（或 `assembleDebug`）。

## 5. 打包

**当前未恢复安装包流水线**：去掉 Tauri bundler 后，`.github/workflows/release.yml`
仍引用旧的 tauri-cli 步骤，发版前必须替换为「cargo build + NSIS / AppImage / deb」。
在那之前只发布开发构建 / 手动压缩目录（`models/` 与 `opus.dll` 必须与 exe 同级）。
`RELEASE.md` 里的版本号与产物命名规则仍然有效，打包命令部分待重写。

## 6. 常见坑

- 运行中的 exe 会锁住 `target/debug/purevox.exe`：用独立 `CARGO_TARGET_DIR` 构建。
- 改完界面直接跑旧二进制是没用的（GPUI 原生，没有前端热更新）；重新 `cargo build`。
- 模型目录：开发态用 `PUREVOX_MODEL_DIR` 指向仓库 `models/`；打包后放 exe 同级 `models/`。
- 调试接口固定 `47821`，隔离实例用 `PUREVOX_DEBUG_PORT` 改端口。
