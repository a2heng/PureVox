# `models/` — 内置模型（不进 GPL，见 `MODEL-LICENSE.md`）

**文件名固定、不带版本号/epoch**（2026-10-29 起）。应用代码只认这四个名字
（`infer::MODEL_DENOISE` / `infer::aec::MODEL_AEC` / `infer::tse::MODEL_TSE` /
`infer::tse::MODEL_TSE_REF`）⇒ **换模型 = 换文件**，一行代码都不用改。

| 文件 | 当前权重 | 接口（每 hop 10 ms / 48 kHz） | cache |
|---|---|---|---|
| `purevox_denoise.onnx` | 降噪 **202610a** ep0038（占位，训练中） | `spec_in (1,2,481) + cache_in` → `spec_out + cache_out` | 15,668 |
| `purevox_aec.onnx` | AEC **202610** ep0381 | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` | 91,920 |
| `purevox_tse.onnx` | TSE **202610a** ep0066（占位，训练中） | `spec_in + keys (1,64,143) + gb (1,64) + cache_in` → `spec_out + cache_out` | 16,072 |
| `purevox_tse_ref_encoder.onnx` | 与上一行**同一 epoch**（权重同源） | `ref_spec (1,2,1001,481)` → `keys + gb`（每句一次） | — |

- **谱接口**：图内只有网络，输入/输出都是 `(1,2,481)` 复数谱；960/480 周期 Hann 分析与合成、
  OLA（逐样本除 Σw²）、1 hop 滞后都在应用侧（`src-tauri/src/infer/spec.rs`，三个模型共用一份）。
- **TSE 的两个文件必须同源**：`ref_encoder` 里是键投影与 FiLM 的权重，跟主图不配套时
  keys/gb 形状或数值对不上（应用侧加载时会明确报错）。
- 每个文件里的 ONNX metadata 记着来源版本（`model=purevox_denoise_202610a` 等），
  想确认当前权重是哪一代可以 `python -c "import onnx;print(onnx.load('…').metadata_props)"`。
- 发布/替换流程在训练仓库 `PureVoxModel/8_purevox_mic_net/publish_to_app.py`
  （内联外部数据 + 用 onnxruntime 真跑一遍 + 拷成上面的固定文件名）。

**为什么文件名不带版本号**：版本号进文件名就必然要在代码里同步改名，"换模型"就又变成
"改代码 + 重新发版"。现在的约定是文件内的 metadata 记版本、本表记部署现状，替换只动文件。
