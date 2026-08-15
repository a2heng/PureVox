// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "aec.h"

#include <pffft.h>

#include <cmath>
#include <cstdlib>
#include <cstring>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

namespace pv {

namespace {
constexpr size_t kResEncConvDefault = 108544;
constexpr size_t kResEncTfaDefault = 248;
constexpr size_t kMicEncConvDefault = 108544;
constexpr size_t kMicEncTfaDefault = 248;
constexpr size_t kDeepEncTfaDefault = 432;
constexpr size_t kDecConvDefault = 10752;
constexpr size_t kDecTfaDefault = 496;
constexpr size_t kInterDefault = 6144;
constexpr size_t kPrevDefault = 256;
constexpr size_t kDelayBufDefault = 39936;
}  // namespace

Aec::Aec() = default;

Aec::~Aec() { freeBuffers(); }

void Aec::freeBuffers() {
    if (fftPlan_) pffft_destroy_setup(fftPlan_);
    if (fftIn_) pffft_aligned_free(fftIn_);
    if (fftOut_) pffft_aligned_free(fftOut_);
    if (ifftOut_) pffft_aligned_free(ifftOut_);
    free(window_);
    free(micHistory_);
    free(farHistory_);
    free(olaAccumulator_);
    free(windowSum_);
    free(micOnnx_);
    free(farOnnx_);
    fftPlan_ = nullptr; fftIn_ = nullptr; fftOut_ = nullptr; ifftOut_ = nullptr;
    window_ = nullptr; micHistory_ = nullptr; farHistory_ = nullptr;
    olaAccumulator_ = nullptr; windowSum_ = nullptr; micOnnx_ = nullptr; farOnnx_ = nullptr;
    outAcc_.clear();
    outAccPos_ = 0;
}

bool Aec::init(const std::string &modelPath) {
    if (modelPath.empty()) return false;
    if (!model_.open("AecProcessor", modelPath)) return false;

    fftPlan_ = pffft_new_setup(kFftSize, PFFFT_REAL);
    fftIn_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    fftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    ifftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    window_ = (float *)malloc(kFftSize * sizeof(float));
    micHistory_ = (float *)calloc(kFftSize, sizeof(float));
    farHistory_ = (float *)calloc(kFftSize, sizeof(float));
    olaAccumulator_ = (float *)calloc(kFftSize, sizeof(float));
    windowSum_ = (float *)calloc(kFftSize, sizeof(float));
    micOnnx_ = (float *)calloc(kSpecSize, sizeof(float));
    farOnnx_ = (float *)calloc(kSpecSize, sizeof(float));
    if (!fftPlan_ || !fftIn_ || !fftOut_ || !ifftOut_ || !window_ || !micHistory_ ||
        !farHistory_ || !olaAccumulator_ || !windowSum_ || !micOnnx_ || !farOnnx_) {
        model_.close();
        freeBuffers();
        return false;
    }
    for (int i = 0; i < kFftSize; ++i) {
        float hann = 0.5f * (1.0f - std::cos(2.0 * M_PI * i / kFftSize));
        window_[i] = std::sqrt(hann);
    }

    resEncConvSz_ = model_.inputTotal("res_enc_conv", kResEncConvDefault);
    resEncTfaSz_ = model_.inputTotal("res_enc_tfa", kResEncTfaDefault);
    micEncConvSz_ = model_.inputTotal("mic_enc_conv", kMicEncConvDefault);
    micEncTfaSz_ = model_.inputTotal("mic_enc_tfa", kMicEncTfaDefault);
    deepEncTfaSz_ = model_.inputTotal("deep_enc_tfa", kDeepEncTfaDefault);
    decConvSz_ = model_.inputTotal("dec_conv", kDecConvDefault);
    decTfaSz_ = model_.inputTotal("dec_tfa", kDecTfaDefault);
    interSz_ = model_.inputTotal("inter", kInterDefault);
    prevSz_ = model_.inputTotal("res_prev1", kPrevDefault);
    delayBufSz_ = model_.inputTotal("delay_buf", kDelayBufDefault);

    resEncConv_.assign(resEncConvSz_, 0.0f);
    resEncTfa_.assign(resEncTfaSz_, 0.0f);
    micEncConv_.assign(micEncConvSz_, 0.0f);
    micEncTfa_.assign(micEncTfaSz_, 0.0f);
    deepEncTfa_.assign(deepEncTfaSz_, 0.0f);
    decConv_.assign(decConvSz_, 0.0f);
    decTfa_.assign(decTfaSz_, 0.0f);
    inter_.assign(interSz_, 0.0f);
    resPrev1_.assign(prevSz_, 0.0f);
    resPrev2_.assign(prevSz_, 0.0f);
    micPrev1_.assign(prevSz_, 0.0f);
    micPrev2_.assign(prevSz_, 0.0f);
    delayBuf_.assign(delayBufSz_, 0.0f);
    return true;
}

void Aec::computeStftFrame(const float *input_nfft, float *onnx_spec) {
    for (int i = 0; i < kFftSize; ++i) fftIn_[i] = input_nfft[i] * window_[i];
    pffft_transform_ordered(fftPlan_, fftIn_, fftOut_, nullptr, PFFFT_FORWARD);
    onnx_spec[0] = fftOut_[0];
    onnx_spec[kFreq] = 0.0f;
    for (int k = 1; k < kFreq - 1; ++k) {
        int p = 2 + (k - 1) * 2;
        onnx_spec[k] = fftOut_[p];
        onnx_spec[kFreq + k] = fftOut_[p + 1];
    }
    onnx_spec[kFreq - 1] = fftOut_[1];
    onnx_spec[kFreq + kFreq - 1] = 0.0f;
}

void Aec::runOnnx() {
    size_t nin = model_.nInputs(), nout = model_.nOutputs();
    OrtValue *inputs[16] = {nullptr};
    OrtValue *outputs[16] = {nullptr};
    int64_t frameShape[4] = {1, 2, 1, kFreq};
    int64_t s2[2];
    int64_t prevShape[4] = {1, 1, 1, (int64_t)prevSz_};
    int64_t delayShape[4] = {1, 3, 52, 256};
    for (size_t i = 0; i < nin && i < 16; ++i) {
        const char *name = model_.inputName(i);
        if (!name) continue;
        if (std::strcmp(name, "mic_frame") == 0) {
            inputs[i] = model_.makeTensor(micOnnx_, kSpecSize, frameShape, 4);
        } else if (std::strcmp(name, "far_frame") == 0) {
            inputs[i] = model_.makeTensor(farOnnx_, kSpecSize, frameShape, 4);
        } else if (std::strcmp(name, "res_enc_conv") == 0) {
            s2[0] = 1; s2[1] = (int64_t)resEncConvSz_;
            inputs[i] = model_.makeTensor(resEncConv_.data(), resEncConvSz_, s2, 2);
        } else if (std::strcmp(name, "res_enc_tfa") == 0) {
            s2[0] = 1; s2[1] = (int64_t)resEncTfaSz_;
            inputs[i] = model_.makeTensor(resEncTfa_.data(), resEncTfaSz_, s2, 2);
        } else if (std::strcmp(name, "mic_enc_conv") == 0) {
            s2[0] = 1; s2[1] = (int64_t)micEncConvSz_;
            inputs[i] = model_.makeTensor(micEncConv_.data(), micEncConvSz_, s2, 2);
        } else if (std::strcmp(name, "mic_enc_tfa") == 0) {
            s2[0] = 1; s2[1] = (int64_t)micEncTfaSz_;
            inputs[i] = model_.makeTensor(micEncTfa_.data(), micEncTfaSz_, s2, 2);
        } else if (std::strcmp(name, "deep_enc_conv") == 0) {
            s2[0] = 1; s2[1] = 0;
            inputs[i] = model_.makeTensor(deepEncTfa_.data(), 0, s2, 2);
        } else if (std::strcmp(name, "deep_enc_tfa") == 0) {
            s2[0] = 1; s2[1] = (int64_t)deepEncTfaSz_;
            inputs[i] = model_.makeTensor(deepEncTfa_.data(), deepEncTfaSz_, s2, 2);
        } else if (std::strcmp(name, "dec_conv") == 0) {
            s2[0] = 1; s2[1] = (int64_t)decConvSz_;
            inputs[i] = model_.makeTensor(decConv_.data(), decConvSz_, s2, 2);
        } else if (std::strcmp(name, "dec_tfa") == 0) {
            s2[0] = 1; s2[1] = (int64_t)decTfaSz_;
            inputs[i] = model_.makeTensor(decTfa_.data(), decTfaSz_, s2, 2);
        } else if (std::strcmp(name, "inter") == 0) {
            s2[0] = 1; s2[1] = (int64_t)interSz_;
            inputs[i] = model_.makeTensor(inter_.data(), interSz_, s2, 2);
        } else if (std::strcmp(name, "res_prev1") == 0) {
            inputs[i] = model_.makeTensor(resPrev1_.data(), prevSz_, prevShape, 4);
        } else if (std::strcmp(name, "res_prev2") == 0) {
            inputs[i] = model_.makeTensor(resPrev2_.data(), prevSz_, prevShape, 4);
        } else if (std::strcmp(name, "mic_prev1") == 0) {
            inputs[i] = model_.makeTensor(micPrev1_.data(), prevSz_, prevShape, 4);
        } else if (std::strcmp(name, "mic_prev2") == 0) {
            inputs[i] = model_.makeTensor(micPrev2_.data(), prevSz_, prevShape, 4);
        } else if (std::strcmp(name, "delay_buf") == 0) {
            inputs[i] = model_.makeTensor(delayBuf_.data(), delayBufSz_, delayShape, 4);
        } else {
            s2[0] = 1; s2[1] = 0;
            inputs[i] = model_.makeTensor(resEncConv_.data(), 0, s2, 2);
        }
    }
    model_.run(nullptr, inputs, nin, nullptr, outputs, nout);

    for (size_t i = 0; i < nout && i < 16; ++i) {
        const char *name = model_.outputName(i);
        if (!name || !outputs[i]) continue;
        float *data = model_.tensorData(outputs[i]);
        size_t total = model_.tensorElems(outputs[i]);
        if (!data) continue;
        size_t n;
        if (std::strcmp(name, "enhanced_frame") == 0) {
            fftOut_[0] = data[0];
            fftOut_[1] = data[kFreq - 1];
            for (int k = 1; k < kFreq - 1; ++k) {
                int p = 2 + (k - 1) * 2;
                fftOut_[p] = data[k];
                fftOut_[p + 1] = data[kFreq + k];
            }
        } else if (std::strcmp(name, "res_enc_conv") == 0 || std::strcmp(name, "res_enc_conv_o") == 0) {
            n = total < resEncConvSz_ ? total : resEncConvSz_;
            std::memcpy(resEncConv_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "res_enc_tfa") == 0 || std::strcmp(name, "res_enc_tfa_o") == 0) {
            n = total < resEncTfaSz_ ? total : resEncTfaSz_;
            std::memcpy(resEncTfa_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "mic_enc_conv") == 0 || std::strcmp(name, "mic_enc_conv_o") == 0) {
            n = total < micEncConvSz_ ? total : micEncConvSz_;
            std::memcpy(micEncConv_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "mic_enc_tfa") == 0 || std::strcmp(name, "mic_enc_tfa_o") == 0) {
            n = total < micEncTfaSz_ ? total : micEncTfaSz_;
            std::memcpy(micEncTfa_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "deep_enc_tfa") == 0 || std::strcmp(name, "deep_enc_tfa_o") == 0) {
            n = total < deepEncTfaSz_ ? total : deepEncTfaSz_;
            std::memcpy(deepEncTfa_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "dec_conv") == 0 || std::strcmp(name, "dec_conv_o") == 0) {
            n = total < decConvSz_ ? total : decConvSz_;
            std::memcpy(decConv_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "dec_tfa") == 0 || std::strcmp(name, "dec_tfa_o") == 0) {
            n = total < decTfaSz_ ? total : decTfaSz_;
            std::memcpy(decTfa_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "inter") == 0 || std::strcmp(name, "inter_o") == 0) {
            n = total < interSz_ ? total : interSz_;
            std::memcpy(inter_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "res_prev1") == 0 || std::strcmp(name, "res_prev1_o") == 0) {
            n = total < prevSz_ ? total : prevSz_;
            std::memcpy(resPrev1_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "res_prev2") == 0 || std::strcmp(name, "res_prev2_o") == 0) {
            n = total < prevSz_ ? total : prevSz_;
            std::memcpy(resPrev2_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "mic_prev1") == 0 || std::strcmp(name, "mic_prev1_o") == 0) {
            n = total < prevSz_ ? total : prevSz_;
            std::memcpy(micPrev1_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "mic_prev2") == 0 || std::strcmp(name, "mic_prev2_o") == 0) {
            n = total < prevSz_ ? total : prevSz_;
            std::memcpy(micPrev2_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "delay_buf") == 0 || std::strcmp(name, "delay_buf_o") == 0) {
            n = total < delayBufSz_ ? total : delayBufSz_;
            std::memcpy(delayBuf_.data(), data, n * sizeof(float));
        }
    }
    for (size_t i = 0; i < nin && i < 16; ++i) model_.releaseValue(inputs[i]);
    for (size_t i = 0; i < nout && i < 16; ++i) model_.releaseValue(outputs[i]);
}

void Aec::processOneFrame(const float *mic_1024, const float *far_1024) {
    std::memmove(micHistory_, micHistory_ + kHop, (kFftSize - kHop) * sizeof(float));
    std::memcpy(micHistory_ + kFftSize - kHop, mic_1024, kHop * sizeof(float));
    std::memmove(farHistory_, farHistory_ + kHop, (kFftSize - kHop) * sizeof(float));
    std::memcpy(farHistory_ + kFftSize - kHop, far_1024, kHop * sizeof(float));

    computeStftFrame(micHistory_, micOnnx_);
    computeStftFrame(farHistory_, farOnnx_);
    runOnnx();

    pffft_transform_ordered(fftPlan_, fftOut_, ifftOut_, nullptr, PFFFT_BACKWARD);
    float scale = 1.0f / kFftSize;
    for (int i = 0; i < kFftSize; ++i) ifftOut_[i] *= scale * window_[i];
    for (int i = 0; i < kFftSize; ++i) olaAccumulator_[i] += ifftOut_[i];
    for (int i = 0; i < kFftSize; ++i) windowSum_[i] += window_[i] * window_[i];

    for (int i = 0; i < kHop; ++i) {
        float norm = windowSum_[i];
        float val = (norm > 1e-6f) ? (olaAccumulator_[i] / norm) : olaAccumulator_[i];
        outAcc_.push_back(val);
    }
    for (int i = 0; i < kFftSize - kHop; ++i) {
        olaAccumulator_[i] = olaAccumulator_[i + kHop];
        windowSum_[i] = windowSum_[i + kHop];
    }
    for (int i = kFftSize - kHop; i < kFftSize; ++i) {
        olaAccumulator_[i] = 0.0f;
        windowSum_[i] = 0.0f;
    }
}

void Aec::processFrame(const float *mic, const float *far, float *out) {
    processOneFrame(mic, far);
    if (outAcc_.size() - outAccPos_ >= (size_t)kHop) {
        std::memcpy(out, outAcc_.data() + outAccPos_, kHop * sizeof(float));
        outAccPos_ += kHop;
    } else {
        std::memset(out, 0, kHop * sizeof(float));
    }
    if (outAccPos_ >= (size_t)kHop && outAccPos_ <= outAcc_.size()) {
        outAcc_.erase(outAcc_.begin(), outAcc_.begin() + outAccPos_);
        outAccPos_ = 0;
    }
}

void Aec::reset() {
    std::fill(resEncConv_.begin(), resEncConv_.end(), 0.0f);
    std::fill(resEncTfa_.begin(), resEncTfa_.end(), 0.0f);
    std::fill(micEncConv_.begin(), micEncConv_.end(), 0.0f);
    std::fill(micEncTfa_.begin(), micEncTfa_.end(), 0.0f);
    std::fill(deepEncTfa_.begin(), deepEncTfa_.end(), 0.0f);
    std::fill(decConv_.begin(), decConv_.end(), 0.0f);
    std::fill(decTfa_.begin(), decTfa_.end(), 0.0f);
    std::fill(inter_.begin(), inter_.end(), 0.0f);
    std::fill(resPrev1_.begin(), resPrev1_.end(), 0.0f);
    std::fill(resPrev2_.begin(), resPrev2_.end(), 0.0f);
    std::fill(micPrev1_.begin(), micPrev1_.end(), 0.0f);
    std::fill(micPrev2_.begin(), micPrev2_.end(), 0.0f);
    std::fill(delayBuf_.begin(), delayBuf_.end(), 0.0f);
    std::memset(micHistory_, 0, kFftSize * sizeof(float));
    std::memset(farHistory_, 0, kFftSize * sizeof(float));
    std::memset(olaAccumulator_, 0, kFftSize * sizeof(float));
    std::memset(windowSum_, 0, kFftSize * sizeof(float));
    outAcc_.clear();
    outAccPos_ = 0;
}

}  // namespace pv
