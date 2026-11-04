# `models/` — 内置模型（不进 GPL，见 `MODEL-LICENSE.md`）

**文件名固定、不带版本号**（2026-10-29 起）。应用代码只认这四个名字
（`infer::MODEL_DENOISE` / `infer::aec::MODEL_AEC` / `infer::tse::MODEL_TSE` /
`infer::tse::MODEL_TSE_REF`）⇒ **换模型 = 换文件**，一行代码都不用改。

| 文件 | 版本（代数 @ epoch） | 接口（每 hop 10 ms / 48 kHz） | cache |
|---|---|---|---|
| `purevox_denoise.onnx` | `purevox_denoise_202611d` @ **E27** | `spec_in (1,2,481) + cache_in` → `spec_out + cache_out` | 25,140 |
| `purevox_aec.onnx` | `purevox_aec_202610` @ **E381** | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` | 91,920 |
| `purevox_tse.onnx` | `purevox_tse_202609c` @ **E205** | `mix_spec (1,2,481) + enr_tok (1,2,1001,180) + cache_in` → `spec_out + cache_out` | 513,216 |
| `purevox_tse_ref_encoder.onnx` | `purevox_tse_202609c` @ **E205**（与主图同批） | `ref_spec (1,2,1001,481)` → `enr_tok (1,2,1001,180)`（每句一次，参考 10 s） | — |

**版本号怎么读**：`model` 是 ONNX metadata 里的代数（`onnx.load('…').metadata_props`），epoch 对应
训练侧 `7_output/<代>/checkpoints/checkpoint_epoch_NNN.tar`。降噪直接取训练侧 `onnx/` 的导出
（本代是 `denoise_202611d_0002` 的 `ep0027`）；AEC 的 epoch 按权重摘要比过训练侧
ONNX；TSE 是 2026-11-04 从 E205 的 checkpoint 现导出的（同一套训练脚本的 `export_onnx`）。
换模型时先改这张表，再改文件（表是部署现状的唯一记录）。

- **`cache_in` 维度随模型变**：应用侧运行时用 `cache_dim()` 从 session 输入形状读，不写死；
  但**模型池按路径缓存 session**，所以**替换文件后必须重启应用**才生效。
- **谱接口**：图内只有网络，输入/输出都是 `(1,2,481)` 复数谱；960/480 周期 Hann 分析与合成、
  OLA（逐样本除 Σw²）、1 hop 滞后都在应用侧（`src-tauri/src/infer/spec.rs`，三个模型共用一份）。
- **TSE 的两个文件必须成对**：`ref_encoder` 里是参考通路的权重，跟主图不配套时形状对不上。
  应用侧的契约**不写死名字**：取「主图输入里除 `spec_in`/`mix_spec` / `cache_in` 之外的项」当参考条件
  （现役 202609c 线是 `enr_tok`，202610 线是 `keys`+`gb`，202611 线是 `enr_comp`），
  注册编码器必须**同名输出、形状逐个对上**，另比 metadata `model`；不配套直接报错（硬失败），
  不会退化成直通。
- **降噪现役是 `202611d@E27`（2026-11-04 换）**：直接取训练侧
  `7_output/denoise_202611d_0002/onnx/purevox_denoise_202611d_ep0027.onnx`（`--out-seq 2` 那轮的
  导出产物，**该轮训练还在跑**，换最终 epoch 只换文件）。这一代把 cache 从 45,988 缩到 **25,140**、
  文件 **7.50 MB → 0.89 MB**；实测单 hop（机上 1 线程 CPU）**0.88 → 0.31 ms**，
  val 集 SI-SDR **+1.37 dB**（202610c@E88 同口径 +1.51 dB，属早期轮次，别当回退看）。
  同目录 `*.onnx.data` 是导出残留（该 `.onnx` 自包含），**只拷 `.onnx`**。
- **TSE 现役是 `202609c@E205`（2026-11-04 更新）**：从训练侧
  `7_output/output_purevox_tse_202609c/checkpoints/checkpoint_epoch_205.tar` 用
  `legacy/202609/purevox_tse_202609c.py` 的 `export_onnx` / `export_ref_encoder` 导出
  （`PureVoxTse202609CStreamSpec`：**傅里叶在图外**，图内只有网络）。
  480 hop / 960 窗与引擎 10 ms 网格一致；参考 1001 帧 = 10 s；`enr_tok` 每 hop 原样重喂。
  实测（`6_epoch_end_test_wav/tse`，mix→目标 −4.15 dB）：输出→目标 **+5.34 dB（Δ +9.5 dB）**。
- ⚠️ **逐位全零**输入可能出 NaN（2026-10-31 实测 202611a：图内归一 0/0；202609c / 202610 线没这
  毛病，但保不齐下一代又踩）。应用侧对这一 hop 补零 + 清缓存并记 warn（`infer/tse.rs`）；
  根治在训练侧（归一分母加 eps）。所以拿全零谱做前向自检会被误判成坏模型 —— 喂非零谱即可。
- **替换流程**：成对文件必须同批替换；替换后重启应用。
- 每个文件里的 ONNX metadata 记着模型标识（`model=…`），可用
  `python -c "import onnx;print(onnx.load('…').metadata_props)"` 查看。

**为什么文件名不带版本号**：版本号进文件名就必然要在代码里同步改名，"换模型"就又变成
"改代码 + 重新发版"。现在的约定是文件内的 metadata 记标识、本表记部署现状，替换只动文件。
