// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "dialog_tse_reference.h"

#include <QDialogButtonBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QPushButton>
#include <QSettings>
#include <QVBoxLayout>

#include <QFileInfo>

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
    row->addWidget(recBtn_);
    playBtn_ = new QPushButton(QStringLiteral("播放"), this);
    playBtn_->setFixedHeight(26);
    playBtn_->setMinimumWidth(72);
    row->addWidget(playBtn_);
    row->addStretch();
    auto *okBtn = new QPushButton(QStringLiteral("关闭"), this);
    okBtn->setFixedHeight(26);
    connect(okBtn, &QPushButton::clicked, this, &QDialog::accept);
    row->addWidget(okBtn);
    layout->addLayout(row);

    refresh();
}

TseReferenceDialog::~TseReferenceDialog() = default;

void TseReferenceDialog::refresh() {
    QSettings s;
    QString wav = s.value("tse_ref_wav_path").toString();
    if (!wav.isEmpty() && QFileInfo::exists(wav)) {
        statusLabel_->setText(QStringLiteral("已录制参考音频"));
        QFileInfo fi(wav);
        infoLabel_->setText(QStringLiteral("文件: %1\n位置: %2").arg(fi.fileName(), fi.absolutePath()));
        playBtn_->setEnabled(true);
    } else {
        statusLabel_->setText(QStringLiteral("尚未录制参考音频"));
        infoLabel_->setText(QStringLiteral("点击「录音」录制 10 秒参考语音。"));
        playBtn_->setEnabled(false);
    }
}
