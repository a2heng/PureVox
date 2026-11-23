# PureVox 内置模型授权声明

**Copyright (C) 2024-2026 a2heng <752848283@qq.com>**

PureVox 的源代码采用 **GNU General Public License v3.0 or later (GPL-3.0)** 开源。
内置模型分两类：

1. **降噪模型（四款）—— BSD-3-Clause**：与开源推理库
   [`purevox_denoise_rust`](https://github.com/a2heng/purevox_denoise_rust) 同仓分发，
   允许任意使用、修改、再分发（保留版权声明与免责声明即可）。
2. **AEC / TSE 模型 —— 保留所有权利**：不随 GPL 授权，只能随 PureVox 使用。

## 一、降噪模型（BSD-3-Clause）

应用用的是下面**四款**，目录名即模型名（`models/<名字>/`，内含
`meta.json + weights.safetensors`），由 `purevox_denoise_rust` 在进程内直接推理
（不经 ONNX Runtime）：

| 模型名 | 说明 |
|---|---|
| `purevox_denoise_202605a` | 组卷积编码器 + 时/频注意力（60+35 ERB 带） |
| `purevox_denoise_202605b` | 同族，前端更宽（100+41 ERB 带） |
| `purevox_denoise_202611d` | h-swish 块 + FIR 门 + 2 层双路径 GRU + 瓶颈记忆（160+80 带） |
| `purevox_denoise_202611f` | 同 d，更宽通道 + 3 层双路径 GRU |

授权：**BSD 3-Clause**（与库代码相同；见 `purevox_denoise_rust` 仓库的 `LICENSE`）。
四个模型都在同一仓库里以 `models/<名字>/` 分发，版权归 a2heng。

## 二、AEC / TSE 模型（保留所有权利）

以下模型**不在** BSD/GPL 授权范围内，仅允许随 PureVox 一起使用，不得单独提取、
再分发或用于其它产品：

- `purevox_aec.onnx`（回声消除）
- `purevox_tse.onnx`（目标说话人提取）
- `purevox_tse_ref_encoder.onnx`（TSE 注册编码器，与 `purevox_tse.onnx` 成对）

## 三、历史版本

早期随发布分发过的降噪/回声消除/目标说话人模型（带 `_epoch_`、`_5a/_5b`、
`medium/large` 等旧文件名）同样适用本节声明：**降噪旧版按 BSD-3-Clause 看待**
（与对应的 `purevox_denoise_2026xx` 模型同一批权重），AEC/TSE 旧版保留所有权利。

## 四、第三方组件

`purevox_denoise_rust` 及其依赖（`realfft` / `rustfft` / `safetensors` / `serde` /
`hound`）的许可见 `LICENSE-THIRD-PARTY.txt`。
