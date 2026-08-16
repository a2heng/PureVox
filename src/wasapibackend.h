// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_WASAPIBACKEND_H
#define PUREVOX_WASAPIBACKEND_H

// 需在检查 Q_OS_WIN 前定义它（audiobackend.h → QString → QtGlobal）
#include <QtGlobal>
#include "audiobackend.h"

#ifdef Q_OS_WIN

struct WasapiCaptureState;
struct WasapiRenderState;

// WASAPI 音频后端（Windows）：输入采集 + 输出播放，48kHz 单声道 float
class WasapiBackend : public AudioBackend {
public:
    WasapiBackend();
    ~WasapiBackend() override;

    bool open(const QString &input, const QString &output, const QString &monitor) override;
    void close() override;
    bool active() const override;
    QString lastError() const override;
    uint32_t sampleRate() const override;
    size_t read(float *out, size_t n) override;
    size_t readFar(float *out, size_t n) override;
    void write(const float *data, size_t n) override;
    bool setFar(const QString &sink, bool on) override;
    bool setMonitor(const QString &name, bool on) override;

    // 启动前 48k 检测：试探设备能否以 48kHz 打开（capture=true 采集，false 播放）
    static bool check48k(const QString &deviceName, bool capture);

private:
    WasapiCaptureState *cap_ = nullptr;
    WasapiRenderState *rend_ = nullptr;
    bool active_ = false;
    QString lastError_;
    uint32_t sampleRate_ = 48000;
    QString inputId_;
    QString outputId_;
};

#endif // Q_OS_WIN

#endif // PUREVOX_WASAPIBACKEND_H
