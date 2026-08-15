// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_DENOISE_H
#define PUREVOX_DSP_DENOISE_H

#include <cstddef>
#include <string>
#include <vector>

#include "onnxmodel.h"

struct PFFFT_Setup;

namespace pv {

// purevox9 降噪模型（2048 FFT + Band256, Single STFT）
// 外部接口：1024 样本块
class Denoise {
public:
    static constexpr int kFftSize = 2048;
    static constexpr int kHop = 1024;
    static constexpr int kFreq = 1025;
    static constexpr int kSpecSize = kFreq * 2;  // 2050 interleaved [r,i]

    Denoise();
    ~Denoise();
    Denoise(const Denoise &) = delete;
    Denoise &operator=(const Denoise &) = delete;

    bool init(const std::string &modelPath);
    void processChunk(const float *in, float *out);
    void processSpecOnly(const float *in, float *spec_out);
    void processSpecFreq(const float *in, float *out);
    void reset();
    bool valid() const { return model_.valid(); }
    int backendEffective() const { return model_.backendEffective(); }
    int backendReason() const { return model_.backendReason(); }

private:
    void computeSpec(const float *input_1024);
    void runOnnx();
    void synthOla();
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
    float *modelSpec_ = nullptr;
    std::vector<float> encC_, decC_, tfaC_, interC_;
    size_t encCSize_ = 0, decCSize_ = 0, tfaCSize_ = 0, interCSize_ = 0;
    std::vector<float> accOutput_;
};

}  // namespace pv

#endif // PUREVOX_DSP_DENOISE_H
