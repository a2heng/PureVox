// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DIALOG_TSE_REFERENCE_H
#define PUREVOX_DIALOG_TSE_REFERENCE_H

#include <QDialog>

class QLabel;
class QPushButton;

// TSE 参考音频弹框：说明 + 参考状态 + 录音（基础版）
class TseReferenceDialog : public QDialog {
    Q_OBJECT

public:
    explicit TseReferenceDialog(QWidget *parent = nullptr);
    ~TseReferenceDialog() override;

private:
    void refresh();

    QLabel *statusLabel_;
    QLabel *infoLabel_;
    QPushButton *recBtn_;
    QPushButton *playBtn_;
    bool recording_ = false;
};

#endif // PUREVOX_DIALOG_TSE_REFERENCE_H
