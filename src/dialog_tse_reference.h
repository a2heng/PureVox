// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DIALOG_TSE_REFERENCE_H
#define PUREVOX_DIALOG_TSE_REFERENCE_H

#include <QDialog>

#include <QTimer>

#include <atomic>
#include <thread>
#include <vector>

class QLabel;
class QPushButton;
class AudioBackend;

namespace pv { class Processor; }

// TSE 参考音频弹框：录音（临时降噪会话采集 10 秒）+ 播放 + 文件信息
class TseReferenceDialog : public QDialog {
    Q_OBJECT

public:
    explicit TseReferenceDialog(QWidget *parent = nullptr);
    ~TseReferenceDialog() override;

signals:
    // 录音完成，把参考样本交回主窗口加载到主引擎（TSE 参考）
    void referenceRecorded(const std::vector<float> &samples, int sampleRate);
    // 录音线程完成（队列到主线程触发 onRecordDone）
    void recordBufferReady();

private:
    void refresh();
    void onRecord();
    void onPlay();
    void doRecord();
    void startCountdown();
    void updateProgress();
    void onRecordDone(bool ok);

    QLabel *statusLabel_;
    QLabel *infoLabel_;
    QPushButton *recBtn_;
    QPushButton *playBtn_;

    std::atomic<bool> recording_{false};
    bool playing_ = false;
    QTimer countdownTimer_;
    int countdown_ = 0;
    QTimer progressTimer_;
    int elapsedMs_ = 0;
    std::thread recordThread_;
    std::vector<float> recordBuffer_;
    int recordSampleRate_ = 48000;
    QString wavPath_;

    static constexpr int kRecordDurationMs = 10000;
};

#endif // PUREVOX_DIALOG_TSE_REFERENCE_H
