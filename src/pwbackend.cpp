// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "pwbackend.h"

#include <pvbridge.h>

PwBackend::PwBackend() { bridge_ = pvb_new(); }

PwBackend::~PwBackend() {
    close();
    if (bridge_) {
        pvb_free(bridge_);
        bridge_ = nullptr;
    }
}

bool PwBackend::open(const QString &input, const QString &output, const QString &monitor) {
    if (!bridge_) return false;
    QByteArray in = input.toUtf8(), out = output.toUtf8(), mon = monitor.toUtf8();
    return pvb_open(bridge_, in.constData(), out.constData(), mon.constData()) != 0;
}

void PwBackend::close() {
    if (bridge_) pvb_close(bridge_);
}

bool PwBackend::active() const { return bridge_ && pvb_active(bridge_); }

QString PwBackend::lastError() const {
    if (!bridge_) return QStringLiteral("PipeWire 桥未初始化");
    return QString::fromUtf8(pvb_last_error(bridge_));
}

uint32_t PwBackend::sampleRate() const {
    return bridge_ ? pvb_sample_rate(bridge_) : 0;
}

size_t PwBackend::read(float *out, size_t n) {
    return bridge_ ? pvb_read(bridge_, out, n) : 0;
}

size_t PwBackend::readFar(float *out, size_t n) {
    return bridge_ ? pvb_read_far(bridge_, out, n) : 0;
}

void PwBackend::write(const float *data, size_t n) {
    if (bridge_) pvb_write(bridge_, data, n);
}

bool PwBackend::setFar(const QString &sink, bool on) {
    QByteArray s = sink.toUtf8();
    return bridge_ && pvb_set_far(bridge_, s.constData(), on ? 1 : 0);
}

bool PwBackend::setMonitor(const QString &name, bool on) {
    QByteArray s = name.toUtf8();
    return bridge_ && pvb_set_monitor(bridge_, s.constData(), on ? 1 : 0);
}
