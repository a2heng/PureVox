// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_RESAMPLER_H
#define PUREVOX_DSP_RESAMPLER_H

#include <cstddef>
#include <vector>

typedef struct SRC_STATE_tag SRC_STATE;

namespace pv {

// libsamplerate 封装
class Resampler {
public:
    explicit Resampler(int converter_type);
    ~Resampler();
    Resampler(const Resampler &) = delete;
    Resampler &operator=(const Resampler &) = delete;

    static int sincFastest();
    size_t run(const float *in, size_t n, double ratio, bool eof);
    size_t take(float *out, size_t cap);
    void reset();

private:
    SRC_STATE *state_ = nullptr;
    double currentRatio_ = 1.0;
    std::vector<float> out_;
};

}  // namespace pv

#endif // PUREVOX_DSP_RESAMPLER_H
