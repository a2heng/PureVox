// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "processor.h"

#include <algorithm>
#include <cmath>
#include <cstring>

#include "aec.h"
#include "denoise.h"
#include "resampler.h"
#include "tse.h"

namespace pv {

Processor::Processor()
    : vad_(-45.0f, 20.0f, 250.0f, 48000.0f, 480),
      agc_(-20.0f, 10.0f),
      compressor_(-20.0f, 3.0f, 15.0f, 180.0f, 8.0f, 4.0f, 48000.0f) {}

Processor::~Processor() = default;

bool Processor::init(const std::string &denoisePath, const std::string &tsePath,
                     const std::string &aecPath) {
    preGain_ = 1.0f;
    mode_ = ModeDenoise;
    eqActive_ = false;
    farSampleRate_ = 48000;
    farRmsTarget_ = 0.05f;
    ioInSr_ = ioOutSr_ = 48000;

    if (!tsePath.empty()) tse_ = std::make_unique<Tse>();
    if (!aecPath.empty()) aec_ = std::make_unique<Aec>();
    if (!denoisePath.empty()) denoise_ = std::make_unique<Denoise>();

    if (tse_) { if (!tse_->init(tsePath)) tse_.reset(); }
    if (aec_) { if (!aec_->init(aecPath)) aec_.reset(); }
    if (denoise_) { if (!denoise_->init(denoisePath)) denoise_.reset(); }

    // 报告后端
    if (denoise_) { backendEffective_ = denoise_->backendEffective(); backendReason_ = denoise_->backendReason(); }
    else if (tse_) { backendEffective_ = tse_->backendEffective(); backendReason_ = tse_->backendReason(); }
    else if (aec_) { backendEffective_ = aec_->backendEffective(); backendReason_ = aec_->backendReason(); }
    return true;
}

void Processor::resetAll() {
    stft_.reset();
    if (mode_ == ModeTse && tse_) tse_->reset();
}

int Processor::backendEffective() const { return backendEffective_; }
int Processor::backendReason() const { return backendReason_; }

void Processor::setPreGain(float db) { preGain_ = std::pow(10.0f, db / 20.0f); }

void Processor::setPostGain(float db) { postGain_ = std::pow(10.0f, db / 20.0f); }

void Processor::setMode(int mode) {
    mode_ = mode;
    stft_.reset();
    if (mode == ModeTse && tse_) tse_->reset();
}

int Processor::mode() const { return mode_; }

void Processor::setEqGains(const float *gains, size_t n) {
    eq_.setGains(gains, n);
}

void Processor::setTseEnabled(bool on) {
    if (on) setMode(ModeTse);
    else if (mode_ == ModeTse) setMode(ModeOff);
}

void Processor::setAecEnabled(bool on) {
    if (on) setMode(ModeAec);
    else if (mode_ == ModeAec) setMode(ModeOff);
}

void Processor::setAecFarSampleRate(int sr) {
    if (sr <= 0) sr = 48000;
    farSampleRate_ = sr;
    if (sr != 48000 && aec_) {
        farResampler_ = std::make_unique<Resampler>(Resampler::sincFastest());
        double ratio = 48000.0 / (double)sr;
        std::vector<float> silence(kHopLength, 0.0f);
        farResampler_->run(silence.data(), kHopLength, ratio, false);
    } else {
        farResampler_.reset();
    }
}

void Processor::setAecFarRmsTarget(float rms) {
    farRmsTarget_ = (rms > 0.0f) ? rms : 0.05f;
}

bool Processor::aecAvailable() const { return (bool)aec_; }

void Processor::setTseReference(const float *data, size_t n) {
    if (tse_) tse_->setReference(data, n);
}

bool Processor::tseReferenceLoaded() const { return tse_ && tse_->hasReference(); }
bool Processor::tseAvailable() const { return (bool)tse_; }

void Processor::setVadEnabled(bool on) {
    if (on && !vadEnabled_) vad_.reset();
    vadEnabled_ = on;
}

bool Processor::vadEnabled() const { return vadEnabled_; }
bool Processor::vadActive() const { return vad_.isActive(); }
void Processor::setVadThreshold(float dbfs) { vad_.setThreshold(dbfs); }
float Processor::vadThreshold() const { return vad_.thresholdDbfs(); }

void Processor::setAgcEnabled(bool on, float initial_gain_db) {
    agc_.setEnabled(on, initial_gain_db);
    agcEnabled_ = on;
}

bool Processor::agcEnabled() const { return agcEnabled_; }
bool Processor::agcVoiceActive() const { return agc_.voiceActive(); }
float Processor::agcGainDb() const { return agc_.gainDb(); }
void Processor::setAgcTarget(float dbfs) { agc_.setTarget(dbfs); }
float Processor::agcTarget() const { return agc_.targetDbfs(); }

void Processor::setCompressorEnabled(bool on) {
    compressorEnabled_ = on;
    compressor_.setEnabled(on);
}
bool Processor::compressorEnabled() const { return compressorEnabled_; }
void Processor::setCompressorThreshold(float db) { compressor_.setThreshold(db); }
void Processor::setCompressorRatio(float r) { compressor_.setRatio(r); }
void Processor::setCompressorAttack(float ms) { compressor_.setAttackMs(ms); }
void Processor::setCompressorRelease(float ms) { compressor_.setReleaseMs(ms); }
void Processor::setCompressorMakeup(float db) { compressor_.setMakeup(db); }
void Processor::setCompressorKnee(float db) { compressor_.setKnee(db); }
float Processor::compressorThreshold() const { return compressor_.threshold(); }
float Processor::compressorRatio() const { return compressor_.ratio(); }
float Processor::compressorAttack() const { return compressor_.attackMs(); }
float Processor::compressorRelease() const { return compressor_.releaseMs(); }
float Processor::compressorMakeup() const { return compressor_.makeup(); }
float Processor::compressorKnee() const { return compressor_.knee(); }

void Processor::setRecordingEnabled(bool on) { recordingEnabled_ = on; }
bool Processor::recordingEnabled() const { return recordingEnabled_; }

void Processor::applyPreGain(float *buf) {
    float g = agcEnabled_ ? agc_.tick() : preGain_;
    for (size_t i = 0; i < kHopLength; ++i) buf[i] *= g;
}

void Processor::applyEqClip(float *buf) {
    if (eq_.active()) eq_.apply(buf, kHopLength);
    clip_buffer(buf, kHopLength);
}

void Processor::applyPostGain(float *buf) {
    if (postGain_ == 0.0f) return;  // 0 dB
    for (size_t i = 0; i < kHopLength; ++i) buf[i] *= postGain_;
}

void Processor::measureAgcRms(const float *out) {
    float sq = 0.0f;
    for (size_t i = 0; i < kHopLength; ++i) sq += out[i] * out[i];
    float rms = std::sqrt(sq / (float)kHopLength);
    agc_.updateRms(rms);
}

void Processor::aecStep(const float *buf, const float *far, size_t farN, float *out) {
    if (!aec_ || !far || farN == 0) {
        std::memcpy(out, buf, kHopLength * sizeof(float));
        return;
    }
    if (farResampler_ && farSampleRate_ != 48000) {
        double ratio = 48000.0 / (double)farSampleRate_;
        farResampler_->run(far, farN, ratio, false);
        float far2[kHopLength];
        if (farResampler_->take(far2, kHopLength) == kHopLength) {
            aec_->processFrame(buf, far2, out);
            return;
        }
    } else if (farN >= kHopLength) {
        aec_->processFrame(buf, far, out);
        return;
    }
    std::memcpy(out, buf, kHopLength * sizeof(float));
}

void Processor::tseStep(const float *buf, float *out) {
    if (!tse_ || !tse_->hasReference()) {
        std::memcpy(out, buf, kHopLength * sizeof(float));
        return;
    }
    float spec[Stft::kSpecFloats];
    float denoised[kHopLength];
    if (denoise_) {
        denoise_->processChunk(buf, denoised);
        stft_.forward(denoised, spec);
    } else {
        stft_.forward(buf, spec);
    }
    tse_->processSpecFreq(spec, spec);
    stft_.backward(spec, out);
}

size_t Processor::process(const float *in, size_t n, const float *far, size_t farN, float *out) {
    if (n != kHopLength) return 0;
    float buf[kHopLength];
    std::memcpy(buf, in, kHopLength * sizeof(float));
    applyPreGain(buf);
    applyEqClip(buf);

    float outBuf[kHopLength];
    switch (mode_) {
    case ModeOff:
        std::memcpy(outBuf, buf, kHopLength * sizeof(float));
        break;
    case ModeDenoise:
        if (denoise_) denoise_->processChunk(buf, outBuf);
        else std::memcpy(outBuf, buf, kHopLength * sizeof(float));
        break;
    case ModeAec:
        if (denoise_) {
            float denoised[kHopLength];
            denoise_->processChunk(buf, denoised);
            std::memcpy(buf, denoised, kHopLength * sizeof(float));
        }
        aecStep(buf, far, farN, outBuf);
        break;
    case ModeTse:
        tseStep(buf, outBuf);
        break;
    default:
        std::memcpy(outBuf, buf, kHopLength * sizeof(float));
        break;
    }

    if (compressorEnabled_) compressor_.process(outBuf, kHopLength);
    if (recordingEnabled_ && mode_ != ModeTse) {
        tseRecordingBuffer_.clear();
        tseRecordingBuffer_.insert(tseRecordingBuffer_.end(), outBuf, outBuf + kHopLength);
    } else if (!recordingEnabled_) {
        tseRecordingBuffer_.clear();
    }
    applyPostGain(outBuf);
    clip_buffer(outBuf, kHopLength);
    if (vadEnabled_) vad_.process(outBuf, kHopLength);
    if (agcEnabled_) measureAgcRms(outBuf);
    // 频谱 viz：输入=原始 in × pre 增益（背景基准），输出=最终处理结果
    {
        float vizIn[kHopLength];
        for (size_t i = 0; i < kHopLength; ++i) vizIn[i] = in[i] * preGain_;
        vizIn48k_.insert(vizIn48k_.end(), vizIn, vizIn + kHopLength);
        vizOut48k_.insert(vizOut48k_.end(), outBuf, outBuf + kHopLength);
        // 防止长时间运行缓冲无限增长（窗口隐藏时无人取走）
        const size_t kVizCap = kHopLength * 64;  // 64 帧
        if (vizIn48k_.size() > kVizCap) vizIn48k_.erase(vizIn48k_.begin(), vizIn48k_.begin() + (vizIn48k_.size() - kVizCap));
        if (vizOut48k_.size() > kVizCap) vizOut48k_.erase(vizOut48k_.begin(), vizOut48k_.begin() + (vizOut48k_.size() - kVizCap));
    }
    std::memcpy(out, outBuf, kHopLength * sizeof(float));
    return kHopLength;
}

size_t Processor::processPipeline(const float *in, size_t n, const float *far, size_t farN) {
    if (n == 0) return ioOutAcc_.size();
    ioInAcc_.insert(ioInAcc_.end(), in, in + n);
    while (ioInAcc_.size() >= kHopLength) {
        float chunk[kHopLength];
        std::memcpy(chunk, ioInAcc_.data(), kHopLength * sizeof(float));
        ioInAcc_.erase(ioInAcc_.begin(), ioInAcc_.begin() + kHopLength);
        vizIn48k_.insert(vizIn48k_.end(), chunk, chunk + kHopLength);
        float p[kHopLength];
        process(chunk, kHopLength, far, farN, p);
        vizOut48k_.insert(vizOut48k_.end(), p, p + kHopLength);
        ioOutAcc_.insert(ioOutAcc_.end(), p, p + kHopLength);
    }
    if (ioInAcc_.size() >= kHopLength * 3 / 4) {
        size_t origSz = ioInAcc_.size();
        ioInAcc_.resize(kHopLength, 0.0f);
        vizIn48k_.insert(vizIn48k_.end(), ioInAcc_.begin(), ioInAcc_.begin() + origSz);
        float p[kHopLength];
        process(ioInAcc_.data(), kHopLength, far, farN, p);
        vizOut48k_.insert(vizOut48k_.end(), p, p + kHopLength);
        ioOutAcc_.insert(ioOutAcc_.end(), p, p + kHopLength);
        ioInAcc_.clear();
    }
    return ioOutAcc_.size();
}

size_t Processor::pipelineTake(float *out, size_t cap) {
    size_t n = ioOutAcc_.size() < cap ? ioOutAcc_.size() : cap;
    if (n) std::memcpy(out, ioOutAcc_.data(), n * sizeof(float));
    ioOutAcc_.erase(ioOutAcc_.begin(), ioOutAcc_.begin() + n);
    return n;
}

size_t Processor::vizInputTake(float *out, size_t cap) {
    size_t n = vizIn48k_.size() < cap ? vizIn48k_.size() : cap;
    if (n) std::memcpy(out, vizIn48k_.data(), n * sizeof(float));
    vizIn48k_.erase(vizIn48k_.begin(), vizIn48k_.begin() + n);
    return n;
}

size_t Processor::vizOutputTake(float *out, size_t cap) {
    size_t n = vizOut48k_.size() < cap ? vizOut48k_.size() : cap;
    if (n) std::memcpy(out, vizOut48k_.data(), n * sizeof(float));
    vizOut48k_.erase(vizOut48k_.begin(), vizOut48k_.begin() + n);
    return n;
}

}  // namespace pv
