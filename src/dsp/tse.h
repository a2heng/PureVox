// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_TSE_H
#define PUREVOX_DSP_TSE_H

#include <cstddef>
#include <string>
#include <vector>

#include "onnxmodel.h"

struct PFFFT_Setup;

namespace pv {

// tse15 streaming ONNX（2048 FFT, 1024 HOP, 48kHz, flat cache）
class Tse {
public:
    static constexpr int kFftSize = 2048;
    static constexpr int kHop = 1024;
    static constexpr int kFreq = 1025;
    static constexpr int kSpecFloats = 2 * kFreq;
    static constexpr int kEnrCh = 2;
    static constexpr int kEnrFloats = kEnrCh * kFreq;
    static constexpr size_t kCacheTotal = 319040;

    Tse();
    ~Tse();
    Tse(const Tse &) = delete;
    Tse &operator=(const Tse &) = delete;

    bool init(const std::string &modelPath);
    bool hasReference() const { return enrLen_ > 0; }
    void setReference(const float *ref, size_t n);
    void setDebugDump(bool enable, const std::string &dir);
    void processChunk(const float *in, float *out);
    void processSpecFreq(const float *in, float *out);
    void processFromSpec(const float *spec, float *out);
    void reset();
    bool valid() const { return model_.valid(); }
    int backendEffective() const { return model_.backendEffective(); }
    int backendReason() const { return model_.backendReason(); }

private:
    void runOnnx();
    void synthOla(const float *in_1024, float *out_1024);
    void freeBuffers();

    OnnxModel model_;
    PFFFT_Setup *fftPlan_ = nullptr;
    float *fftIn_ = nullptr;
    float *fftOut_ = nullptr;
    float *ifftOut_ = nullptr;
    float *window_ = nullptr;
    float *inputHistory_ = nullptr;
    float *olaAccumulator_ = nullptr;
    float *windowSum_ = nullptr;
    bool primed_ = false;
    int frameCount_ = 0;
    bool debugDump_ = false;
    std::string debugDir_;
    std::vector<float> cache_;
    std::vector<float> enrBuf_;
    size_t enrLen_ = 0;
    size_t enrFrames_ = 0;
    std::vector<float> specBuf_;
};

}  // namespace pv

#endif // PUREVOX_DSP_TSE_H
