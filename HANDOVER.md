# 交接：Windows 输出侧低延迟改造

> 分支 `latency-probe`，2026-11-10。**本文是这一轮工作的交接单，不是长期文档**；
> 稳定结论已经并进 `ARCHITECTURE.md`（§音频缓冲表 + 2026-11-10 两段实测）和 `DEBUG.md`（§1.1）。

## 0. 一句话现状

输出已经**绕开 cpal 直连 WASAPI**：CABLE 上回调从 480 帧（10 ms）降到 **128 帧（2.67 ms）**，
播放设备环 19.9 → **10 ms**，60 s 实测 `underrun_per_s = 0`。
无处理本地路径从 66.9~81 ms 降到 **65~73 ms**（剩下的成本在采集侧和四段 10 ms 环）。
下一步是 P2（单 pull 图），预计能到 40~45 ms。

## 1. 分支

| 分支 | 状态 | 说明 |
| --- | --- | --- |
| `latency-probe` | 已推 `origin`，**当前分支** | 本轮全部工作 |
| `main` | `8a79b99` | 主干，没有本轮改动 |
| `p1-mixer-fusion` | 本地，**不要 merge** | 混音并入播放线程的试验，实测 10 ms 环下不降反升且有欠载 |
| `deps-update` | 本地，未验证 | `cargo update`（tauri 3.0.0-alpha.6 等 64 个包） |
| `gpui` | worktree | 另一条线的前端试验，与延迟无关 |

## 2. 本轮提交（从旧到新）

| 提交 | 作用 |
| --- | --- |
| `6ad379d` | 逐列延迟探测（实测处理链 + 缓冲归因）+ 隔离实例端口覆盖 |
| `f9a6aa7` | 降噪/TSE 改相位时退到能量包络判延迟 |
| `09afcaf` | 包络窗口延到 1 s（300 ms 语音自相似，主/次峰分不开） |
| `a10c3b1` | 采集侧 PI 时钟伺服；输出/混音水位 40/30/30 → 20/20/20 |
| `0ef9ddd` | 延迟分段说明从常量取，避免文档漂移 |
| `26e48a6` | 48k/设备/混音水位降到 10 ms + 采集/播放高精度等待 |
| `1b425cf` | 10 ms 版本实测（68 ms，逐段归因） |
| `a947da6` | 采集/播放/混音工作线程进 MMCSS |
| `7eb48b2` | **端点能力探测**：`wasapi_caps` / `wasapi_lowlat` 两个 `#[ignore]` 测试 |
| `77443b8` | **输出直连 WASAPI**（`IAudioClient3` 128 帧低延迟共享）+ 设备环 1 帧粒度补水 |
| `ba024e5` | 文档：`ARCHITECTURE.md` / `DEBUG.md` |

## 3. 关键实测（这台机器，2026-11-10）

### 3.1 端点能力（`cargo test --bin purevox wasapi_caps -- --ignored --nocapture`）

| 端点 | `GetDevicePeriod` min | `IAudioClient3` min 周期 | 独占 |
| --- | --- | --- | --- |
| **CABLE Input (VB-Cable)** | 2.00 ms (96 f) | **128 f（2.67 ms）**，fundamental=1 | 混音格式不支持 |
| Headphone (Realtek) | 3.00 ms (144 f) | 480 f（不支持低延迟） | 不支持 |
| Headset Microphone | 3.00 ms | 480 f | 不支持 |
| CABLE Output | 3.00 ms | 480 f | 不支持 |

独占对**所有**端点都返回 `0x88890008`（32-bit float 不支持独占；要用得改成 16 bit 再试）。

### 3.2 128 帧共享低延迟（`wasapi_lowlat`，3 s）

```
buffer=280 f (5.83 ms)
回调 1126 次，块大小 p50=128 p99=128 max=280
间隔 ms: p50=2.66 p99=5.06 max=7.06      ← 加了 MMCSS
（不加 MMCSS：max=15.09，>10 ms 有 4 次 —— 尾巴是线程调度，不是引擎）
```

### 3.3 整机 60 s（4 列 profile）

