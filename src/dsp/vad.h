// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_VAD_H
#define PUREVOX_DSP_VAD_H

#include <cstddef>

namespace pv {

// 语音活性检测（VAD）：RMS 阈值 + 起音/挂音帧计数。
// process 会在不活跃时把缓冲静音。
class Vad {
public:
    Vad(float threshold_dbfs, float onset_ms, float hang_ms, float fs, int hop);
    void reset();
    int process(float *samples, size_t n);
    bool isActive() const { return active_; }
    void setThreshold(float dbfs);
    float thresholdDbfs() const;

private:
    float thresholdLinear_;
    int onsetFrames_;
    int hangFrames_;
    bool active_ = false;
    int voiceCnt_ = 0;
    int silenceCnt_ = 0;
};

}  // namespace pv

#endif // PUREVOX_DSP_VAD_H
