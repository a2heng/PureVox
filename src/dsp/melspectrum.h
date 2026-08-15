// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_MELSPECTRUM_H
#define PUREVOX_DSP_MELSPECTRUM_H

#include <cstddef>
#include <vector>

#include "dsp_common.h"

struct PFFFT_Setup;

namespace pv {

// 128 段 Mel 频谱（匹配人类听觉）
class MelSpectrum {
public:
    MelSpectrum();
    ~MelSpectrum();
    MelSpectrum(const MelSpectrum &) = delete;
    MelSpectrum &operator=(const MelSpectrum &) = delete;

    size_t compute(const float *samples, size_t n, float *out);
    void warmup();

private:
    static constexpr int kFftSize = 2048;
    static constexpr float kSampleRate = 48000.0f;
    static constexpr float kMelLowFreq = 20.0f;
    static constexpr float kMelHighFreq = 20000.0f;

    bool initFilterbank();
    static float hzToMel(float hz);
    static float melToHz(float mel);

    PFFFT_Setup *setup_ = nullptr;
    float *fftIn_ = nullptr;
    float *fftOut_ = nullptr;
    float *buf_ = nullptr;
    std::vector<float> weights_;
    std::vector<int> starts_;
    std::vector<int> ends_;
    int numBins_ = 0;
    bool initialized_ = false;
};

}  // namespace pv

#endif // PUREVOX_DSP_MELSPECTRUM_H
