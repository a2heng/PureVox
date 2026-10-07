# PureVox 设计（Tauri 3 / Rust）

本文件是 Tauri 主线（`src-tauri/`）的顶层设计：分层、数据单位与不变量、**列（Column）** 模型、
Stage 契约、SessionPlan、设备生命周期（含刷新语义）、模型生命周期、内存管理、扩展指南。
**实现与本文件冲突时以本文件为准**，改动设计先改这里再改代码。

> 旧实现（Python + Tk）冻结在 `legacy-v2026.09.30.1944/`。本文**不照搬**它：只保留骨架
> （10 ms hop / Stage 契约 / 单一实现路径 / 有界缓冲），并按下面的**列模型**重组链路。

## 0. 设计取向

- **显式优于隐式**：刷新设备、开始/停止、模型选择都由用户明确点击触发；不做后台自动重连、
  不自动改选设备、不做空闲自动卸载。
- **少状态**：流只有「运行中 / 已停止」两态；异常统一为「不可用 + 原因字符串」，无状态机、无重试。
- **列与列完全独立**：没有跨列混合，没有全局混音器。

---

## 1. 分层与目录

| 层 | 位置 | 职责 | 禁止 |
| --- | --- | --- | --- |
| L0 平台 | `src-tauri/src/audio/`（capture / playback / fanout）、`devices.rs` | 设备枚举与打开、流读写、时钟主控 | 不做 DSP |
| L1 DSP | `src-tauri/src/dsp/`（resampler / meter / ring） | 重采样、hop 切片、环形缓冲、播放时钟伺服、频谱 | 不碰设备 |
| L2 引擎 | `src-tauri/src/engine/`（stage / pipeline / registry / column / components） | 列与行、信号装配、会话生命周期 | 不做设备枚举 |
| L3 计划 | `src-tauri/src/plan.rs` | `SessionPlan`：配置 → 可启动描述；纯函数 | 不做 I/O |
| L4 推理 | `src-tauri/src/infer/` | ONNX 会话加载/卸载与流式推理 | 不碰设备 |
| L5 界面 | `src-tauri/ui/`、`src-tauri/src/main.rs` | 面板、命令、调试接口 | 不做信号处理 |

**单一实现路径**：DSP 只在 L1，播放缓冲策略只在 L1，设备枚举只在 L0，推理会话只在 L4。
新增音频功能 = 新增一个 Stage 或一种行/源，不改骨架。

---

## 2. 数据单位与不变量

- **内部格式恒为 F32 单声道 48 kHz**；原生采样率/声道只在 L0 转换（下混 + 重采样）。
- **10 ms hop 是唯一时间粒度**：`SAMPLE_RATE = 48_000`，`HOP = 480`，`NFFT = 960`，`FREQ = 481`。
  所有数据面按 hop 整数倍前进，禁止 1024/2048 等错位块。
- **一帧 = 480 样本**，链路内就地处理（`&mut [f32]`），数据面不新增分配。
- **设备回调是唯一主时钟**：回调只做「取样 / 补静音 / 写无锁环 + 原子计数」，
  **禁止在回调里写任何缓冲策略**。
- 采样率与 hop 唯一定义在 `audio/mod.rs`，其它模块一律引用。

---

## 3. 列（Column）与行（Row）

### 3.1 结构

**一个会话 = 若干列；一列 = 一条自上而下的有序节点列表；列与列完全独立（不混合）。**

```
列 1（默认，可增加更多列）
┌───────────────────────────────────────────────┐
│ 【输入】 mic（固定，不可删，下拉可换类型）        │  ← 最上面必须是输入
│   输入   loopback（可加，可拖动）                │
│   处理   denoise（可加，可拖动）                 │
│   输出   扬声器（可加，可拖动）                  │
│   处理   eq（可加，可拖动）                      │
│ 【输出】 CABLE Input（固定，不可删，下拉可换类型）│  ← 最下面必须是输出
└───────────────────────────────────────────────┘
```

- **最上面一行必须是输入、最下面一行必须是输出**，两者默认存在、**不可删除**，类型用下拉切换。
- **中间可任意增删、拖动**输入 / 输出 / 处理节点；增加的输入与输出也遵守「只能在最上面输入
  之下、最下面输出之上」。
- **列与列之间零耦合**：各自的输入、各自的处理、各自的输出。

### 3.2 一列的执行语义（自上而下累加）

