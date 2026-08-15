// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_COMMON_H
#define PUREVOX_DSP_COMMON_H

#include <cmath>
#include <cstring>
#include <vector>

namespace pv {

constexpr size_t kHopLength = 1024;
constexpr float kSampleRate = 48000.0f;
constexpr int kEqBands = 61;
constexpr int kSpectrumNumBands = 128;

inline float clip_sample(float x) {
    if (std::isnan(x) || std::isinf(x)) return 0.0f;
    if (x > 1.0f) return 1.0f;
    if (x < -1.0f) return -1.0f;
    return x;
}

inline void clip_buffer(float *data, size_t len) {
    for (size_t i = 0; i < len; ++i) data[i] = clip_sample(data[i]);
}

// sqrt-Hann analysis window（torch.hann_window().pow(0.5) 语义）
inline void make_sqrt_hann(float *w, int n) {
    for (int i = 0; i < n; ++i) {
        float hann = 0.5f - 0.5f * std::cos(2.0 * M_PI * i / n);
        w[i] = std::sqrt(hann + 1e-10f);
    }
}

}  // namespace pv

#endif // PUREVOX_DSP_COMMON_H