| 指标 | 改前（cpal） | 改后 |
| --- | --- | --- |
| 输出回调块 | 480 帧 / 10~20 ms，偶有 1056 帧（22 ms） | 128 帧 / 2.67 ms，恒定 |
| 播放设备环 | 19.9 ms | 10.0 ms |
| `underrun_per_s` | 0.35 ~ 133 | **0** |
| 时钟伺服 | +30000 ppm 打满 | −41 ppm |
| `overrun_per_s` | 0（正常时） | 0 |
| 无处理列合计 | 66.9 ~ 81 ms | 65 ~ 73 ms |

**为什么只降了 10 来 ms**：设备环省了 10 ms、输出回调省了 7 ms，但同一轮里采集环水位飘到
18 ms（伺服设定点 10、硬件顶 60），把一半收益吃掉了。采集侧是 Realtek 480 帧回调的抖动。

### 3.4 现在的延迟账（无处理列）

```
10    输入回调（Realtek 480 帧，降不了）
10    采集环
1.3   采集重采样
10    行输入环
0     处理
10    混音源环
10    播放 48k 环
10    播放设备环
1.3   播放重采样
2.7   输出回调（128 帧）
```

## 4. 代码地图

| 文件 | 作用 |
| --- | --- |
| `src-tauri/src/audio/out.rs` | **输出后端选择层**：定义 `Format` / `Sample`，按平台 re-export |
| `src-tauri/src/audio/wasapi_out.rs` | Windows 输出：`probe()` 问格式+周期，`start()` 建流；回调线程 `run()` |
| `src-tauri/src/audio/cpal_out.rs` | 非 Windows 输出（cpal），接口与上面一致 |
| `src-tauri/src/audio/playback.rs` | 播放工作线程：48k 环 + 伺服 + 转换器 + 设备环补水 |
| `src-tauri/src/audio/capture.rs` | 采集（仍是 cpal）+ PI 时钟伺服；`CallbackStats` 已改 `pub(crate)` |
| `src-tauri/src/audio/loopback_windows.rs` | AEC 回环；`resolve_device` / `read_format` 改 `pub(super)` 供输出复用；**两个探测测试**也在文件末尾 |
| `src-tauri/src/engine/latency.rs` | 逐列延迟探针 + 分段预算 + `/debug/latency` |
| `src-tauri/src/audio/rt_guard.rs` | MMCSS（`Pro Audio`） |

## 5. 本机构建（Windows）

工具链：VS 2026 Build Tools（MSVC 14.51.36231）+ Rust stable MSVC。
**ONNX Runtime 不走下载**，配置已经写进 `~/.cargo/config.toml`（不进仓库）：

```toml
[env]
ORT_LIB_LOCATION = "C:/Users/a2heng/.local/ort-win"   # onnxruntime.lib + DirectML.dll
ORT_PREFER_DYNAMIC_LINK = "1"
ORT_SKIP_DOWNLOAD = "1"

[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "link-arg=DirectML.lib"]           # pyke 包是 DirectML 版
```

构建 / 检查 / 测试：

```bash
export PATH="/c/Users/a2heng/.cargo/bin:$PATH"
cd /d/PureVox

cargo build  --manifest-path src-tauri/Cargo.toml           # 约 1~2 min
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt    --manifest-path src-tauri/Cargo.toml
cargo test   --manifest-path src-tauri/Cargo.toml --bin purevox   # 176 passed, 3 ignored
```

cargo 用国内镜像 `rsproxy`；rustup 用 USTC（`RUSTUP_DIST_SERVER`）。
只有万不得已拉 GitHub/pyke 时才临时 `HTTPS_PROXY=http://127.0.0.1:10808`，不要全局设。

## 6. 怎么验证

### 6.1 隔离实例（不要碰用户已装的 47821）

```bash
export PV_HOME="C:/Users/a2heng/AppData/Local/Temp/pv-lat"
USERPROFILE="$PV_HOME" HOME="$PV_HOME" \
  PUREVOX_DEBUG_PORT=47822 \
  PUREVOX_AUTOCALIB_MS=600000 \
  PUREVOX_MODEL_DIR="D:/PureVox/models" \
  ./src-tauri/target/debug/purevox.exe
```

读数据：

```bash
curl -s "http://127.0.0.1:47822/debug?full=0" > /tmp/d.json
curl -s "http://127.0.0.1:47822/debug/latency"
```

