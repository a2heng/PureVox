// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_STFT_H
#define PUREVOX_DSP_STFT_H

#include <cstddef>

struct PFFFT_Setup;

namespace pv {

// 统一 FFT/IFFT/OLA（2048-pt, 1024-hop, 48kHz）
// 频谱为扁平 [r0..r1024, i0..i1024] = 2050 floats
class Stft {
public:
    static constexpr int kFftSize = 2048;
    static constexpr int kHop = 1024;
    static constexpr int kFreq = 1025;
    static constexpr int kSpecFloats = 2050;

    Stft();
    ~Stft();
    Stft(const Stft &) = delete;
    Stft &operator=(const Stft &) = delete;

    void forward(const float *in, float *spec_planar);
    void backward(const float *spec_planar, float *out);
    void reset();

private:
    PFFFT_Setup *fftPlan_ = nullptr;
    float *fftIn_ = nullptr;
    float *fftOut_ = nullptr;
    float *ifftOut_ = nullptr;
    float *window_ = nullptr;
    float *inputHistory_ = nullptr;
    float *olaAcc_ = nullptr;
    float *winSum_ = nullptr;
    bool primed_ = false;
};

}  // namespace pv

#endif // PUREVOX_DSP_STFT_H
