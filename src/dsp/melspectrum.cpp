// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "melspectrum.h"

#include <pffft.h>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>

namespace pv {

MelSpectrum::MelSpectrum() = default;

MelSpectrum::~MelSpectrum() {
    if (setup_) pffft_destroy_setup(setup_);
    if (fftIn_) pffft_aligned_free(fftIn_);
    if (fftOut_) pffft_aligned_free(fftOut_);
    if (buf_) pffft_aligned_free(buf_);
}

float MelSpectrum::hzToMel(float hz) { return 2595.0f * std::log10(1.0f + hz / 700.0f); }
float MelSpectrum::melToHz(float mel) { return 700.0f * (std::pow(10.0f, mel / 2595.0f) - 1.0f); }

bool MelSpectrum::initFilterbank() {
    if (initialized_) return true;
    int nFft = kFftSize;
    numBins_ = nFft / 2 + 1;
    float melMax = hzToMel(kMelHighFreq);
    float melMin = hzToMel(kMelLowFreq);

    std::vector<float> centerFreqs(kSpectrumNumBands + 2);
    for (int i = 0; i < kSpectrumNumBands + 2; ++i) {
        float melv = melMin + (melMax - melMin) * i / (kSpectrumNumBands + 1);
        centerFreqs[i] = melToHz(melv);
    }

    std::vector<int> cnt(numBins_, 0);
    for (int b = 0; b < kSpectrumNumBands; ++b) {
        float fl = centerFreqs[b], fc = centerFreqs[b + 1], fr = centerFreqs[b + 2];
        for (int k = 0; k < numBins_; ++k) {
            float freq = (float)k * kSampleRate / nFft;
            int active = 0;
            if (freq >= fl && freq <= fc && fc > fl) active = 1;
            else if (freq > fc && freq <= fr && fr > fc) active = 1;
            if (active) cnt[k]++;
        }
    }
    starts_.assign(numBins_ + 1, 0);
    int total = 0;
    for (int k = 0; k < numBins_; ++k) { starts_[k] = total; total += cnt[k]; }
    starts_[numBins_] = total;
    weights_.assign(total ? total : 1, 0.0f);
    ends_.assign(total ? total : 1, 0);

    std::vector<int> pos(numBins_, 0);
    for (int b = 0; b < kSpectrumNumBands; ++b) {
        float fl = centerFreqs[b], fc = centerFreqs[b + 1], fr = centerFreqs[b + 2];
        for (int k = 0; k < numBins_; ++k) {
            float freq = (float)k * kSampleRate / nFft;
            float w = 0.0f;
            if (freq >= fl && freq <= fc && fc > fl) w = (freq - fl) / (fc - fl);
            else if (freq > fc && freq <= fr && fr > fc) w = (fr - freq) / (fr - fc);
            if (w > 0.0f) {
                int idx = starts_[k] + pos[k];
                weights_[idx] = w;
                ends_[idx] = b;
                pos[k]++;
            }
        }
    }
    initialized_ = true;
    return true;
}

void MelSpectrum::warmup() {
    if (!initFilterbank()) return;
    if (!setup_) {
        setup_ = pffft_new_setup(kFftSize, PFFFT_REAL);
        if (setup_) {
            fftIn_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
            fftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
            buf_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
        }
    }
}

size_t MelSpectrum::compute(const float *samples, size_t n, float *out) {
    if (!initFilterbank()) {
        for (int i = 0; i < kSpectrumNumBands; ++i) out[i] = -90.0f;
        return kSpectrumNumBands;
    }
    if (!setup_) {
        setup_ = pffft_new_setup(kFftSize, PFFFT_REAL);
        if (setup_) {
            fftIn_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
            fftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
            buf_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
        }
    }
    if (!setup_ || !fftIn_ || !fftOut_ || !buf_) {
        for (int i = 0; i < kSpectrumNumBands; ++i) out[i] = -90.0f;
        return 0;
    }

    int copyLen = (int)(n < (size_t)kFftSize ? n : (size_t)kFftSize);
    std::memset(buf_, 0, kFftSize * sizeof(float));
    if (copyLen > 0) {
        int start = (int)n - copyLen;
        std::memcpy(buf_, samples + start, copyLen * sizeof(float));
    }
    for (int i = 0; i < kFftSize; ++i) {
        float w = 0.5f - 0.5f * std::cos(2.0 * M_PI * i / kFftSize);
        buf_[i] *= w;
    }
    std::memcpy(fftIn_, buf_, kFftSize * sizeof(float));
    pffft_transform_ordered(setup_, fftIn_, fftOut_, nullptr, PFFFT_FORWARD);

    float scale = 1.0f / (float)(kFftSize * kFftSize);
    std::vector<float> power(numBins_, 0.0f);
    power[0] = fftOut_[0] * fftOut_[0] * scale;
    for (int k = 1; k < kFftSize / 2; ++k) {
        float re = fftOut_[2 * k], im = fftOut_[2 * k + 1];
        power[k] = (re * re + im * im) * scale;
    }
    power[kFftSize / 2] = fftOut_[1] * fftOut_[1] * scale;

    std::vector<float> melEnergy(kSpectrumNumBands, 0.0f);
    for (int k = 0; k < numBins_; ++k) {
        int si = starts_[k], ei = starts_[k + 1];
        for (int j = si; j < ei; ++j) {
            int band = ends_[j];
            melEnergy[band] += power[k] * weights_[j];
        }
    }
    for (int i = 0; i < kSpectrumNumBands; ++i) {
        out[i] = -90.0f;
        if (melEnergy[i] > 1e-12f) {
            float db = 10.0f * std::log10(melEnergy[i]);
            out[i] = std::fmax(-90.0f, std::fmin(-20.0f, db));
        }
    }
    return kSpectrumNumBands;
}

}  // namespace pv
