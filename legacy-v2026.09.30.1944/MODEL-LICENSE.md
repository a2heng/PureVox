# PureVox 内置 AI 模型授权声明

**Copyright (C) 2024-2026 a2heng <752848283@qq.com>**

PureVox 的源代码采用 **GNU General Public License v3.0 or later (GPL-3.0)** 开源，
但**内置 AI 模型及其权重不随 GPL 授权**。

## 一、适用模型

**现行版本：**

- `purevox_denoise_202609_*.onnx`（降噪模型）
- `purevox_aec_202609_cpx_*.onnx`（AEC 回声消除模型）
- `purevox_tse_202609c_*.onnx`（TSE 目标说话人提取模型）
- `purevox_tse_202609c_ref_encoder.onnx`（TSE 参考音频编码器）

**历史版本**（已随历史发布分发、现已从仓库移除，同样适用本声明）：

- `v9_fft2048_band256_epoch_261.onnx`（降噪模型）
- `aec9_ep0544.onnx`（AEC 回声消除模型）
- `tse15_stream_ep_0673.onnx`（TSE 目标说话人提取模型）
- 及上述各模型的更早迭代版本

以上模型归 a2heng 所有，是本项目的更新版本，**不适用于**作者的 MIT 开源模型仓库：

- <https://github.com/a2heng/lightweight-denoise-48k>（MIT，可自由使用）
- <https://github.com/a2heng/lightweight-aec-48k>（MIT，可自由使用）

## 二、授权范围

在未获得作者另行书面授权的情况下：

- 允许：随 PureVox 正常安装、运行、使用；
- **禁止：将模型或其权重提取出来，用于其它任何项目 / 软件 / 训练 / 再分发**；
- 禁止：以任何形式将模型用于闭源或开源商业产品的核心功能；
- 禁止：移除、隐匿本声明及模型的版权信息。

## 三、申请授权

如需将模型用于其它用途（包括但不限于：集成到其它项目、商业产品、模型蒸馏与训练），
请联系作者获取授权：

- GitHub: <https://a2heng.github.io/>
- 哔哩哔哩: <https://space.bilibili.com/10850943>
- 邮箱: <752848283@qq.com>

## 四、免责声明

除适用法律强制规定外，PureVox 及内置模型按"原样"提供，不附带任何明示或默示保证。
任何因使用本软件或模型而产生的直接或间接损失，作者均不承担责任（除非另行书面约定）。

---

最后更新: 2026-09-01
