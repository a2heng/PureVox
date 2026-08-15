// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_AUDIOTHREAD_H
#define PUREVOX_AUDIOTHREAD_H

#include <QThread>
#include <QString>

#include <QVector>

#include <atomic>

#include "audioengine.h"

class AudioBackend;
class VUBar;

// 音频处理线程：从后端读 → 引擎降噪 → 写回，并回调 VU / 频谱
class AudioThread : public QThread {
    Q_OBJECT

public:
    AudioThread(AudioEngine *engine, AudioBackend *backend, const QString &input,
                const QString &output, const QString &monitor, int mode,
                QObject *parent = nullptr);
    ~AudioThread() override;

    void stop();
    bool running() const { return running_.load(); }
    AudioBackend *takeBackend() { auto *b = backend_; backend_ = nullptr; return b; }

signals:
    void levelUpdated(double db);
    void spectrumData(const QVector<float> &in, const QVector<float> &out);
    void errorOccurred(const QString &msg);

protected:
    void run() override;

private:
    AudioEngine *engine_;
    AudioBackend *backend_;
    QString input_;
    QString output_;
    QString monitor_;
    int mode_;
    std::atomic<bool> running_{false};
};

#endif // PUREVOX_AUDIOTHREAD_H
