// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_EQ_H
#define PUREVOX_DSP_EQ_H

#include <cstddef>

#include "dsp_common.h"

namespace pv {

// 61 段峰值 EQ（peaking biquad 级联）
class Equalizer {
public:
    Equalizer();
    void setGains(const float *gains, size_t n);
    void apply(float *data, size_t len);
    bool active() const { return active_; }
    static int bandCount() { return kEqBands; }
    static void freqs(float *out);

private:
    struct Biquad {
        float b0 = 1, b1 = 0, b2 = 0, a1 = 0, a2 = 0;
        float x1 = 0, x2 = 0, y1 = 0, y2 = 0;
    };
    Biquad designPeaking(float freq, float gain_db, float q, float sample_rate) const;
    void applyBiquad(Biquad *f, float *data, size_t len);

    Biquad filters_[kEqBands];
    bool active_ = false;
};

}  // namespace pv

#endif // PUREVOX_DSP_EQ_H
