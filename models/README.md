# `models/` — 内置模型（不进 GPL，见 `MODEL-LICENSE.md`）

> ## 本次发布（2026-10-30）说明
>
> 这一版换上的是**下一步将要使用**的新一代模型（降噪 `202610c`、TSE `202610e`），
> **两者都还在训练中**，后续效果可能继续提升；
> **但截至本次发版，实测效果已经打平历史版本**（`purevox_denoise_202610a` / `purevox_tse_202610a`）。
>
> **发版的主要目的**是验证**新的 CPU 占用**与**真实场景效果**：
> 这两代是按"单核占用预算"重新定档的（降噪目标 ≈10% 单核、TSE 目标 ≈8% 单核，
> 均为 960/480 周期 Hann + 图外 FFT 的完整流式回路，单线程实测）。
> **欢迎反馈**：卡顿/爆音/残留噪声/人声发闷都请带上机型与场景描述。

**文件名固定、不带版本号/epoch**（2026-10-29 起）。应用代码只认这四个名字
（`infer::MODEL_DENOISE` / `infer::aec::MODEL_AEC` / `infer::tse::MODEL_TSE` /
`infer::tse::MODEL_TSE_REF`）⇒ **换模型 = 换文件**，一行代码都不用改。

| 文件 | 当前权重 | 接口（每 hop 10 ms / 48 kHz） | cache |
|---|---|---|---|
| `purevox_denoise.onnx` | 降噪 **202610c** ep0042（训练中） | `spec_in (1,2,481) + cache_in` → `spec_out + cache_out` | **45,988** |
| `purevox_aec.onnx` | AEC **202610** ep0381 | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` | 91,920 |
| `purevox_tse.onnx` | TSE **202610e** ep0053（训练中） | `spec_in + keys (1,64,143) + gb (1,**144**) + cache_in` → `spec_out + cache_out` | **34,568** |
| `purevox_tse_ref_encoder.onnx` | 与上一行**同一 epoch**（权重同源） | `ref_spec (1,2,1001,481)` → `keys + gb`（每句一次） | — |

- ⚠ **`cache_in` 维度随模型变**（本次：降噪 15,668 → **45,988**、TSE 16,072 → **34,568**、
  TSE 的 `gb` 64 → **144**）。应用侧是运行时用 `cache_dim()` 从 session 输入形状读的，
  不写死维度；但**模型池按路径缓存 session**，所以**替换文件后必须重启应用**才生效。
- 本次两代模型的算力（单线程 ONNX，含图外 FFT/OLA，100 s 音频实测）：
  降噪 c ≈ **1.00 ms/hop（单核 10%）**、TSE e ≈ **0.76 ms/hop（单核 7.6%）**，
  AEC 仍是 ≈ 1.19 ms/hop（单核 12%）。

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
