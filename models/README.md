# `models/` — 内置模型（不进 GPL，见 `MODEL-LICENSE.md`）

**文件名固定、不带版本号**（2026-10-29 起）。应用代码只认这些名字
（`infer::MODEL_DENOISE` / `infer::MODEL_DENOISE_M` / `infer::MODEL_DENOISE_L` /
`infer::aec::MODEL_AEC` / `infer::tse::MODEL_TSE` / `infer::tse::MODEL_TSE_REF`）⇒
**换模型 = 换文件**，一行代码都不用改。
降噪有**三个尺寸档**（界面下拉「降噪（小）/（中）/（大）」），只是容量不同，接口完全一致。

| 文件 | 版本（代数 @ epoch） | 接口（每 hop 10 ms / 48 kHz） | cache |
|---|---|---|---|
| `purevox_denoise.onnx`（小） | `purevox_denoise_202611d` @ **E27** | `spec_in (1,2,481) + cache_in` → `spec_out + cache_out` | 25,140 |
| `purevox_denoise_medium.onnx`（中） | `purevox_denoise_202611e` @ **E87** | 同上（同一套谱接口） | 22,964 |
| `purevox_denoise_large.onnx`（大） | `purevox_denoise_202611f` @ **E8** | 同上（同一套谱接口） | 31,860 |
| `purevox_aec.onnx` | `purevox_aec_202610` @ **E381** | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` | 91,920 |
| `purevox_tse.onnx` | `purevox_tse_202609c` @ **E205** | `mix_spec (1,2,481) + enr_tok (1,2,1001,180) + cache_in` → `spec_out + cache_out` | 513,216 |
| `purevox_tse_ref_encoder.onnx` | `purevox_tse_202609c` @ **E205**（与主图同批） | `ref_spec (1,2,1001,481)` → `enr_tok (1,2,1001,180)`（每句一次，参考 10 s） | — |

**版本号怎么读**：`model` 是 ONNX metadata 里的代数（`onnx.load('…').metadata_props`），epoch 对应
训练侧 `7_output/<代>/checkpoints/checkpoint_epoch_NNN.tar`。降噪直接取训练侧 `onnx/` 的导出
（小档 `denoise_202611d_0002/ep0027`，中档 `denoise_202611e_0010/ep0087`，
大档 `denoise_202611f_0001/ep0008`）；AEC 的 epoch 按权重
摘要比过训练侧 ONNX；TSE 是 2026-11-04 从 E205 的 checkpoint 现导出的（同一套训练脚本的 `export_onnx`）。
换模型时先改这张表，再改文件（表是部署现状的唯一记录）。

- **`cache_in` 维度随模型变**：应用侧运行时用 `cache_dim()` 从 session 输入形状读，不写死；
  但**模型池按路径缓存 session**，所以**替换文件后必须重启应用**才生效。
- **谱接口**：图内只有网络，输入/输出都是 `(1,2,481)` 复数谱；960/480 周期 Hann 分析与合成、
  OLA（逐样本除 Σw²）、1 hop 滞后都在应用侧（`src-tauri/src/infer/spec.rs`，三类模型共用一份）。
- **TSE 的两个文件必须成对**：`ref_encoder` 里是参考通路的权重，跟主图不配套时形状对不上。
  应用侧的契约**不写死名字**：取「主图输入里除 `spec_in`/`mix_spec` / `cache_in` 之外的项」当参考条件
  （202609c 线是 `enr_tok`，202610 线是 `keys`+`gb`，202611 线是 `enr_comp`），
  注册编码器必须**同名输出、形状逐个对上**，另比 metadata `model`；不配套直接报错（硬失败），
  不会退化成直通。
- **降噪小档是 `202611d@E27`（2026-11-04 换）**：直接取训练侧
  `7_output/denoise_202611d_0002/onnx/purevox_denoise_202611d_ep0027.onnx`（`--out-seq 2` 那轮的
  导出产物，**该轮训练还在跑**，换最终 epoch 只换文件）。这一代把 cache 从 45,988 缩到 **25,140**、
  文件 **7.50 MB → 0.89 MB**；实测单 hop（机上 1 线程 CPU，`ort_bench`）**0.88 → 0.28 ms**，
  val 集 SI-SDR **+1.37 dB**（202610c@E88 同口径 +1.51 dB，属早期轮次，别当回退看）。
  同目录 `*.onnx.data` 是导出残留（该 `.onnx` 自包含），**只拷 `.onnx`**。
- **降噪中档是 `202611e@E87`（2026-11-05 加）**：取训练侧
  `7_output/denoise_202611e_0010/onnx/purevox_denoise_202611e_ep0087.onnx`（`ep0087`，该轮仍在跑）。
  202611e 是 202611d 的**宽通道版**（CH 内三层 48/64/80、瓶颈 DPGRNN 64；参数 0.081M → 0.127M，
  MAC 2.13M → 3.37M），谱接口与 cache 形状不变，应用侧不用改代码：
  文件 **0.89 MB → 1.00 MB**，单 hop（机上 1 线程 CPU）**0.28 → 0.30 ms**。
  ⚠️ DNS5 val 上 SI-SDR **+0.38 dB**（小档同口径 +1.37 dB），是训练中版本、未经听感筛选 ——
  界面给的是**选项**，默认仍走小档。
  同目录 `*.onnx.data` 同样是导出残留，**只拷 `.onnx`**。
- **降噪大档是 `202611f@E8`（2026-11-05 加）**：取训练侧
  `7_output/denoise_202611f_0001/onnx/purevox_denoise_202611f_ep0008.onnx`（`ep0008`，该轮刚起步）。
  202611f 是**新骨架**（CH `[24,48,96,96,64,48]`，通道峰 96 占两级；cTFA 时间门用逐通道 C·5），
  谱接口不变、cache 略大（**31,860**）：文件 **1.10 MB**、参数 16.7 万，
  单 hop（机上 1 线程 CPU，`ort_bench`）**0.51 ms**（p95 0.99）、RTF 0.051。
  ⚠️ DNS5 val 上 SI-SDR **+0.54 dB**（小档 +1.37 / 中档 +0.38），**只练到 E8**、远未收敛，
  界面给的是选项，默认仍走小档。
  同目录 `*.onnx.data` 同样是导出残留，**只拷 `.onnx`**。
- **TSE 是 `202609c@E205`（2026-11-04 更新）**：从训练侧
  `7_output/output_purevox_tse_202609c/checkpoints/checkpoint_epoch_205.tar` 用
  `legacy/202609/purevox_tse_202609c.py` 的 `export_onnx` / `export_ref_encoder` 导出
  （`PureVoxTse202609CStreamSpec`：**傅里叶在图外**，图内只有网络）。
  480 hop / 960 窗与引擎 10 ms 网格一致；参考 1001 帧 = 10 s；`enr_tok` 每 hop 原样重喂。
  实测（`6_epoch_end_test_wav/tse`，mix→目标 −4.15 dB）：输出→目标 **+5.34 dB（Δ +9.5 dB）**。
- ⚠️ **逐位全零**输入可能出 NaN（2026-10-31 实测 202611a：图内归一 0/0；202609c / 202610 线没这
  毛病，但保不齐下一代又踩）。应用侧对这一 hop 补零 + 清缓存并记 warn（`infer/tse.rs`）；
  根治在训练侧（归一分母加 eps）。所以拿全零谱做前向自检会被误判成坏模型 —— 喂非零谱即可。
- **替换流程**：TSE 成对文件必须同批替换；降噪三档各自独立（换哪个档只动哪个文件）。替换后重启应用。
- 每个文件里的 ONNX metadata 记着模型标识（`model=…`），可用
  `python -c "import onnx;print(onnx.load('…').metadata_props)"` 查看。

**为什么文件名不带版本号**：版本号进文件名就必然要在代码里同步改名，"换模型"就又变成
"改代码 + 重新发版"。现在的约定是文件内的 metadata 记标识、本表记部署现状，替换只动文件。
