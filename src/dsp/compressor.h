// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_COMPRESSOR_H
#define PUREVOX_DSP_COMPRESSOR_H

#include <cstddef>

namespace pv {

// 动态范围压缩器
class Compressor {
public:
    Compressor(float threshold_db, float ratio, float attack_ms, float release_ms,
               float knee_db, float makeup_db, float fs);
    void setThreshold(float db);
    float threshold() const { return thresholdDb_; }
    void setRatio(float r);
    float ratio() const { return ratio_; }
    void setAttackMs(float ms);
    float attackMs() const { return detectorAttackMs_; }
    void setReleaseMs(float ms);
    float releaseMs() const { return detectorReleaseMs_; }
    void setKnee(float db);
    float knee() const { return kneeDb_; }
    void setMakeup(float db);
    float makeup() const { return makeupDb_; }
    void setEnabled(bool on);
    bool isEnabled() const { return enabled_; }
    void reset();
    void process(float *data, size_t len);

private:
    void setDetectorAttack(float ms, float fs);
    void setDetectorRelease(float ms, float fs);

    float thresholdDb_ = 0;
    float ratio_ = 0;
    float kneeDb_ = 0;
    float makeupDb_ = 0;
    float detectorAttackAlpha_ = 0;
    float detectorAttackMs_ = 0;
    float detectorReleaseAlpha_ = 0;
    float detectorReleaseMs_ = 0;
    float gainAttackAlpha_ = 0;
    float gainReleaseAlpha_ = 0;
    bool enabled_ = false;
    float envelope_ = 0.0f;
    float gainSmooth_ = 1.0f;
};

}  // namespace pv

#endif // PUREVOX_DSP_COMPRESSOR_H
