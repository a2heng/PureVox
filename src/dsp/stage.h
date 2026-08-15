// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_STAGE_H
#define PUREVOX_DSP_STAGE_H

#include <cmath>
#include <cstddef>
#include <vector>

#include "agc.h"
#include "compressor.h"
#include "eq.h"
#include "vad.h"

namespace pv {

// 处理阶段抽象：就地处理一段浮点缓冲（单声道 48kHz 帧）。
// 各阶段可独立测试、独立复用，由 Processor 按顺序组合成处理链。
// 目前 Processor 通过内部成员(Vad/Agc/Compressor/Equalizer)实现同等的阶段
// 组合；本文件提供更细粒度的统一 Stage 抽象，供独立复用/测试。
class Stage {
public:
    virtual ~Stage() = default;
    // 就地处理 in/out 缓冲（长度 n）
    virtual void process(float *buf, size_t n) = 0;
    // 阶段是否启用（决定是否进入链）
    virtual bool enabled() const = 0;
};

// pre 增益（链首输入增益）
class PreGainStage : public Stage {
public:
    explicit PreGainStage(float db = 0.0f) { setGain(db); }
    void setGain(float db) { gainLinear_ = std::pow(10.0f, db / 20.0f); }
    void process(float *buf, size_t n) override {
        for (size_t i = 0; i < n; ++i) buf[i] *= gainLinear_;
    }
    bool enabled() const override { return true; }

private:
    float gainLinear_ = 1.0f;
};

// post 增益（链尾输出增益）
class PostGainStage : public Stage {
public:
    explicit PostGainStage(float db = 0.0f) { setGain(db); }
    void setGain(float db) { gainLinear_ = std::pow(10.0f, db / 20.0f); }
    void process(float *buf, size_t n) override {
        for (size_t i = 0; i < n; ++i) buf[i] *= gainLinear_;
    }
    bool enabled() const override { return true; }

private:
    float gainLinear_ = 1.0f;
};

// EQ 阶段（包装 Equalizer）
class EqStage : public Stage {
public:
    void setGains(const float *gains, size_t n) { eq_.setGains(gains, n); }
    void process(float *buf, size_t n) override { eq_.apply(buf, n); }
    bool enabled() const override { return eq_.active(); }

private:
    Equalizer eq_;
};

// 压缩器阶段（包装 Compressor）
class CompressorStage : public Stage {
public:
    void setEnabled(bool on) { on_ = on; comp_.setEnabled(on); }
    void process(float *buf, size_t n) override { comp_.process(buf, n); }
    bool enabled() const override { return on_; }

private:
    Compressor comp_{-20.0f, 3.0f, 15.0f, 180.0f, 8.0f, 4.0f, 48000.0f};
    bool on_ = false;
};

// VAD 阶段（包装 Vad）
class VadStage : public Stage {
public:
    void setEnabled(bool on) { on_ = on; if (on) vad_.reset(); }
    void setThreshold(float db) { vad_.setThreshold(db); }
    void process(float *buf, size_t n) override { vad_.process(buf, n); }
    bool enabled() const override { return on_; }

private:
    Vad vad_{-45.0f, 20.0f, 250.0f, 48000.0f, 480};
    bool on_ = false;
};

// AGC 阶段（包装 Agc）
class AgcStage : public Stage {
public:
    void setEnabled(bool on, float initialDb) { agc_.setEnabled(on, initialDb); on_ = on; }
    void setTarget(float db) { agc_.setTarget(db); }
    float gainDb() const { return agc_.gainDb(); }
    void updateRms(float rms) { agc_.updateRms(rms); }
    // AGC 需要先 tick 得增益再应用（与 pre 不同）
    void process(float *buf, size_t n) override {
        float g = agc_.tick();
        for (size_t i = 0; i < n; ++i) buf[i] *= g;
    }
    bool enabled() const override { return on_; }

private:
    Agc agc_{-20.0f, 10.0f};
    bool on_ = false;
};

}  // namespace pv

#endif // PUREVOX_DSP_STAGE_H
