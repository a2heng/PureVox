// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "dialog_tse_reference.h"

#include <QDialogButtonBox>
#include <QDir>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QLabel>
#include <QMessageBox>
#include <QPushButton>
#include <QSettings>
#include <QThread>
#include <QVBoxLayout>

#include <algorithm>
#include <chrono>

#include "audiobackend.h"
#include "dsp/processor.h"
#include "wav_io.h"

#ifdef Q_OS_LINUX
#include "pwbackend.h"
#endif

namespace {
const char *kRefWavKey = "tse_ref_wav_path";
}  // namespace

TseReferenceDialog::TseReferenceDialog(QWidget *parent) : QDialog(parent) {
    setWindowTitle(QStringLiteral("TSE 参考音频"));
    setModal(true);
    setMinimumWidth(400);

    auto *layout = new QVBoxLayout(this);

    auto *hint = new QLabel(QStringLiteral(
        "录制 10 秒参考语音（你的声音），TSE 据此提取目标说话人。\n"
        "保持安静环境，不要有背景噪声或他人的声音。"), this);
    hint->setWordWrap(true);
    layout->addWidget(hint);

    statusLabel_ = new QLabel(QString(), this);
    statusLabel_->setStyleSheet(QStringLiteral("font-weight: bold;"));
    layout->addWidget(statusLabel_);

    infoLabel_ = new QLabel(QString(), this);
    infoLabel_->setWordWrap(true);
    infoLabel_->setTextInteractionFlags(Qt::TextSelectableByMouse);
    layout->addWidget(infoLabel_);

    auto *row = new QHBoxLayout();
    recBtn_ = new QPushButton(QStringLiteral("录音"), this);
    recBtn_->setFixedHeight(26);
    recBtn_->setMinimumWidth(96);
    connect(recBtn_, &QPushButton::clicked, this, &TseReferenceDialog::onRecord);
    row->addWidget(recBtn_);
    playBtn_ = new QPushButton(QStringLiteral("播放"), this);
    playBtn_->setFixedHeight(26);
    playBtn_->setMinimumWidth(72);
    connect(playBtn_, &QPushButton::clicked, this, &TseReferenceDialog::onPlay);
    row->addWidget(playBtn_);
    row->addStretch();
    auto *okBtn = new QPushButton(QStringLiteral("关闭"), this);
    okBtn->setFixedHeight(26);
    connect(okBtn, &QPushButton::clicked, this, &QDialog::accept);
    row->addWidget(okBtn);
    layout->addLayout(row);

    connect(&countdownTimer_, &QTimer::timeout, this, &TseReferenceDialog::startCountdown);
    connect(&progressTimer_, &QTimer::timeout, this, &TseReferenceDialog::updateProgress);
    connect(this, &TseReferenceDialog::recordBufferReady, this,
            [this]() { onRecordDone(true); });

    refresh();
}

TseReferenceDialog::~TseReferenceDialog() {
    if (recordThread_.joinable()) recordThread_.join();
}

void TseReferenceDialog::refresh() {
    QSettings s;
    wavPath_ = s.value(kRefWavKey).toString();
    if (!wavPath_.isEmpty() && QFileInfo::exists(wavPath_)) {
        statusLabel_->setText(QStringLiteral("已录制参考音频"));
        QFileInfo fi(wavPath_);
        infoLabel_->setText(QStringLiteral("文件: %1\n位置: %2").arg(fi.fileName(), fi.absolutePath()));
        playBtn_->setEnabled(true);
    } else {
        statusLabel_->setText(QStringLiteral("尚未录制参考音频"));
        infoLabel_->setText(QStringLiteral("点击「录音」录制 10 秒参考语音。"));
        playBtn_->setEnabled(false);
    }
}

void TseReferenceDialog::onRecord() {
    if (recording_) return;
    recBtn_->setEnabled(false);
    statusLabel_->setText(QStringLiteral("3 秒倒计时…"));
    countdown_ = 3;
    countdownTimer_.start(1000);
}

void TseReferenceDialog::startCountdown() {
    countdown_--;
    if (countdown_ > 0) {
        statusLabel_->setText(QStringLiteral("%1 秒倒计时…").arg(countdown_));
        return;
    }
    countdownTimer_.stop();
    statusLabel_->setText(QStringLiteral("录音中…"));
    recording_ = true;
    recordBuffer_.clear();
    elapsedMs_ = 0;
    progressTimer_.start(50);
    if (recordThread_.joinable()) recordThread_.join();
    recordThread_ = std::thread([this]() { doRecord(); });
}

void TseReferenceDialog::doRecord() {
    std::vector<float> buf;
#ifdef Q_OS_LINUX
    // 独立降噪会话采集参考（降噪模式，输出即参考语音）
    pv::Processor proc;
    bool engineOk = proc.init("v9_fft2048_band256_epoch_261.onnx",
                              "tse15_stream_ep_0673.onnx", "aec9_ep0544.onnx");
    proc.setMode(pv::Processor::ModeDenoise);
    QSettings s;
    QString input = s.value("input_device").toString();
    QString output = s.value("output_device").toString();

    PwBackend backend;
    if (engineOk && !input.isEmpty() && !output.isEmpty() &&
        backend.open(input, output, QString())) {
        float in[1024], out[1024];
        auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(kRecordDurationMs);
        while (recording_ && std::chrono::steady_clock::now() < deadline) {
            size_t n = backend.read(in, 1024);
            if (n > 0) {
                size_t on = proc.process(in, n, nullptr, 0, out);
                buf.insert(buf.end(), out, out + on);
            }
            QThread::msleep(2);
        }
        backend.close();
    }
#endif
    recording_ = false;
    recordBuffer_ = std::move(buf);
    emit recordBufferReady();
}

void TseReferenceDialog::updateProgress() {
    elapsedMs_ += 50;
    int pct = std::min(100, elapsedMs_ * 100 / kRecordDurationMs);
    statusLabel_->setText(QStringLiteral("录音中… %1%").arg(pct));
    if (elapsedMs_ >= kRecordDurationMs) progressTimer_.stop();
}

void TseReferenceDialog::onRecordDone(bool ok) {
    recBtn_->setEnabled(true);
    recBtn_->setText(QStringLiteral("录音"));
    if (!ok || recordBuffer_.empty()) {
        statusLabel_->setText(QStringLiteral("录音失败，请检查麦克风/输出配置"));
        return;
    }
    // 写 WAV
    QString wavDir = QDir::homePath() + "/.purevox";
    QDir().mkpath(wavDir);
    wavPath_ = wavDir + "/tse_reference.wav";
    if (!WavIO::writeF32ToWav(wavPath_, recordBuffer_, recordSampleRate_)) {
        statusLabel_->setText(QStringLiteral("保存参考音频失败"));
        return;
    }
    QSettings().setValue(kRefWavKey, wavPath_);
    statusLabel_->setText(QStringLiteral("参考音频已录制并保存"));
    infoLabel_->setText(QStringLiteral("文件: tse_reference.wav\n位置: %1").arg(wavDir));
    playBtn_->setEnabled(true);
    emit referenceRecorded(recordBuffer_, recordSampleRate_);
    refresh();
}

void TseReferenceDialog::onPlay() {
    if (playing_) return;
    if (wavPath_.isEmpty() || !QFileInfo::exists(wavPath_)) return;
    playing_ = true;
    // 简化：用 QSound 或系统播放（后续完善）
    playing_ = false;
}
