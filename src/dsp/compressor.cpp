// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "compressor.h"

#include <cmath>

namespace pv {

Compressor::Compressor(float threshold_db, float ratio, float attack_ms, float release_ms,
                       float knee_db, float makeup_db, float fs) {
    thresholdDb_ = threshold_db;
    ratio_ = ratio;
    kneeDb_ = knee_db;
    makeupDb_ = makeup_db;
    enabled_ = false;
    envelope_ = 0.0f;
    gainSmooth_ = 1.0f;
    setDetectorAttack(attack_ms, fs);
    setDetectorRelease(release_ms, fs);
    gainAttackAlpha_ = 1.0f - std::exp(-1.0 / (25.0f * 0.001f * fs));
    gainReleaseAlpha_ = 1.0f - std::exp(-1.0 / (220.0f * 0.001f * fs));
}

void Compressor::setDetectorAttack(float ms, float fs) {
    detectorAttackMs_ = ms;
    detectorAttackAlpha_ = 1.0f - std::exp(-1.0 / (ms * 0.001f * fs));
}

void Compressor::setDetectorRelease(float ms, float fs) {
    detectorReleaseMs_ = ms;
    detectorReleaseAlpha_ = 1.0f - std::exp(-1.0 / (ms * 0.001f * fs));
}

void Compressor::setThreshold(float db) { thresholdDb_ = db; }
void Compressor::setRatio(float r) { ratio_ = (r < 1.0f) ? 1.0f : r; }
void Compressor::setAttackMs(float ms) { setDetectorAttack(ms, 48000.0f); }
void Compressor::setReleaseMs(float ms) { setDetectorRelease(ms, 48000.0f); }
void Compressor::setKnee(float db) { kneeDb_ = db; }
void Compressor::setMakeup(float db) { makeupDb_ = db; }
void Compressor::setEnabled(bool on) { enabled_ = on; }
void Compressor::reset() { envelope_ = 0.0f; gainSmooth_ = 1.0f; }

void Compressor::process(float *data, size_t len) {
    if (!enabled_) return;
    for (size_t i = 0; i < len; ++i) {
        float x2 = data[i] * data[i];
        float alpha = (x2 > envelope_) ? detectorAttackAlpha_ : detectorReleaseAlpha_;
        envelope_ += alpha * (x2 - envelope_);
        float env_db = (envelope_ > 1e-12f) ? 10.0f * std::log10(envelope_) : -120.0f;
        float over = env_db - thresholdDb_;
        float gr_db = 0.0f;
        if (over > 0.0f) {
            if (kneeDb_ > 0.0f && over < kneeDb_) {
                float t = over / kneeDb_;
                gr_db = (1.0f / ratio_ - 1.0f) * over * t * 0.5f;
            } else {
                gr_db = (1.0f / ratio_ - 1.0f) * over;
            }
        }
        float gain_target = std::pow(10.0f, (gr_db + makeupDb_) / 20.0f);
        if (gain_target < gainSmooth_) {
            gainSmooth_ = gainAttackAlpha_ * gain_target + (1.0f - gainAttackAlpha_) * gainSmooth_;
        } else {
            gainSmooth_ = gainReleaseAlpha_ * gain_target + (1.0f - gainReleaseAlpha_) * gainSmooth_;
        }
        data[i] = std::tanh(data[i] * gainSmooth_);
    }
}

}  // namespace pv
