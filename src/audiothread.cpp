// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "audiothread.h"

#include "audiobackend.h"

#include <algorithm>
#include <cmath>

namespace {
constexpr size_t kHop = 1024;
}

AudioThread::AudioThread(AudioEngine *engine, AudioBackend *backend, const QString &input,
                         const QString &output, const QString &monitor, int mode,
                         QObject *parent)
    : QThread(parent), engine_(engine), backend_(backend), input_(input), output_(output),
      monitor_(monitor), mode_(mode) {}

AudioThread::~AudioThread() {
    stop();
    delete backend_;
    backend_ = nullptr;
}

void AudioThread::stop() {
    running_.store(false);
    if (isRunning()) wait();
}

void AudioThread::run() {
    if (!backend_->open(input_, output_, monitor_)) {
        emit errorOccurred(QStringLiteral("打开音频流失败: %1").arg(backend_->lastError()));
        return;
    }
    if (mode_ == AudioEngine::ModeAec) {
        engine_->setAecEnabled(true);
        backend_->setFar(output_, true);
    }

    running_.store(true);
    engine_->setMode(mode_);

    QVector<float> in(kHop), out(kHop);
    QVector<float> farBuf(kHop);
    while (running_.load()) {
        size_t n = backend_->read(in.data(), kHop);
        if (n == 0) {
            msleep(2);
            continue;
        }
        size_t farN = 0;
        if (mode_ == AudioEngine::ModeAec) {
            farN = backend_->readFar(farBuf.data(), kHop);
        }
        size_t on = engine_->process(in.data(), n, farN ? farBuf.data() : nullptr, farN,
                                     out.data());
        if (on > 0) backend_->write(out.data(), on);
        // 降噪输出峰值 → VU
        float peak = 0;
        for (size_t i = 0; i < on; ++i) peak = std::max(peak, std::fabs(out[i]));
        double db = peak > 1e-10 ? 20.0 * std::log10(peak) : -60.0;
        emit levelUpdated(db);
        // 频谱数据：从引擎 viz 缓冲取（线程安全）
        {
            QVector<float> inFrame(kHop), outFrame(kHop);
            size_t g1 = engine_->vizInputTake(inFrame.data(), kHop);
            size_t g2 = engine_->vizOutputTake(outFrame.data(), kHop);
            if (g1 > 0 || g2 > 0) {
                inFrame.resize(g1);
                outFrame.resize(g2);
                emit spectrumData(inFrame, outFrame);
            }
        }
    }

    if (mode_ == AudioEngine::ModeAec) backend_->setFar("", false);
    backend_->close();
}
