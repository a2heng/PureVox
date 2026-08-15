// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_AGC_H
#define PUREVOX_DSP_AGC_H

#include <cmath>

namespace pv {

// 自动增益控制（AGC）：EMA RMS → 目标增益平滑
class Agc {
public:
    explicit Agc(float target_dbfs, float call_interval_ms);
    void reset();
    void updateRms(float rms_linear);
    float tick();
    float gainLinear() const { return smoothedGainLinear_; }
    float gainDb() const {
        return 20.0f * std::log10(smoothedGainLinear_ > 0.0f ? smoothedGainLinear_ : 1e-10f);
    }
    bool voiceActive() const { return voiceActive_; }
    void setEnabled(bool on, float initial_gain_db);
    bool enabled() const { return enabled_; }
    void setTarget(float dbfs);
    float targetDbfs() const { return targetDbfs_; }

private:
    float targetDbfs_ = 0;
    float targetLinear_ = 0;
    float gainMinLinear_ = 0;
    float gainMaxLinear_ = 0;
    float silenceThrLinear_ = 0;
    float rmsFloorLinear_ = 0;
    float attackAlpha_ = 0;
    float releaseAlpha_ = 0;
    float decayFactor_ = 0;
    float deadZone_ = 0;
    float rmsAlpha_ = 0;
    float smoothedGainLinear_ = 1.0f;
    float rmsEma_ = 0.0f;
    bool initialized_ = false;
    bool enabled_ = false;
    bool voiceActive_ = false;
    int silentTailCount_ = 0;
};

}  // namespace pv

#endif // PUREVOX_DSP_AGC_H
