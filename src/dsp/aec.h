// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_AEC_H
#define PUREVOX_DSP_AEC_H

#include <cstddef>
#include <string>
#include <vector>

#include "onnxmodel.h"

struct PFFFT_Setup;

namespace pv {

// aec9 streaming ONNX（NFFT=2048, HOP=1024, FREQ=1025, Mel-256, delay line）
class Aec {
public:
    static constexpr int kFftSize = 2048;
    static constexpr int kHop = 1024;
    static constexpr int kFreq = 1025;
    static constexpr int kSpecSize = kFreq * 2;

    Aec();
    ~Aec();
    Aec(const Aec &) = delete;
    Aec &operator=(const Aec &) = delete;

    bool init(const std::string &modelPath);
    void processFrame(const float *mic, const float *far, float *out);
    void reset();
    bool valid() const { return model_.valid(); }
    int backendEffective() const { return model_.backendEffective(); }
    int backendReason() const { return model_.backendReason(); }

private:
    void allocSizes();
    void computeStftFrame(const float *input_nfft, float *onnx_spec);
    void runOnnx();
    void processOneFrame(const float *mic_1024, const float *far_1024);
    void freeBuffers();

    OnnxModel model_;
    PFFFT_Setup *fftPlan_ = nullptr;
    float *fftIn_ = nullptr;
    float *fftOut_ = nullptr;
    float *ifftOut_ = nullptr;
    float *window_ = nullptr;
    float *micHistory_ = nullptr;
    float *farHistory_ = nullptr;
    float *olaAccumulator_ = nullptr;
    float *windowSum_ = nullptr;
    float *micOnnx_ = nullptr;
    float *farOnnx_ = nullptr;
    size_t resEncConvSz_ = 0, resEncTfaSz_ = 0, micEncConvSz_ = 0, micEncTfaSz_ = 0;
    size_t deepEncTfaSz_ = 0, decConvSz_ = 0, decTfaSz_ = 0, interSz_ = 0, prevSz_ = 0, delayBufSz_ = 0;
    std::vector<float> resEncConv_, resEncTfa_, micEncConv_, micEncTfa_;
    std::vector<float> deepEncTfa_, decConv_, decTfa_, inter_;
    std::vector<float> resPrev1_, resPrev2_, micPrev1_, micPrev2_, delayBuf_;
    std::vector<float> outAcc_;
    size_t outAccPos_ = 0;
};

}  // namespace pv

#endif // PUREVOX_DSP_AEC_H
