# `models/` — 内置模型（不进 GPL，见 `MODEL-LICENSE.md`）

**文件名固定、不带版本号**（2026-10-29 起）。应用代码只认这四个名字
（`infer::MODEL_DENOISE` / `infer::aec::MODEL_AEC` / `infer::tse::MODEL_TSE` /
`infer::tse::MODEL_TSE_REF`）⇒ **换模型 = 换文件**，一行代码都不用改。

| 文件 | 版本（代数 @ epoch） | 接口（每 hop 10 ms / 48 kHz） | cache |
|---|---|---|---|
| `purevox_denoise.onnx` | `purevox_denoise_202610c` @ **E88** | `spec_in (1,2,481) + cache_in` → `spec_out + cache_out` | 45,988 |
| `purevox_aec.onnx` | `purevox_aec_202610` @ **E381** | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` | 91,920 |
| `purevox_tse.onnx` | `purevox_tse_202610b` @ **E747** | `spec_in (1,2,481) + keys (1,64,143) + gb (1,64) + cache_in` → `spec_out + cache_out` | 16,072 |
| `purevox_tse_ref_encoder.onnx` | `purevox_tse_202610b` @ **E747**（与主图同批） | `ref_spec (1,2,1001,481)` → `keys + gb`（每句一次，参考 10 s） | — |

**版本号怎么读**：`model` 是 ONNX metadata 里的代数（`onnx.load('…').metadata_props`），epoch 对应
训练侧 `7_output/<代>/onnx/*_epNNNN.onnx`。本表四个 epoch 都是**按权重摘要比对训练侧 ONNX 核出来的**，
不是照抄提交说明；换模型时先改这张表，再改文件（表是部署现状的唯一记录）。

- **`cache_in` 维度随模型变**：应用侧运行时用 `cache_dim()` 从 session 输入形状读，不写死；
  但**模型池按路径缓存 session**，所以**替换文件后必须重启应用**才生效。
- **谱接口**：图内只有网络，输入/输出都是 `(1,2,481)` 复数谱；960/480 周期 Hann 分析与合成、
  OLA（逐样本除 Σw²）、1 hop 滞后都在应用侧（`src-tauri/src/infer/spec.rs`，三个模型共用一份）。
- **TSE 的两个文件必须成对**：`ref_encoder` 里是参考通路的权重，跟主图不配套时形状对不上。
  应用侧的契约**不写死名字**：取「主图输入里除 `spec_in` / `cache_in` 之外的项」当参考条件
  （现役 202610b 线是 `keys`+`gb`，202609c 线是 `enr_tok`，202611 线是 `enr_comp`），
  注册编码器必须**同名输出、形状逐个对上**，另比 metadata `model`；不配套直接报错（硬失败），
  不会退化成直通。
- ⚠️ **逐位全零**输入可能出 NaN（2026-10-31 实测 202611a：图内归一 0/0；202609c / 202610 线没这
  毛病，但保不齐下一代又踩）。应用侧对这一 hop 补零 + 清缓存并记 warn（`infer/tse.rs`）；
  根治在训练侧（归一分母加 eps）。所以拿全零谱做前向自检会被误判成坏模型 —— 喂非零谱即可。
- **替换流程**：成对文件必须同批替换；替换后重启应用。
- 每个文件里的 ONNX metadata 记着模型标识（`model=…`），可用
  `python -c "import onnx;print(onnx.load('…').metadata_props)"` 查看。

**为什么文件名不带版本号**：版本号进文件名就必然要在代码里同步改名，"换模型"就又变成
"改代码 + 重新发版"。现在的约定是文件内的 metadata 记标识、本表记部署现状，替换只动文件。