```
acc = 0
for row in column.rows:          # 从最上面到最下面
    Input  → acc += 该源的一个 hop × k    # 多个输入在此混合
    Process→ acc = stage.process(acc)     # 就地处理
    Output → 把当前的 acc 送到该输出       # 位置 tap：取「到这里为止」的信号
    Viz    → 把当前的 acc 交给可视化缓冲   # 只读，不入信号路径
```

- 信号在一个 hop 内自上而下流动；**输出的位置决定它拿到哪一段信号**（放在处理之前即原始声）。
- 多个输入按 `k = 1 / max(1, 该列的输入数)`（用**配置的**输入数，避免拔插时增益跳变）混合。
- 一列至少是「输入 → 输出」（直通）。

### 3.3 列的运行方式

每列一个**列工作线程**，由系统时钟按 10 ms 驱动：

```
各输入行: 采集线程 → [L0 原生→48k, 切 hop] → 行输入环(有界)
列工作线程: 每 10ms:  从每个输入行的环各取 1 hop（空则补静音）→ 按 §3.2 执行整列
                     → 输出行把 acc 推给该输出的扇出环 → 输出设备(playback + PI 时钟伺服)
```

- **链上所有处理都在列工作线程里跑**（不在采集回调里），模型/滤波器状态天然**每列一份**。
- 每个输入行一个输入环（目标 2 hop，超 4 hop 丢最旧，空则补静音）；每个输出一个扇出环（1 s）。
- 设备与系统时钟的速率差由「输入环水位 + 输出 PI 伺服（±3%）」消化，列工作线程不需要知道任何设备时钟。
- **节拍必须绝对对齐 10 ms 网格**：`next += 10ms; sleep(next - now)`，**不能**用 `sleep(10ms - 本轮已用)`。
  后者每轮把 sleep 过冲累加，实测只产 ~96 hop/s；输出设备仍按 100 hop/s 消耗，4% 缺口超出 ±3% 伺服，
  会周期性抽干 48k 环 → 约 1 s 一次的欠载 + 重同步（听感为周期性断档）。节拍速率由列概要的
  `hop/s` 字段观测，正常必须稳定在 100。测试音等系统时钟源同理用绝对时刻节拍。

### 3.4 Stage 契约

```rust
pub struct FrameContext { pub ts: f64 }   // 当前无必需字段，固定签名

pub trait Stage: Send {
    fn name(&self) -> &'static str;
    fn accepts(&self, _ctx: &FrameContext) -> bool { true }
    fn process(&mut self, frame: &mut [f32], ctx: &FrameContext) -> Result<(), StageError>;
    fn reset(&mut self) {}
    fn release(&mut self) {}
}
pub enum StageError { Unavailable(String), Fatal(String) }
```

- 进出一律 480；组件不做采样率/声道转换、不切 hop。
- `Fatal` → 该列标记不可用并显示原因；`Unavailable` → 本帧跳过（直通）。
- 组件实例属于**行**（每列每行一份），因此流式状态按行隔离。

### 3.5 注册表

```rust
pub enum NodeKind { Input, Process, Output, Viz }
pub struct NodeSpec { pub ptype: &'static str, pub label: &'static str,
                      pub kind: NodeKind, pub params: &'static [ParamSpec] }
pub fn all_specs() -> &'static [NodeSpec];                       // 下拉与计划唯一来源
pub fn get_spec(ptype: &str) -> Option<&'static NodeSpec>;
pub fn create_stage(ptype: &str, params: &Params) -> Result<Box<dyn Stage>, String>;  // 仅 Process
```

- 输入 / 输出节点不是 Stage（它们是源与汇），由 L0 实现，注册表里只登记它们的类型与参数。

### 3.6 可视化

- viz 是**列内的一种行**，取它所在位置的信号（只读）。
- 缓冲有界：`cap = 48_000 * 5`（5 s），满时丢最旧。**禁止未上界的可视化缓冲**。

---

## 4. 节点清单

| 类别 | ptype（示例） | 说明 |
| --- | --- | --- |
| 输入 | `audio_input` / `loopback` / `remote_mic` / `media` / `echo_cancel` | 源；同列多个混合 |
| 处理 | `denoise_*` / `eq` / `gain` / `agc` / `compressor` | Stage；就地处理 |
| 输出 | `audio_output` | 汇；位置 tap，送往设备 |
| 可视化 | `vu_meter` / `spectrum` | 只读 tap |

