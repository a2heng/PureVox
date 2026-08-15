// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "resampler.h"

#include <samplerate.h>

#include <cstdio>
#include <cstring>

namespace pv {

namespace {
constexpr size_t kMaxBlock = 4096;
}

int Resampler::sincFastest() { return SRC_SINC_FASTEST; }

Resampler::Resampler(int converter_type) {
    int error = 0;
    state_ = src_new(converter_type, 1, &error);
    if (!state_) {
        std::fprintf(stderr, "pv: Resampler init failed: %s\n", src_strerror(error));
    }
}

Resampler::~Resampler() {
    if (state_) src_delete(state_);
}

size_t Resampler::run(const float *in, size_t n, double ratio, bool eof) {
    if (!state_) return out_.size();
    if (n == 0 && !eof) return out_.size();
    if (ratio <= 0.0) ratio = 1.0;
    currentRatio_ = ratio;
    const float *ptr = in;
    size_t remaining = n;
    float tmp[kMaxBlock];
    SRC_DATA data;
    while (remaining > 0) {
        std::memset(&data, 0, sizeof(data));
        data.data_in = ptr;
        data.input_frames = (long)remaining;
        data.data_out = tmp;
        data.output_frames = kMaxBlock;
        data.src_ratio = ratio;
        data.end_of_input = 0;
        int err = src_process(state_, &data);
        if (err) { std::fprintf(stderr, "pv: resampler: %s\n", src_strerror(err)); break; }
        out_.insert(out_.end(), tmp, tmp + data.output_frames_gen);
        remaining -= (size_t)data.input_frames_used;
        ptr += data.input_frames_used;
    }
    if (eof) {
        for (;;) {
            std::memset(&data, 0, sizeof(data));
            data.data_in = nullptr;
            data.input_frames = 0;
            data.data_out = tmp;
            data.output_frames = kMaxBlock;
            data.src_ratio = ratio;
            data.end_of_input = 1;
            int err = src_process(state_, &data);
            if (err) { std::fprintf(stderr, "pv: resampler flush: %s\n", src_strerror(err)); break; }
            if (data.output_frames_gen == 0) break;
            out_.insert(out_.end(), tmp, tmp + data.output_frames_gen);
        }
    }
    return out_.size();
}

size_t Resampler::take(float *out, size_t cap) {
    size_t n = out_.size() < cap ? out_.size() : cap;
    if (n) std::memcpy(out, out_.data(), n * sizeof(float));
    out_.erase(out_.begin(), out_.begin() + n);
    return n;
}

void Resampler::reset() {
    if (state_) src_reset(state_);
    out_.clear();
}

}  // namespace pv
