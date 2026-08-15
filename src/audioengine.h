// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_AUDIOENGINE_H
#define PUREVOX_AUDIOENGINE_H

#include <QString>

#include <QVector>

#include <string>

struct AudioProcessor;

// aimic C 音频引擎的 C++ RAII 封装
class AudioEngine {
public:
    // 模式（与 aimic.h 一致）
    static constexpr int ModeOff = 0;      // 直通
    static constexpr int ModeDenoise = 1;  // 降噪
    static constexpr int ModeAec = 2;      // AEC
    static constexpr int ModeTse = 3;      // TSE

    AudioEngine();
    ~AudioEngine();

    AudioEngine(const AudioEngine &) = delete;
    AudioEngine &operator=(const AudioEngine &) = delete;

    bool init(const QString &denoiseModel, const QString &tseModel, const QString &aecModel,
              QString *errMsg);

    bool ready() const { return proc_ != nullptr; }
    AudioProcessor *raw() { return proc_; }

    int backendEffective() const;
    int backendReason() const;

    void setMode(int mode);
    int mode() const;
    void setPreGain(double db);
    void applyEqGains(const QVector<double> &gains);
    void setCompressorEnabled(bool on);
    void setAgcEnabled(bool on, double initialGainDb);
    void setVadEnabled(bool on);
    void setVadThreshold(double dbfs);
    void setAgcTarget(double dbfs);
    void setAecEnabled(bool on);
    void setTseEnabled(bool on);

    // process: in(n) -> out(>=n)，返回输出采样数
    size_t process(const float *in, size_t n, const float *far, size_t farN, float *out);

private:
    AudioProcessor *proc_ = nullptr;
    QString lastError_;
};

#endif // PUREVOX_AUDIOENGINE_H