- **AEC 行**：一个输入行 =「一路 mic + 一路 far」，mic 在行内先过 AEC 再进列；
  far 参考只给模型用，**不进信号路径**；历史不足则直通 mic。
- **媒体行**（音板 / 音乐 / 系统声）：无采集设备，按需生成 hop 的输入行。
- **网络行**：`remote_mic`，Opus 帧即 480，只允许一个。

---

## 5. SessionPlan（L3）

```rust
pub struct SessionPlan { pub columns: Vec<ColumnSpec>, pub problems: Vec<String>, pub warnings: Vec<String> }
pub struct ColumnSpec { pub rows: Vec<RowSpec> }        // rows[0] 必为 Input，rows[last] 必为 Output
pub struct RowSpec {
    pub kind: NodeKind,
    pub ptype: String,
    pub enabled: bool,
    pub params: Params,
    pub device: Option<DeviceRef>,   // 输入/输出行
    pub fixed: bool,                 // 两端的默认输入/输出 = true（不可删）
}
```

- `SessionPlan::from_config(cfg)`：**纯函数**，只读注册表与配置，不做 I/O。
- 校验：每列首行必须是输入、末行必须是输出（否则 `problems`）；网络行仅一个；无输出等。
- **变更语义**：结构性变更（增删/排序行、开关节点、换设备、换型号、增删列）→ **整会话重启**；
  热参数（增益、EQ 频点、阈值）→ 下发给运行中的实例，不重启。

---

## 6. 设备生命周期

### 6.1 枚举与刷新（唯一入口）

- 枚举只用 cpal（`devices.rs`），**唯一入口** `devices::spawn_refresh`，后台线程执行。
- 触发点只有两个：**程序启动**、**面板上显眼的「刷新设备」按钮**。

### 6.2 标识与持久化

- **标识用 cpal 稳定 `DeviceId`**（不用名字；旧实现按名字模糊匹配是有意修正掉的坑）。
- 配置键按接口后缀写全：`input_device_<host>` / `output_device_<host>` / `aec_far_sink_<host>`，
  全部接口显式写全（占位保留）。
- 设备不存在时条目**保留**并标记不可用（附原因），**不静默替换**。

### 6.3 刷新对运行中链路的影响

**刷新只更新列表，不动正在跑的流，不重建链路。**

- 流按设备 ID 绑定：设备还在 → 不受影响；设备不在 → 该行标「设备不在（+原因）」并停止产出
  （回调停顿 ≥ 1 s 判定），界面给「重新打开」；**只标记、不动流**，不自动换设备、不自动重连。
- 只有**改设备选择**（结构性变更）才重启会话。

### 6.4 流的加载 / 卸载

- **两态 + 一个原因**：运行中 / 已停止 + 不可用原因；无状态机、无重试计数。
- 停止即停线程、关流、释放该行对模型的引用。
- 打开/关闭设备不在 UI 线程（异步命令 + 阻塞线程池）。

---

## 7. 模型生命周期

```rust
pub enum ModelKind { Denoise, Tse, Aec, TseRefEncoder }
pub struct ModelDesc { pub key: &'static str, pub file: &'static str, pub label: &'static str, pub kind: ModelKind }
pub fn models() -> &'static [ModelDesc];
pub fn resolve(key: &str, kind: ModelKind) -> Option<&'static ModelDesc>;
```

- 现存：`purevox_denoise_202609c_ep0012.onnx`（现役）、`202609b` / `202609a` / `202606`、
  `purevox_aec_202609_cpx_ep0375.onnx`、`purevox_tse_202609c_ep0201.onnx`（+ `..._ref_encoder.onnx`）。
  **缓存维度从模型输入读，不写死**。
- 选择存配置（`denoise_model` 等），计划构建时校验文件存在，缺失进 `problems`。
- 会话配置：`with_intra_threads(1)` + `with_inter_threads(1)` + `with_intra_op_spinning(false)`
  （实时音频必须；默认多线程忙等会占满 CPU 并把推理拖慢 50 倍）。
- **权重按文件共享一份**（`Arc<Session>`），**`cache` 每行一份**；惰性加载；行/会话停止即释放引用。
- **不做空闲自动卸载**；**不热切换模型文件**（改型号 = 结构性变更 → 重启）。

---

## 8. 内存管理

