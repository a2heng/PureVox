// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_AUDIOTHREAD_H
#define PUREVOX_AUDIOTHREAD_H

#include <QThread>
#include <QString>

#include <atomic>

#include "audioengine.h"

class VUBar;
class PwBridge;

// 音频处理线程：从 PipeWire 读 → 引擎降噪 → 写回 PipeWire，并回调 VU
class AudioThread : public QThread {
    Q_OBJECT

public:
    AudioThread(AudioEngine *engine, const QString &inputNode, const QString &outputNode,
                const QString &monitorNode, int mode, QObject *parent = nullptr);
    ~AudioThread() override;

    void stop();
    bool running() const { return running_.load(); }

signals:
    void levelUpdated(double db);
    void errorOccurred(const QString &msg);

protected:
    void run() override;

private:
    AudioEngine *engine_;
    QString inputNode_;
    QString outputNode_;
    QString monitorNode_;
    int mode_;
    std::atomic<bool> running_{false};
    PwBridge *bridge_ = nullptr;
};

#endif // PUREVOX_AUDIOTHREAD_H
