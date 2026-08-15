// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "alsabackend.h"

#include <alsbridge.h>

AlsaBackend::AlsaBackend() { bridge_ = als_new(); }

AlsaBackend::~AlsaBackend() {
    close();
    if (bridge_) {
        als_free(bridge_);
        bridge_ = nullptr;
    }
}

bool AlsaBackend::open(const QString &input, const QString &output, const QString &monitor) {
    if (!bridge_) return false;
    QByteArray in = input.toUtf8(), out = output.toUtf8(), mon = monitor.toUtf8();
    return als_open(bridge_, in.constData(), out.constData(), mon.constData()) != 0;
}

void AlsaBackend::close() {
    if (bridge_) als_close(bridge_);
}

bool AlsaBackend::active() const { return bridge_ && als_active(bridge_); }

QString AlsaBackend::lastError() const {
    if (!bridge_) return QStringLiteral("ALSA 桥未初始化");
    return QString::fromUtf8(als_last_error(bridge_));
}

uint32_t AlsaBackend::sampleRate() const {
    return bridge_ ? als_sample_rate(bridge_) : 0;
}

size_t AlsaBackend::read(float *out, size_t n) {
    return bridge_ ? als_read(bridge_, out, n) : 0;
}

size_t AlsaBackend::readFar(float *out, size_t n) {
    return bridge_ ? als_read_far(bridge_, out, n) : 0;
}

void AlsaBackend::write(const float *data, size_t n) {
    if (bridge_) als_write(bridge_, data, n);
}

bool AlsaBackend::setFar(const QString &sink, bool on) {
    QByteArray s = sink.toUtf8();
    return bridge_ && als_set_far(bridge_, s.constData(), on ? 1 : 0);
}

bool AlsaBackend::setMonitor(const QString &name, bool on) {
    QByteArray s = name.toUtf8();
    return bridge_ && als_set_monitor(bridge_, s.constData(), on ? 1 : 0);
}
