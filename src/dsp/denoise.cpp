// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "denoise.h"

#include <pffft.h>

#include <cstdlib>
#include <cstring>

#include "dsp_common.h"

namespace pv {

namespace {
constexpr size_t kEncCDefault = 77106;
constexpr size_t kDecCDefault = 53862;
constexpr size_t kTfaCDefault = 1056;
constexpr size_t kInterCDefault = 1024;
}  // namespace

Denoise::Denoise() = default;

Denoise::~Denoise() { freeBuffers(); }

void Denoise::freeBuffers() {
    if (fftPlan_) pffft_destroy_setup(fftPlan_);
    if (fftIn_) pffft_aligned_free(fftIn_);
    if (fftOut_) pffft_aligned_free(fftOut_);
    if (ifftOut_) pffft_aligned_free(ifftOut_);
    free(window_);
    free(inputHistory_);
    free(olaAccumulator_);
    free(windowSum_);
    free(modelSpec_);
    fftPlan_ = nullptr; fftIn_ = nullptr; fftOut_ = nullptr; ifftOut_ = nullptr;
    window_ = nullptr; inputHistory_ = nullptr; olaAccumulator_ = nullptr;
    windowSum_ = nullptr; modelSpec_ = nullptr;
    accOutput_.clear();
}

bool Denoise::init(const std::string &modelPath) {
    if (modelPath.empty()) return false;
    if (!model_.open("DenoiseProcessor", modelPath)) return false;

    fftPlan_ = pffft_new_setup(kFftSize, PFFFT_REAL);
    fftIn_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    fftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    ifftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    window_ = (float *)malloc(kFftSize * sizeof(float));
    inputHistory_ = (float *)calloc(kFftSize - kHop, sizeof(float));
    olaAccumulator_ = (float *)calloc(kFftSize, sizeof(float));
    windowSum_ = (float *)calloc(kFftSize, sizeof(float));
    modelSpec_ = (float *)calloc(kSpecSize, sizeof(float));
    if (!fftPlan_ || !fftIn_ || !fftOut_ || !ifftOut_ || !window_ || !inputHistory_ ||
        !olaAccumulator_ || !windowSum_ || !modelSpec_) {
        model_.close();
        freeBuffers();
        return false;
    }
    make_sqrt_hann(window_, kFftSize);

    encCSize_ = model_.inputTotal("enc_c", kEncCDefault);
    decCSize_ = model_.inputTotal("dec_c", kDecCDefault);
    tfaCSize_ = model_.inputTotal("tfa_c", kTfaCDefault);
    interCSize_ = model_.inputTotal("inter_c", kInterCDefault);
    encC_.assign(encCSize_, 0.0f);
    decC_.assign(decCSize_, 0.0f);
    tfaC_.assign(tfaCSize_, 0.0f);
    interC_.assign(interCSize_, 0.0f);

    // warm-up：3 个静音块
    std::vector<float> silent(kHop, 0.0f), dummy(kHop);
    for (int i = 0; i < 3; ++i) processChunk(silent.data(), dummy.data());
    accOutput_.clear();
    return true;
}

void Denoise::computeSpec(const float *input_1024) {
    size_t prevSize = kFftSize - kHop;
    std::memcpy(fftIn_, inputHistory_, prevSize * sizeof(float));
    std::memcpy(fftIn_ + prevSize, input_1024, kHop * sizeof(float));
    std::memmove(inputHistory_, inputHistory_ + kHop, (prevSize - kHop) * sizeof(float));
    std::memcpy(inputHistory_ + prevSize - kHop, input_1024, kHop * sizeof(float));
    for (int i = 0; i < kFftSize; ++i) fftIn_[i] *= window_[i];
    pffft_transform_ordered(fftPlan_, fftIn_, fftOut_, nullptr, PFFFT_FORWARD);
    modelSpec_[0] = fftOut_[0];
    modelSpec_[1] = 0.0f;
    modelSpec_[kSpecSize - 2] = fftOut_[1];
    modelSpec_[kSpecSize - 1] = 0.0f;
    for (int k = 1; k < kFreq - 1; ++k) {
        int pidx = 2 + (k - 1) * 2;
        modelSpec_[k * 2] = fftOut_[pidx];
        modelSpec_[k * 2 + 1] = fftOut_[pidx + 1];
    }
}

void Denoise::runOnnx() {
    size_t nin = model_.nInputs(), nout = model_.nOutputs();
    OrtValue *inputs[16] = {nullptr};
    OrtValue *outputs[16] = {nullptr};
    int64_t specShape[4] = {1, kFreq, 1, 2};
    int64_t s2[2];
    for (size_t i = 0; i < nin && i < 16; ++i) {
        const char *name = model_.inputName(i);
        if (!name) continue;
        if (std::strcmp(name, "spec") == 0)
            inputs[i] = model_.makeTensor(modelSpec_, kSpecSize, specShape, 4);
        else if (std::strcmp(name, "enc_c") == 0) {
            s2[0] = 1; s2[1] = (int64_t)encCSize_;
            inputs[i] = model_.makeTensor(encC_.data(), encCSize_, s2, 2);
        } else if (std::strcmp(name, "dec_c") == 0) {
            s2[0] = 1; s2[1] = (int64_t)decCSize_;
            inputs[i] = model_.makeTensor(decC_.data(), decCSize_, s2, 2);
        } else if (std::strcmp(name, "tfa_c") == 0) {
            s2[0] = 1; s2[1] = (int64_t)tfaCSize_;
            inputs[i] = model_.makeTensor(tfaC_.data(), tfaCSize_, s2, 2);
        } else if (std::strcmp(name, "inter_c") == 0) {
            s2[0] = 1; s2[1] = (int64_t)interCSize_;
            inputs[i] = model_.makeTensor(interC_.data(), interCSize_, s2, 2);
        } else {
            s2[0] = 1; s2[1] = 0;
            inputs[i] = model_.makeTensor(modelSpec_, 0, s2, 2);
        }
    }
    model_.run(nullptr, inputs, nin, nullptr, outputs, nout);
    for (size_t i = 0; i < nout && i < 16; ++i) {
        const char *name = model_.outputName(i);
        if (!name || !outputs[i]) continue;
        float *data = model_.tensorData(outputs[i]);
        size_t total = model_.tensorElems(outputs[i]);
        if (!data) continue;
        if (std::strcmp(name, "enhanced_spec") == 0)
            std::memcpy(modelSpec_, data, kSpecSize * sizeof(float));
        else if (std::strcmp(name, "enc_c_out") == 0) {
            size_t n = total < encCSize_ ? total : encCSize_;
            std::memcpy(encC_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "dec_c_out") == 0) {
            size_t n = total < decCSize_ ? total : decCSize_;
            std::memcpy(decC_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "tfa_c_out") == 0) {
            size_t n = total < tfaCSize_ ? total : tfaCSize_;
            std::memcpy(tfaC_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "inter_c_out") == 0) {
            size_t n = total < interCSize_ ? total : interCSize_;
            std::memcpy(interC_.data(), data, n * sizeof(float));
        }
    }
    for (size_t i = 0; i < nin && i < 16; ++i) model_.releaseValue(inputs[i]);
    for (size_t i = 0; i < nout && i < 16; ++i) model_.releaseValue(outputs[i]);
}

void Denoise::synthOla() {
    fftOut_[0] = modelSpec_[0];
    fftOut_[1] = modelSpec_[kSpecSize - 2];
    for (int k = 1; k < kFreq - 1; ++k) {
        int p = 2 + (k - 1) * 2;
        fftOut_[p] = modelSpec_[k * 2];
        fftOut_[p + 1] = modelSpec_[k * 2 + 1];
    }
    pffft_transform_ordered(fftPlan_, fftOut_, ifftOut_, nullptr, PFFFT_BACKWARD);
    float scale = 1.0f / kFftSize;
    for (int i = 0; i < kFftSize; ++i) ifftOut_[i] *= scale * window_[i];
    for (int i = 0; i < kFftSize; ++i) olaAccumulator_[i] += ifftOut_[i];
    for (int i = 0; i < kFftSize; ++i) windowSum_[i] += window_[i] * window_[i];
    for (int i = 0; i < kHop; ++i) {
        float norm = windowSum_[i];
        float val = (norm > 1e-6f) ? (olaAccumulator_[i] / norm) : olaAccumulator_[i];
        accOutput_.push_back(val);
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

void Denoise::processChunk(const float *in, float *out) {
    computeSpec(in);
    runOnnx();
    synthOla();
    if (accOutput_.size() >= kHop) {
        std::memcpy(out, accOutput_.data(), kHop * sizeof(float));
        accOutput_.erase(accOutput_.begin(), accOutput_.begin() + kHop);
    } else {
        std::memset(out, 0, kHop * sizeof(float));
    }
}

void Denoise::processSpecOnly(const float *in, float *spec_out) {
    computeSpec(in);
    runOnnx();
    std::memcpy(spec_out, modelSpec_, kSpecSize * sizeof(float));
}

void Denoise::processSpecFreq(const float *in, float *out) {
    std::memcpy(modelSpec_, in, kSpecSize * sizeof(float));
    runOnnx();
    std::memcpy(out, modelSpec_, kSpecSize * sizeof(float));
}

void Denoise::reset() {
    std::fill(encC_.begin(), encC_.end(), 0.0f);
    std::fill(decC_.begin(), decC_.end(), 0.0f);
    std::fill(tfaC_.begin(), tfaC_.end(), 0.0f);
    std::fill(interC_.begin(), interC_.end(), 0.0f);
    std::memset(inputHistory_, 0, (kFftSize - kHop) * sizeof(float));
    std::memset(olaAccumulator_, 0, kFftSize * sizeof(float));
    std::memset(windowSum_, 0, kFftSize * sizeof(float));
    std::memset(modelSpec_, 0, kSpecSize * sizeof(float));
    accOutput_.clear();
}

}  // namespace pv
