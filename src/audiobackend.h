// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_AUDIOBACKEND_H
#define PUREVOX_AUDIOBACKEND_H

#include <QString>

// 音频后端抽象：输入/输出/监听/far 采集与读写
class AudioBackend {
public:
    virtual ~AudioBackend() = default;

    virtual bool open(const QString &input, const QString &output, const QString &monitor) = 0;
    virtual void close() = 0;
    virtual bool active() const = 0;
    virtual QString lastError() const = 0;
    virtual uint32_t sampleRate() const = 0;
    virtual size_t read(float *out, size_t n) = 0;
    virtual size_t readFar(float *out, size_t n) = 0;
    virtual void write(const float *data, size_t n) = 0;
    virtual bool setFar(const QString &sink, bool on) = 0;
    virtual bool setMonitor(const QString &name, bool on) = 0;
};

#endif // PUREVOX_AUDIOBACKEND_H
