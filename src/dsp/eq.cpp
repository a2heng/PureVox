// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "eq.h"

#include <cmath>

namespace pv {

namespace {
constexpr float kEqQ = 1.414f;
const float kEqFreqs[kEqBands] = {
    20.0f, 22.4f, 25.0f, 28.0f, 31.5f, 35.5f, 40.0f, 45.0f, 50.0f, 56.0f,
    63.0f, 71.0f, 80.0f, 90.0f, 100.0f, 112.0f, 125.0f, 140.0f, 160.0f, 180.0f,
    200.0f, 224.0f, 250.0f, 280.0f, 315.0f, 355.0f, 400.0f, 450.0f, 500.0f, 560.0f,
    630.0f, 710.0f, 800.0f, 900.0f, 1000.0f, 1120.0f, 1250.0f, 1400.0f, 1600.0f, 1800.0f,
    2000.0f, 2240.0f, 2500.0f, 2800.0f, 3150.0f, 3550.0f, 4000.0f, 4500.0f, 5000.0f, 5600.0f,
    6300.0f, 7100.0f, 8000.0f, 9000.0f, 10000.0f, 11200.0f, 12500.0f, 14000.0f, 16000.0f, 18000.0f,
    20000.0f};
}  // namespace

Equalizer::Equalizer() {
    for (int i = 0; i < kEqBands; ++i) {
        filters_[i] = designPeaking(kEqFreqs[i], 0.0f, kEqQ, kSampleRate);
    }
}

Equalizer::Biquad Equalizer::designPeaking(float freq, float gain_db, float q,
                                           float sample_rate) const {
    Biquad c;
    c.b0 = 1; c.b1 = 0; c.b2 = 0; c.a1 = 0; c.a2 = 0;
    float A = std::pow(10.0f, gain_db / 40.0f);
    float w0 = 2.0f * (float)M_PI * freq / sample_rate;
    float cos_w0 = std::cos(w0);
    float sin_w0 = std::sin(w0);
    float alpha = sin_w0 / (2.0f * q);
    float a0 = 1.0f + alpha / A;
    c.b0 = (1.0f + alpha * A) / a0;
    c.b1 = (-2.0f * cos_w0) / a0;
    c.b2 = (1.0f - alpha * A) / a0;
    c.a1 = (-2.0f * cos_w0) / a0;
    c.a2 = (1.0f - alpha / A) / a0;
    return c;
}

void Equalizer::setGains(const float *gains, size_t n) {
    bool anyNonzero = false;
    for (int i = 0; i < kEqBands; ++i) {
        float g = (i < (int)n) ? gains[i] : 0.0f;
        if (g != 0.0f) anyNonzero = true;
        filters_[i] = designPeaking(kEqFreqs[i], g, kEqQ, kSampleRate);
    }
    active_ = anyNonzero;
}

void Equalizer::freqs(float *out) {
    for (int i = 0; i < kEqBands; ++i) out[i] = kEqFreqs[i];
}

void Equalizer::applyBiquad(Biquad *f, float *data, size_t len) {
    for (size_t nd = 0; nd < len; ++nd) {
        float x = data[nd];
        float y = f->b0 * x + f->b1 * f->x1 + f->b2 * f->x2 - f->a1 * f->y1 - f->a2 * f->y2;
        f->x2 = f->x1;
        f->x1 = x;
        f->y2 = f->y1;
        f->y1 = y;
        data[nd] = y;
    }
}

void Equalizer::apply(float *data, size_t len) {
    if (!active_) return;
    for (int b = 0; b < kEqBands; ++b) applyBiquad(&filters_[b], data, len);
}

}  // namespace pv
