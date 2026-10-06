# PureVox Lite Denoise Only — 跨时钟域播放缓冲（主线别名）
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 单一实现路径：直接复用主线 pvengine.dsp.playback.PlaybackSink，
# 不再维护 Lite 本地副本（原先与 lite_mic/playback.py 两份手动同步）。
# 历史类名 PlaybackBuffer 保留为别名，调用方无需改动。

from pvengine.dsp.playback import PlaybackSink as PlaybackBuffer

__all__ = ["PlaybackBuffer"]
