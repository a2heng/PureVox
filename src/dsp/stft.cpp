// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "stft.h"

#include <pffft.h>

#include <cstdlib>
#include <cstring>

#include "dsp_common.h"

namespace pv {

Stft::Stft() {
    fftPlan_ = pffft_new_setup(kFftSize, PFFFT_REAL);
    fftIn_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    fftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    ifftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    window_ = (float *)malloc(kFftSize * sizeof(float));
    inputHistory_ = (float *)calloc(kFftSize - kHop, sizeof(float));
    olaAcc_ = (float *)calloc(kFftSize, sizeof(float));
    winSum_ = (float *)calloc(kFftSize, sizeof(float));
    make_sqrt_hann(window_, kFftSize);
    for (int i = 0; i < kFftSize; ++i) window_[i] *= window_[i];  // Hann
}

Stft::~Stft() {
    if (fftPlan_) pffft_destroy_setup(fftPlan_);
    if (fftIn_) pffft_aligned_free(fftIn_);
    if (fftOut_) pffft_aligned_free(fftOut_);
    if (ifftOut_) pffft_aligned_free(ifftOut_);
    free(window_);
    free(inputHistory_);
    free(olaAcc_);
    free(winSum_);
}

void Stft::forward(const float *in, float *spec_planar) {
    size_t prev = kFftSize - kHop;
    std::memcpy(fftIn_, inputHistory_, prev * sizeof(float));
    std::memcpy(fftIn_ + prev, in, kHop * sizeof(float));
    std::memmove(inputHistory_, inputHistory_ + kHop, (prev - kHop) * sizeof(float));
    std::memcpy(inputHistory_ + prev - kHop, in, kHop * sizeof(float));
    for (int i = 0; i < kFftSize; ++i) fftIn_[i] *= window_[i];
    pffft_transform_ordered(fftPlan_, fftIn_, fftOut_, nullptr, PFFFT_FORWARD);
    float *rp = spec_planar;
    float *ip = spec_planar + kFreq;
    rp[0] = fftOut_[0]; ip[0] = 0.0f;
    rp[kFreq - 1] = fftOut_[1]; ip[kFreq - 1] = 0.0f;
    for (int k = 1; k < kFreq - 1; ++k) {
        int p = 2 + (k - 1) * 2;
        rp[k] = fftOut_[p]; ip[k] = fftOut_[p + 1];
    }
}

void Stft::backward(const float *spec_planar, float *out) {
    fftOut_[0] = spec_planar[0];
    fftOut_[1] = spec_planar[kFreq - 1];
    for (int k = 1; k < kFreq - 1; ++k) {
        int p = 2 + (k - 1) * 2;
        fftOut_[p] = spec_planar[k];
        fftOut_[p + 1] = spec_planar[kFreq + k];
    }
    pffft_transform_ordered(fftPlan_, fftOut_, ifftOut_, nullptr, PFFFT_BACKWARD);
    float sc = 1.0f / kFftSize;
    for (int i = 0; i < kFftSize; ++i) {
        ifftOut_[i] *= sc * window_[i];
        olaAcc_[i] += ifftOut_[i];
        winSum_[i] += window_[i] * window_[i];
    }
    if (!primed_) {
        primed_ = true;
        std::memset(out, 0, kHop * sizeof(float));
    } else {
        for (int i = 0; i < kHop; ++i) {
            float n = winSum_[i];
            out[i] = (n > 1e-6f) ? (olaAcc_[i] / n) : olaAcc_[i];
        }
    }
    for (int i = 0; i < kFftSize - kHop; ++i) {
        olaAcc_[i] = olaAcc_[i + kHop];
        winSum_[i] = winSum_[i + kHop];
    }
    for (int i = kFftSize - kHop; i < kFftSize; ++i) {
        olaAcc_[i] = 0.0f;
        winSum_[i] = 0.0f;
    }
}

void Stft::reset() {
    std::memset(inputHistory_, 0, (kFftSize - kHop) * sizeof(float));
    std::memset(olaAcc_, 0, kFftSize * sizeof(float));
    std::memset(winSum_, 0, kFftSize * sizeof(float));
    primed_ = false;
}

}  // namespace pv
