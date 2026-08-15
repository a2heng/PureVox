// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "agc.h"

#include <cmath>

namespace pv {

namespace {
constexpr int kSilentTailFrames = 15;
}

Agc::Agc(float target_dbfs, float call_interval_ms) {
    targetDbfs_ = target_dbfs;
    targetLinear_ = std::pow(10.0f, target_dbfs / 20.0f);
    gainMinLinear_ = std::pow(10.0f, -30.0f / 20.0f);
    gainMaxLinear_ = std::pow(10.0f, 30.0f / 20.0f);
    silenceThrLinear_ = std::pow(10.0f, -45.0f / 20.0f);
    rmsFloorLinear_ = std::pow(10.0f, -60.0f / 20.0f);
    smoothedGainLinear_ = 1.0f;
    initialized_ = false;
    enabled_ = false;
    voiceActive_ = false;
    silentTailCount_ = 0;
    float dt = call_interval_ms / 1000.0f;
    attackAlpha_ = 1.0f - std::exp(-dt / 0.010f);
    releaseAlpha_ = 1.0f - std::exp(-dt / 0.150f);
    decayFactor_ = std::pow(0.5f, dt);
    deadZone_ = std::pow(10.0f, 0.5f / 20.0f);
    rmsAlpha_ = 1.0f - std::exp(-dt / 0.200f);
}

void Agc::reset() {
    smoothedGainLinear_ = 1.0f;
    rmsEma_ = 0.0f;
    initialized_ = false;
    voiceActive_ = false;
    silentTailCount_ = 0;
}

void Agc::updateRms(float rms_linear) {
    bool isVoice = rms_linear > silenceThrLinear_;
    if (isVoice) { silentTailCount_ = 0; voiceActive_ = true; }
    else {
        silentTailCount_++;
        if (silentTailCount_ >= kSilentTailFrames) voiceActive_ = false;
    }
    if (!voiceActive_) return;
    if (rms_linear <= silenceThrLinear_) return;
    if (rmsEma_ == 0.0f) rmsEma_ = rms_linear;
    else rmsEma_ = rmsAlpha_ * rms_linear + (1.0f - rmsAlpha_) * rmsEma_;
}

float Agc::tick() {
    if (!enabled_) return 1.0f;
    if (!voiceActive_) {
        if (smoothedGainLinear_ > 1.0f) {
            smoothedGainLinear_ *= decayFactor_;
            if (smoothedGainLinear_ < 1.0f) smoothedGainLinear_ = 1.0f;
        }
        return smoothedGainLinear_;
    }
    if (rmsEma_ == 0.0f) return smoothedGainLinear_;
    float rms = rmsEma_;
    if (rms < rmsFloorLinear_) rms = rmsFloorLinear_;
    float targetGain = targetLinear_ / rms;
    if (targetGain < gainMinLinear_) targetGain = gainMinLinear_;
    if (targetGain > gainMaxLinear_) targetGain = gainMaxLinear_;
    if (!initialized_) {
        initialized_ = true;
        smoothedGainLinear_ = targetGain;
    } else {
        float ratio = targetGain / smoothedGainLinear_;
        if (ratio > (1.0f / deadZone_) && ratio < deadZone_) return smoothedGainLinear_;
        float alpha = (targetGain < smoothedGainLinear_) ? attackAlpha_ : releaseAlpha_;
        smoothedGainLinear_ = alpha * targetGain + (1.0f - alpha) * smoothedGainLinear_;
    }
    return smoothedGainLinear_;
}

void Agc::setEnabled(bool on, float initial_gain_db) {
    if (on && !enabled_) {
        smoothedGainLinear_ = std::pow(10.0f, initial_gain_db / 20.0f);
        rmsEma_ = 0.0f;
        initialized_ = false;
        voiceActive_ = false;
        silentTailCount_ = 0;
    }
    enabled_ = on;
}

void Agc::setTarget(float dbfs) {
    targetDbfs_ = dbfs;
    targetLinear_ = std::pow(10.0f, dbfs / 20.0f);
}

}  // namespace pv
