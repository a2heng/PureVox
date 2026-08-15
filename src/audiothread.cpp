// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "audiothread.h"

#include <pvbridge.h>

#include <QVector>
#include <algorithm>

namespace {
constexpr size_t kHop = 1024;
}

AudioThread::AudioThread(AudioEngine *engine, const QString &inputNode,
                         const QString &outputNode, const QString &monitorNode, int mode,
                         QObject *parent)
    : QThread(parent), engine_(engine), inputNode_(inputNode), outputNode_(outputNode),
      monitorNode_(monitorNode), mode_(mode) {}

AudioThread::~AudioThread() { stop(); }

void AudioThread::stop() {
    running_.store(false);
    if (isRunning()) wait();
}

void AudioThread::run() {
    bridge_ = pvb_new();
    if (!bridge_) {
        emit errorOccurred("PipeWire 桥初始化失败");
        return;
    }
    if (!pvb_open(bridge_, inputNode_.toUtf8().constData(), outputNode_.toUtf8().constData(),
                  monitorNode_.toUtf8().constData())) {
        QString err = QString::fromUtf8(pvb_last_error(bridge_));
        emit errorOccurred(QString("打开音频流失败: %1").arg(err));
        pvb_free(bridge_);
        bridge_ = nullptr;
        return;
    }
    if (mode_ == AudioEngine::ModeAec) {
        engine_->setAecEnabled(true);
        pvb_set_far(bridge_, outputNode_.toUtf8().constData(), 1);
    }

    running_.store(true);
    engine_->setMode(mode_);

    QVector<float> in(kHop), out(kHop);
    QVector<float> farBuf(kHop);
    size_t max = 0;
    while (running_.load()) {
        size_t n = pvb_read(bridge_, in.data(), kHop);
        if (n == 0) {
            msleep(2);
            continue;
        }
        size_t farN = 0;
        if (mode_ == AudioEngine::ModeAec) {
            farN = pvb_read_far(bridge_, farBuf.data(), kHop);
        }
        size_t on = engine_->process(in.data(), n, farN ? farBuf.data() : nullptr, farN,
                                     out.data());
        if (on > 0) pvb_write(bridge_, out.data(), on);
        // 降噪输出峰值 → VU
        float peak = 0;
        for (size_t i = 0; i < on; ++i) peak = std::max(peak, std::fabs(out[i]));
        double db = peak > 1e-10 ? 20.0 * std::log10(peak) : -60.0;
        if (on > max) max = on;
        emit levelUpdated(db);
    }

    if (mode_ == AudioEngine::ModeAec) pvb_set_far(bridge_, outputNode_.toUtf8().constData(), 0);
    pvb_close(bridge_);
    pvb_free(bridge_);
    bridge_ = nullptr;
}
