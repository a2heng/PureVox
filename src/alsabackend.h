// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_ALSABACKEND_H
#define PUREVOX_ALSABACKEND_H

#include "audiobackend.h"

struct AlsaBridge;

// ALSA 后端（原生 libasound）
class AlsaBackend : public AudioBackend {
public:
    AlsaBackend();
    ~AlsaBackend() override;

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
    AlsaBridge *bridge_ = nullptr;
};

#endif // PUREVOX_ALSABACKEND_H
