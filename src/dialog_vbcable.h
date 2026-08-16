// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DIALOG_VBCABLE_H
#define PUREVOX_DIALOG_VBCABLE_H

#ifdef Q_OS_WIN

#include <QDialog>

class QLabel;
class QCheckBox;
class QPushButton;

// VB-CABLE 虚拟声卡检测面板（仅 Windows）：检测 + 安装引导。
// VB-CABLE 由用户自行下载安装，本面板只负责检测与引导。
class VbCableDialog : public QDialog {
    Q_OBJECT

public:
    explicit VbCableDialog(QWidget *parent = nullptr);

private:
    void refresh();

    QLabel *stateLabel_;
    QLabel *dotLabel_;
    QCheckBox *checkCb_;
    QPushButton *panelBtn_;
};

// VB-CABLE 是否已安装（CABLE Input + CABLE Output 双端点齐全）
bool vbcableInstalled();

#endif // Q_OS_WIN

#endif // PUREVOX_DIALOG_VBCABLE_H
