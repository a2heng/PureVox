# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
# 
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
# 
# SPDX-License-Identifier: GPL-3.0-or-later

"""
集中管理模型文件名和 STFT 契约参数。
修改模型时只需改这个文件，所有引用处自动同步。

契约 (202609 三件套统一): 10ms hop / 波形进出 / STFT 在模型图内
- hop = SAMPLE_RATE // 100 (48kHz → 480 样本 = 10ms)，应对多采样率按 10ms 派生
- NFFT = 2 × hop (48kHz → 960)，sqrt-Hann
- 输出 enh_hop 滞后 1 hop (10ms，模型内部 tail 语义)
- TSE 的 enr_tok 由 ref_encoder.onnx 一次性预计算 (10s 注册全帧 1001 key)
"""

# ── ONNX 模型（相对应用根目录，仓库与打包产物同布局：models/）──
# 版本对应训练侧 epoch-end 试听 wav (PureVoxModel/7_output/*/results_wav)：
#   aec cpx ep0375；tse 09c ep0201
AEC_MODEL = "models/purevox_aec_202609_cpx_ep0375.onnx"
TSE_MODEL = "models/purevox_tse_202609c_ep0201.onnx"

# ── 降噪模型注册表（UI 下拉可选；key = 训练侧 202609a/b/c + 旧版 202606）──
#   目录顺序 = 小号 → 中号 → 大号 → 旧版 v6（与插件目录 plugins.CATALOG 一致）
#   a = 小号 0.28M / b = 中号 0.57M（默认）/ c = 大号 1.66M / 202606 = 旧版 v6 0.52M
#   值 = (相对路径, 下拉显示名)；引擎按 cache_in 形状自适应，四者可热切换。
#
#   ⚠️ **占位模型（placeholder）**：202609 三份 ONNX 都取自训练中期的 epoch
#      （a ep0278 / b ep0046 / c ep0012），目的只是先把小/中/大三档的代码链路
#      接进应用、验证接口与热切换。**模型权重随后续训练会滚动替换，但接口契约
#      不变**（hop = SAMPLE_RATE//100、enh_hop 滞后 1 hop、扁平 cache 形状自适应）。
#
#   202606 = 旧版 v6（v6_erb_skip_proj，0.52M）：由 lightweight-denoise-48k 的
#      checkpoint_epoch_14.tar 重新导出成 PureVox 契约——傅里叶内化在图内
#      （常量 DFT 矩阵，无 DFT/RDFT 算子）、2 进 2 出、单个扁平 cache 15176，
#      引擎零 DSP。与 202609 的唯一语义差别：分析/合成窗用普通 Hann + 常量
#      ola 包络（v6 训练时如此），而非 202609 的 sqrt-Hann。
DENOISE_MODELS = {
    "202609a": ("models/purevox_denoise_202609a_ep0278.onnx", "202609a（小号 0.28M · 更快）"),
    "202609b": ("models/purevox_denoise_202609b_ep0046.onnx", "202609b（中号 0.57M · 默认）"),
    "202609c": ("models/purevox_denoise_202609c_ep0012.onnx", "202609c（大号 1.66M · 最强）"),
    "202606": ("models/purevox_denoise_202606_ep0014_op17.onnx", "202606（旧版 v6 · 0.52M）"),
}
DENOISE_MODEL_DEFAULT = "202609b"
# 单模型路径（lite_mic/lite_net 及无选择时用；= 默认项）
DENOISE_MODEL = DENOISE_MODELS[DENOISE_MODEL_DEFAULT][0]
