// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "vad.h"

#include <cmath>
#include <cstdlib>

namespace pv {

Vad::Vad(float threshold_dbfs, float onset_ms, float hang_ms, float fs, int hop) {
    thresholdLinear_ = std::pow(10.0f, threshold_dbfs / 20.0f);
    onsetFrames_ = (int)(onset_ms / 1000.0f * fs / hop);
    if (onsetFrames_ < 1) onsetFrames_ = 1;
    hangFrames_ = (int)(hang_ms / 1000.0f * fs / hop);
    if (hangFrames_ < 1) hangFrames_ = 1;
}

void Vad::reset() {
    active_ = false;
    voiceCnt_ = 0;
    silenceCnt_ = 0;
}

int Vad::process(float *samples, size_t n) {
    float sq = 0.0f;
    for (size_t i = 0; i < n; ++i) sq += samples[i] * samples[i];
    float rms = (sq > 0.0f) ? std::sqrt(sq / (float)n) : 0.0f;
    bool isVoice = rms > thresholdLinear_;
    if (isVoice) { voiceCnt_++; silenceCnt_ = 0; }
    else { silenceCnt_++; voiceCnt_ = 0; }
    if (!active_ && voiceCnt_ >= onsetFrames_) active_ = true;
    else if (active_ && silenceCnt_ >= hangFrames_) active_ = false;
    if (!active_) {
        for (size_t i = 0; i < n; ++i) samples[i] = 0.0f;
    }
    return active_ ? 1 : 0;
}

void Vad::setThreshold(float dbfs) {
    thresholdLinear_ = std::pow(10.0f, dbfs / 20.0f);
}

float Vad::thresholdDbfs() const {
    return 20.0f * std::log10(thresholdLinear_);
}

}  // namespace pv
