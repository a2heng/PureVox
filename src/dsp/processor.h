// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_PROCESSOR_H
#define PUREVOX_DSP_PROCESSOR_H

#include <cstddef>
#include <memory>
#include <string>
#include <vector>

#include "agc.h"
#include "compressor.h"
#include "dsp_common.h"
#include "eq.h"
#include "stft.h"
#include "vad.h"

namespace pv {

class Denoise;
class Tse;
class Aec;
class Resampler;

// 统一处理链：pre_gain → EQ → clip → [passthrough|denoise|aec|tse] → compressor
// → clip → VAD → AGC
class Processor {
public:
    static constexpr int ModeOff = 0;      // 直通
    static constexpr int ModeDenoise = 1;  // 降噪
    static constexpr int ModeAec = 2;      // AEC
    static constexpr int ModeTse = 3;      // TSE

    Processor();
    ~Processor();
    Processor(const Processor &) = delete;
    Processor &operator=(const Processor &) = delete;

    bool init(const std::string &denoisePath, const std::string &tsePath,
              const std::string &aecPath);
    void resetAll();

    int backendEffective() const;
    int backendReason() const;

    // 参数
    void setPreGain(float db);   // pre 增益（链首，输入侧）
    void setPostGain(float db);  // post 增益（链尾，输出侧）
    float postGain() const { return postGain_; }
    void setMode(int mode);
    int mode() const;
    void setEqGains(const float *gains, size_t n);
    static int eqBandCount() { return Equalizer::bandCount(); }
    static void eqFreqs(float *out) { Equalizer::freqs(out); }

    void setTseEnabled(bool on);
    void setAecEnabled(bool on);
    void setAecFarSampleRate(int sr);
    int aecFarSampleRate() const { return farSampleRate_; }
    void setAecFarRmsTarget(float rms);
    float aecFarRmsTarget() const { return farRmsTarget_; }
    bool aecAvailable() const;

    void setTseReference(const float *data, size_t n);
    bool tseReferenceLoaded() const;
    bool tseAvailable() const;

    void setVadEnabled(bool on);
    bool vadEnabled() const;
    bool vadActive() const;
    void setVadThreshold(float dbfs);
    float vadThreshold() const;

    void setAgcEnabled(bool on, float initial_gain_db);
    bool agcEnabled() const;
    bool agcVoiceActive() const;
    float agcGainDb() const;
    void setAgcTarget(float dbfs);
    float agcTarget() const;

    void setCompressorEnabled(bool on);
    bool compressorEnabled() const;
    void setCompressorThreshold(float db);
    void setCompressorRatio(float r);
    void setCompressorAttack(float ms);
    void setCompressorRelease(float ms);
    void setCompressorMakeup(float db);
    void setCompressorKnee(float db);
    float compressorThreshold() const;
    float compressorRatio() const;
    float compressorAttack() const;
    float compressorRelease() const;
    float compressorMakeup() const;
    float compressorKnee() const;

    void setRecordingEnabled(bool on);
    bool recordingEnabled() const;

    // 处理：输入 1024 采样 → 输出（≥1024），far/farN 可空用于 AEC
    size_t process(const float *in, size_t n, const float *far, size_t farN, float *out);

    // pipeline：任意长输入，输出累积内部缓冲
    size_t processPipeline(const float *in, size_t n, const float *far, size_t farN);
    size_t pipelineTake(float *out, size_t cap);
    size_t vizInputTake(float *out, size_t cap);
    size_t vizOutputTake(float *out, size_t cap);

private:
    void applyPreGain(float *buf);
    void applyPostGain(float *buf);
    void applyEqClip(float *buf);
    void measureAgcRms(const float *out);
    void aecStep(const float *buf, const float *far, size_t farN, float *out);
    void tseStep(const float *buf, float *out);

    float preGain_ = 0;   // pre 增益（链首）
    float postGain_ = 0;  // post 增益（链尾）
    Equalizer eq_;
    int mode_ = ModeDenoise;
    bool eqActive_ = false;
    std::unique_ptr<Denoise> denoise_;
    std::unique_ptr<Tse> tse_;
    std::unique_ptr<Aec> aec_;
    Stft stft_;
    int farSampleRate_ = 48000;
    std::unique_ptr<Resampler> farResampler_;
    float farRmsTarget_ = 0.05f;
    int ioInSr_ = 48000, ioOutSr_ = 48000;
    std::vector<float> ioInAcc_, ioOutAcc_, vizIn48k_, vizOut48k_;
    Vad vad_;
    bool vadEnabled_ = false;
    Agc agc_;
    bool agcEnabled_ = false;
    Compressor compressor_;
    bool compressorEnabled_ = false;
    std::vector<float> tseRecordingBuffer_;
    bool recordingEnabled_ = false;
    int backendEffective_ = 0;
    int backendReason_ = 0;
};

}  // namespace pv

#endif // PUREVOX_DSP_PROCESSOR_H
