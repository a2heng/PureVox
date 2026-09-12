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
#   denoise ep0000；aec cpx ep0375；tse 09c ep0201
DENOISE_MODEL = "models/purevox_denoise_202609_ep0000.onnx"
AEC_MODEL = "models/purevox_aec_202609_cpx_ep0375.onnx"
TSE_MODEL = "models/purevox_tse_202609c_ep0201.onnx"
TSE_REF_ENCODER = "models/purevox_tse_202609c_ref_encoder.onnx"