// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "tse.h"

#include <pffft.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "dsp_common.h"

namespace pv {

namespace {
void dumpBin(const char *path, const float *data, size_t n) {
    FILE *f = fopen(path, "wb");
    if (f) { fwrite(data, sizeof(float), n, f); fclose(f); }
}
}  // namespace

Tse::Tse() = default;

Tse::~Tse() { freeBuffers(); }

void Tse::freeBuffers() {
    if (fftPlan_) pffft_destroy_setup(fftPlan_);
    if (fftIn_) pffft_aligned_free(fftIn_);
    if (fftOut_) pffft_aligned_free(fftOut_);
    if (ifftOut_) pffft_aligned_free(ifftOut_);
    free(window_);
    free(inputHistory_);
    free(olaAccumulator_);
    free(windowSum_);
    fftPlan_ = nullptr; fftIn_ = nullptr; fftOut_ = nullptr; ifftOut_ = nullptr;
    window_ = nullptr; inputHistory_ = nullptr; olaAccumulator_ = nullptr; windowSum_ = nullptr;
    specBuf_.clear();
    cache_.clear();
    enrBuf_.clear();
    enrLen_ = 0;
    enrFrames_ = 0;
}

bool Tse::init(const std::string &modelPath) {
    if (modelPath.empty()) return false;
    if (!model_.open("TseProcessor", modelPath)) return false;

    fftPlan_ = pffft_new_setup(kFftSize, PFFFT_REAL);
    fftIn_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    fftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    ifftOut_ = (float *)pffft_aligned_malloc(kFftSize * sizeof(float));
    window_ = (float *)malloc(kFftSize * sizeof(float));
    inputHistory_ = (float *)calloc(kFftSize - kHop, sizeof(float));
    olaAccumulator_ = (float *)calloc(kFftSize, sizeof(float));
    windowSum_ = (float *)calloc(kFftSize, sizeof(float));
    specBuf_.assign(kSpecFloats, 0.0f);
    cache_.assign(kCacheTotal, 0.0f);
    if (!fftPlan_ || !fftIn_ || !fftOut_ || !ifftOut_ || !window_ || !inputHistory_ ||
        !olaAccumulator_ || !windowSum_ || specBuf_.size() != kSpecFloats) {
        model_.close();
        freeBuffers();
        return false;
    }
    make_sqrt_hann(window_, kFftSize);
    for (int i = 0; i < kFftSize; ++i) window_[i] *= window_[i];  // Hann
    return true;
}

void Tse::setReference(const float *ref, size_t n) {
    if (n < (size_t)kFftSize) return;
    size_t nFrames = n / kHop + 1;
    size_t newLen = nFrames * kEnrFloats;
    enrBuf_.assign(newLen, 0.0f);
    enrLen_ = newLen;
    enrFrames_ = nFrames;

    size_t pad = kFftSize / 2;
    std::vector<float> padded(n + pad * 2, 0.0f);
    for (size_t i = 0; i < pad && i + 1 < n; ++i) padded[pad - 1 - i] = ref[i + 1];
    std::memcpy(padded.data() + pad, ref, n * sizeof(float));
    for (size_t i = 0; i < pad && i + 1 < n; ++i) padded[pad + n + i] = ref[n - 2 - i];

    float *realPtr = enrBuf_.data();
    float *imagPtr = enrBuf_.data() + nFrames * kFreq;
    for (size_t tt = 0; tt < nFrames; ++tt) {
        size_t off = tt * kHop;
        for (int i = 0; i < kFftSize; ++i) fftIn_[i] = padded[off + i] * window_[i];
        pffft_transform_ordered(fftPlan_, fftIn_, fftOut_, nullptr, PFFFT_FORWARD);
        realPtr[tt * kFreq] = fftOut_[0];
        imagPtr[tt * kFreq] = 0.0f;
        for (int k = 1; k < kFreq - 1; ++k) {
            int p = 2 + (k - 1) * 2;
            realPtr[tt * kFreq + k] = fftOut_[p];
            imagPtr[tt * kFreq + k] = fftOut_[p + 1];
        }
        realPtr[tt * kFreq + kFreq - 1] = fftOut_[1];
        imagPtr[tt * kFreq + kFreq - 1] = 0.0f;
    }
}

void Tse::setDebugDump(bool enable, const std::string &dir) {
    debugDump_ = enable;
    if (enable && !dir.empty()) {
        debugDir_ = dir;
        if (enrLen_ > 0) {
            char p[1024];
            std::snprintf(p, sizeof(p), "%s/debug_enr_spec.bin", debugDir_.c_str());
            dumpBin(p, enrBuf_.data(), enrLen_);
        }
    }
}

void Tse::runOnnx() {
    size_t nin = model_.nInputs(), nout = model_.nOutputs();
    OrtValue *inputs[8] = {nullptr};
    OrtValue *outputs[8] = {nullptr};
    int64_t ss[4] = {1, 2, 1, kFreq};
    int64_t es[4];
    int64_t cs[1] = {(int64_t)kCacheTotal};
    for (size_t i = 0; i < nin && i < 8; ++i) {
        const char *name = model_.inputName(i);
        if (!name) continue;
        if (std::strcmp(name, "spec_frame") == 0) {
            inputs[i] = model_.makeTensor(specBuf_.data(), kSpecFloats, ss, 4);
        } else if (std::strcmp(name, "enr_spec") == 0) {
            es[0] = 1; es[1] = kEnrCh; es[2] = (int64_t)enrFrames_; es[3] = kFreq;
            inputs[i] = model_.makeTensor(enrBuf_.data(), enrLen_, es, 4);
        } else if (std::strcmp(name, "cache_in") == 0) {
            inputs[i] = model_.makeTensor(cache_.data(), kCacheTotal, cs, 1);
        } else {
            int64_t zeroShape[1] = {0};
            inputs[i] = model_.makeTensor(specBuf_.data(), 0, zeroShape, 1);
        }
    }
    model_.run(nullptr, inputs, nin, nullptr, outputs, nout);
    for (size_t i = 0; i < nout && i < 8; ++i) {
        const char *name = model_.outputName(i);
        if (!name || !outputs[i]) continue;
        float *data = model_.tensorData(outputs[i]);
        size_t total = model_.tensorElems(outputs[i]);
        if (!data) continue;
        if (std::strcmp(name, "enh_frame") == 0) {
            size_t n = total < kSpecFloats ? total : kSpecFloats;
            std::memcpy(specBuf_.data(), data, n * sizeof(float));
        } else if (std::strcmp(name, "cache_out") == 0) {
            size_t n = total < kCacheTotal ? total : kCacheTotal;
            std::memcpy(cache_.data(), data, n * sizeof(float));
        }
    }
    for (size_t i = 0; i < nin && i < 8; ++i) model_.releaseValue(inputs[i]);
    for (size_t i = 0; i < nout && i < 8; ++i) model_.releaseValue(outputs[i]);
}

void Tse::synthOla(const float *in_1024, float *out_1024) {
    fftOut_[0] = specBuf_[0];
    fftOut_[1] = specBuf_[kFreq - 1];
    for (int k = 1; k < kFreq - 1; ++k) {
        int p = 2 + (k - 1) * 2;
        fftOut_[p] = specBuf_[k];
        fftOut_[p + 1] = specBuf_[kFreq + k];
    }
    pffft_transform_ordered(fftPlan_, fftOut_, ifftOut_, nullptr, PFFFT_BACKWARD);
    float scale = 1.0f / kFftSize;
    for (int i = 0; i < kFftSize; ++i) ifftOut_[i] *= scale * window_[i];
    for (int i = 0; i < kFftSize; ++i) olaAccumulator_[i] += ifftOut_[i];
    for (int i = 0; i < kFftSize; ++i) windowSum_[i] += window_[i] * window_[i];
    if (!primed_) {
        primed_ = true;
        std::memcpy(out_1024, in_1024, kHop * sizeof(float));
    } else {
        for (int i = 0; i < kHop; ++i) {
            float norm = windowSum_[i];
            out_1024[i] = (norm > 1e-6f) ? (olaAccumulator_[i] / norm) : olaAccumulator_[i];
        }
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

void Tse::processChunk(const float *in, float *out) {
    if (enrLen_ == 0) {
        std::memcpy(out, in, kHop * sizeof(float));
        return;
    }
    std::memcpy(fftIn_, inputHistory_, (kFftSize - kHop) * sizeof(float));
    std::memcpy(fftIn_ + kFftSize - kHop, in, kHop * sizeof(float));
    std::memmove(inputHistory_, inputHistory_ + kHop, (kFftSize - 2 * kHop) * sizeof(float));
    std::memcpy(inputHistory_ + kFftSize - 2 * kHop, in, kHop * sizeof(float));
    for (int i = 0; i < kFftSize; ++i) fftIn_[i] *= window_[i];
    pffft_transform_ordered(fftPlan_, fftIn_, fftOut_, nullptr, PFFFT_FORWARD);

    float *rp = specBuf_.data();
    float *ip = specBuf_.data() + kFreq;
    rp[0] = fftOut_[0]; ip[0] = 0.0f;
    for (int k = 1; k < kFreq - 1; ++k) {
        int p = 2 + (k - 1) * 2;
        rp[k] = fftOut_[p];
        ip[k] = fftOut_[p + 1];
    }
    rp[kFreq - 1] = fftOut_[1]; ip[kFreq - 1] = 0.0f;

    int fc = frameCount_++;
    if (debugDump_) {
        char path[1024];
        std::snprintf(path, sizeof(path), "%s/f%02d_in.bin", debugDir_.c_str(), fc);
        dumpBin(path, in, kHop);
    }
    runOnnx();
    synthOla(in, out);
    if (debugDump_) {
        char path[1024];
        std::snprintf(path, sizeof(path), "%s/f%02d_out.bin", debugDir_.c_str(), fc);
        dumpBin(path, out, kHop);
    }
}

void Tse::processSpecFreq(const float *in, float *out) {
    std::memcpy(specBuf_.data(), in, kSpecFloats * sizeof(float));
    frameCount_++;
    runOnnx();
    std::memcpy(out, specBuf_.data(), kSpecFloats * sizeof(float));
}

void Tse::processFromSpec(const float *spec, float *out) {
    if (enrLen_ == 0) { std::memset(out, 0, kHop * sizeof(float)); return; }
    std::memcpy(specBuf_.data(), spec, kSpecFloats * sizeof(float));
    frameCount_++;
    runOnnx();
    synthOla(specBuf_.data(), out);
}

void Tse::reset() {
    std::fill(cache_.begin(), cache_.end(), 0.0f);
    std::memset(inputHistory_, 0, (kFftSize - kHop) * sizeof(float));
    std::memset(olaAccumulator_, 0, kFftSize * sizeof(float));
    std::memset(windowSum_, 0, kFftSize * sizeof(float));
    primed_ = false;
    frameCount_ = 0;
}

}  // namespace pv