`pv-lat` 里已经有 4 列 `session.json`（测试音 / 麦克风 / 网络 / 麦克风+CABLE）和
`noise.wav` / `speech48.wav`。

看这几项就够判断有没有退化：

- `audio.streams[].callback_frames`：`last/min/max` 帧数、`last_gap_ms` / `max_gap_ms`
- `audio.streams[].device_buffer_ms`：**播放设备环水位**（不是设备周期）
- `audio.streams[].underrun_per_s` / `overrun_per_s` / `asrc_adjust_ppm`
- `audio.engine_info.columns[].latency`：分段预算 + `dominant`

### 6.2 两个端点测试（需要真实设备）

```bash
cargo test --manifest-path src-tauri/Cargo.toml --bin purevox wasapi_caps   -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --bin purevox wasapi_lowlat -- --ignored --nocapture
```

⚠️ `wasapi_lowlat` 里的 `ENDPOINT` 是这台机器 CABLE 的 GUID **硬编码**。换机器要改，
或者先跑 `wasapi_caps` 拿新 GUID。

## 7. 坑（都踩过，别再踩）

1. **`SetEventHandle` 必须在 `Initialize` 之后**。先设报 `AUDCLNT_E_NOT_INITIALIZED (0x88890001)`。
2. **设备环必须逐样本补水**。按 hop（480 帧）整块推，水位只在 0/480 之间跳，128 帧的回调
   必抽到 0：实测 48k 环冲到 300 ms 封顶、`in=37k`（只有 78% 实时）。
   现在用 `stage: Vec<f32>` 暂存转换器输出，按目标水位**逐帧**推。
3. **`primed` 是工作线程的预热标志，不是「流健康」标志**。我第一版在回调里欠载就把它设
   `false`，结果欠载计数直接停摆（1697 之后再不动），数据全错。
4. **cpal 0.18 的 WASAPI 后端只有共享模式**，没有 `IAudioClient3`，`BufferSize::Fixed` 也
   改不了回调节奏（源码注释：*the callback period is always `GetDevicePeriod()`*）。
   试过 `Fixed(480)`：`max` 还是 1056 帧，没用。
5. **`max=1056 帧` 不是抖动，是引擎把两次回调并成一次**（1056/48000 = 22 ms = 观测到的
   `max_gap`）。所以 480 帧周期下设备环必须 ≥ 22 ms，10 ms 必欠载——这是 P1 失败的根本原因。
6. **不加 MMCSS 会量出假的 15 ms 尾巴**。`wasapi_lowlat` 里也调了
   `rt_guard::enter_audio_thread()`，否则量的是测试线程自己的调度。
7. **设备环目标不要低于 `3 × 回调周期`**，也别低于 10 ms：实测 `max_gap` 会到 8.7 ms。
8. 独占模式在这台机器上直接不可用（见 §3.1），别把 P2 的收益押在独占上。

## 8. 下一步：P2（单 pull 图）

现在的链：`采集回调 → 采集环 → 列线程 → 行环 → 混音线程 → 混音源环 → 48k 环 → 设备环 → 输出回调`。

P2 的目标形态（**每列一条 pull 链**）：

```
输出设备回调(128 帧) → 列处理链 → 共享采集环
```

能删掉的环：行输入环、混音源环、48k 环、播放设备环（四段 10 ms），
代价是采集环要吸收输出侧的抖动（约 +5 ms）。净省 **~25 ms**，预计 **40~45 ms**。

要注意的点：

- **不是「一个回调跑所有列」**。PureVox 是 N 条独立链汇进混音，不是 DAW 的单条图；
  多列写同一个设备时仍需要一个混音（可以留在设备回调里做加法，但要多源对齐）。
- **网络列 pull 不动**（对端按 10 ms 推），那几列必须保留环。
- 列处理（降噪/TSE）必须在**一个回调周期（2.67 ms）内**跑完，否则整个设备卡顿。
  需要先量单列处理耗时；跑不完就得把「处理在哪跑」做成可配置（处理线程 + 小环）。
- 采集侧不动（Realtek 480 帧），所以 10 ms 输入回调是地板。
