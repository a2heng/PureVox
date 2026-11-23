# `models/` — 内置模型

**降噪四款 = BSD-3-Clause**（与推理库 `purevox_denoise_rust` 同仓、同许可）；
**AEC / TSE = 保留所有权利**。完整声明见 `MODEL-LICENSE.md`。

## 降噪（纯 Rust 推理，`models/<模型名>/`）

降噪不再是单个 `.onnx` 文件，而是**一个目录**（`meta.json` 记结构与 cache 布局，
`weights.safetensors` 是权重），由 `purevox_denoise_rust` 在进程内直接推理。
目录名就是模型名，界面下拉一行一个、直接显示完整目录名：

| 目录（= 模型名） | 结构 | cache（f32） |
|---|---|---|
| `purevox_denoise_202605a/` | 组卷积编码器 + 时/频注意力（60+35 ERB 带） | 10,208 |
| `purevox_denoise_202605b/` | 同族，前端更宽（100+41 带） | 14,024 |
| `purevox_denoise_202611d/` | h-swish 块 + FIR 门 + 2 层双路径 GRU + 瓶颈记忆（160+80 带） | 25,588 |
| `purevox_denoise_202611f/` | 同 d，更宽通道 + 3 层双路径 GRU | 32,308 |

- 接口：每 hop 10 ms / 48 kHz，谱进谱出（`spec_in (2,481)` + `cache_in` → `spec_out` + `cache_out`）；
  FFT / OLA 在应用侧（`infer::spec`）。
- **换模型 = 换目录**，代码零改动；新增一款就放一个新目录并在
  `infer::denoise_models()` 里加一行。
- 四款都是 48 kHz、实数幅度掩码（保留含噪相位）、无运行期旋钮。

## 其它模型（ONNX，保留所有权利）

| 文件 | 用途 | 接口 |
|---|---|---|
| `purevox_aec.onnx` | 回声消除 | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` |
| `purevox_tse.onnx` | 目标说话人提取 | `mix_spec + enr_tok + cache_in` → `spec_out + cache_out` |
| `purevox_tse_ref_encoder.onnx` | TSE 注册编码器 | `ref_spec` → `enr_tok`（每句一次） |
