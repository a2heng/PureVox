// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_AUDIOENGINE_H
#define PUREVOX_AUDIOENGINE_H

#include <QRecursiveMutex>
#include <QString>

#include <QVector>

#include <memory>

#include "dsp/processor.h"

// 音频处理引擎（模块化 C++ 处理链封装）
class AudioEngine {
public:
    // 模式（与 pv::Processor 一致）
    static constexpr int ModeOff = pv::Processor::ModeOff;
    static constexpr int ModeDenoise = pv::Processor::ModeDenoise;
    static constexpr int ModeAec = pv::Processor::ModeAec;
    static constexpr int ModeTse = pv::Processor::ModeTse;

    AudioEngine();
    ~AudioEngine();

    AudioEngine(const AudioEngine &) = delete;
    AudioEngine &operator=(const AudioEngine &) = delete;

    bool init(const QString &denoiseModel, const QString &tseModel, const QString &aecModel,
              QString *errMsg);
    // 销毁处理器（停止音频处理时调用；下次启动 init() 重建）
    void cleanup();
    bool ready() const { return proc_ != nullptr; }
    pv::Processor *raw() { return proc_.get(); }

    int backendEffective() const;
    int backendReason() const;

    void setMode(int mode);
    int mode() const;
    void setPreGain(double db);   // pre 增益（链首）
    void setPostGain(double db);  // post 增益（链尾）
    void applyEqGains(const QVector<double> &gains);
    void setCompressorEnabled(bool on);
    void setAgcEnabled(bool on, double initialGainDb);
    void setVadEnabled(bool on);
    void setVadThreshold(double dbfs);
    void setAgcTarget(double dbfs);
    void setAecEnabled(bool on);
    void setTseEnabled(bool on);
    // 设置 TSE 参考语音（录音完成后）
    void setTseReference(const float *data, size_t n);
    bool tseReferenceLoaded() const;
    // 把 UI 全部参数重放到当前 processor（重建 processor 后调用）
    void replayParams(int mode, double preGain, double postGain,
                      const QVector<double> &eqGains, bool compOn, bool agcOn,
                      bool vadOn, bool monitorOn);

    // process: in(n) -> out(>=n)，返回输出采样数
    size_t process(const float *in, size_t n, const float *far, size_t farN, float *out);
    // 频谱 viz 取数（线程安全）
    size_t vizInputTake(float *out, size_t cap);
    size_t vizOutputTake(float *out, size_t cap);

private:
    std::unique_ptr<pv::Processor> proc_;
    mutable QRecursiveMutex mutex_;  // 保护 proc_ 跨线程访问
    QString lastError_;
};

#endif // PUREVOX_AUDIOENGINE_H
