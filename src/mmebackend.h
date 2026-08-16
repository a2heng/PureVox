// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_MMEBACKEND_H
#define PUREVOX_MMEBACKEND_H

#include <QtGlobal>
#include "audiobackend.h"

#ifdef Q_OS_WIN

struct MmeCaptureState;
struct MmeRenderState;

// MME 音频后端（Windows）：waveIn 采集 + waveOut 播放，48kHz 单声道 float。
// 作为 WASAPI 打不开设备时的备选接口。
class MmeBackend : public AudioBackend {
public:
    MmeBackend();
    ~MmeBackend() override;

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

private:
    MmeCaptureState *cap_ = nullptr;
    MmeRenderState *rend_ = nullptr;
    bool active_ = false;
    QString lastError_;
    uint32_t sampleRate_ = 48000;
};

#endif // Q_OS_WIN

#endif // PUREVOX_MMEBACKEND_H