| 缓冲 | 位置 | 容量 / 策略 |
| --- | --- | --- |
| 采集回调 → 工作线程环 | `audio::capture`（rtrb） | 原生 500 ms；满则丢样并计数 |
| 行输入环 | `engine` | 目标 2 hop，超 4 hop 丢最旧；空补静音 |
| 输出扇出环 | `audio::fanout`（rtrb） | 1 s/订阅者；满则丢最旧 |
| 源缓冲（输出侧） | `audio::playback` | 目标 40 ms，封顶 300 ms，超限丢最旧 |
| 设备侧环 | `audio::playback` | 目标 30 ms |
| 重采样状态 | `dsp::resampler`（rubato） | 固定（数帧） |
| viz 缓冲 | `engine` | 5 s，丢最旧 |
| 频谱 / 波形 | `dsp::meter` | 波形 50 ms、频谱 481 bin，固定 |
| AEC far 历史 | `engine` | 2 s 网格；`far_delay` ≤ 1000 ms |
| 回环历史 | `engine` | 1.5 s |
| 网络 acc | 未来 | 目标 50 ms，硬顶 80 ms |
| 模型 `cache` | `infer` | 由模型决定（降噪 36506 / AEC 215504 / TSE 513216 f32），**每行一份** |

规则：**音频回调零分配零加锁零日志**；工作线程预分配复用；任何进入数据面的容器都必须有上限与丢弃策略。

---

## 9. 扩展指南

- **新增处理组件**：`engine/components/` 新建实现 `Stage` → 注册表登记 `NodeSpec{kind: Process}` →
  冒烟测试 → 用户可感知的变更追加 `about/changelog.md`。
- **新增输入/输出**：注册表登记 + 实现 L0 端点 + `SessionPlan` 提取规则 + 单测。
- 必须遵守：先扩展再新建；被替代实现直接删除、不留平行；DSP 只在 L1；不改骨架。

---

## 10. 现状对照与实施顺序

| 能力 | 现状 | 目标 |
| --- | --- | --- |
| 设备枚举（cpal / 稳定 ID）+ 刷新按钮 | ✅ 已实现（顶栏「刷新设备」，只更新列表、不动流） | §6 |
| 采集 → 重采样 → 48k hop | ✅ 已实现 | 作为「输入行」的源 |
| 输出 + 时钟伺服 | ✅ 已实现 | 作为「输出行」的汇 |
| 降噪 ONNX | ✅ 已实现（1.8 ms/hop） | 包成 `Stage`，放进列 |
| 调试面板 + HTTP 调试接口 | ✅ 已实现（含 `ui` 前端自报、`column` 列概要） | 补行状态、模型字段 |
| **Stage / Pipeline / 注册表** | ✅ 已实现 | §3.4 / 3.5 |
| **列工作线程 + 行环** | ✅ 已实现（多列独立、自上而下位置语义） | §3.3 |
| **配置 + SessionPlan** | ◑ 列/行/设备/型号已接入并持久化；热参数下发未做 | §5 |
| **列 UI（增删列、加/拖行、两端固定）** | ✅ 已实现（增删列、加/删/上下移行、两端固定；拖动待做） | §3.1 |
| **模型注册表 + 型号选择** | ◑ 降噪/TSE 型号下拉已接入；统一 `ModelDesc` 注册表待做 | §7 |
| **TSE 目标说话人提取** | ✅ 已实现（参考 10 s → `enr_tok`，3.9 ms/hop；参考可录制=降噪后+音量归一化；无参考直通） | §4 / §7 |
| **AEC 回声消除** | ✅ 已实现（far 参考采集 + 2 s 网格对齐 + 每行缓存 + 互相关延时校准） | §4 |
| 回环 / 媒体 / 网络 | ⛔ | §4 |

**实施顺序**：
1. ✅ Stage / Pipeline / 注册表骨架 + 现有降噪包成 Stage。
2. ✅ 列工作线程 + 行输入环（源 → 列 → 输出；多列独立）。
3. ◑ 配置 + SessionPlan（计划/校验/持久化/重建已接入；热参数待做）。
4. ◑ 列 UI（两端固定、中间加/删/移、增删列已接入；拖动排序待做）。
5. 刷新按钮 + 行状态展示（含「重新打开」）。
6. ◑ 模型注册表 + 型号选择（降噪/TSE/AEC 已接入）。
7. ✅ TSE、✅ AEC（含参考录制、延时校准）；再回环 → 媒体 / 网络。
