// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DIALOG_ABOUT_H
#define PUREVOX_DIALOG_ABOUT_H

#include <QDialog>

// 关于对话框（介绍 / 使用说明 / 更新日志 / 许可证）
class AboutDialog : public QDialog {
    Q_OBJECT

public:
    explicit AboutDialog(QWidget *parent = nullptr);
};

#endif // PUREVOX_DIALOG_ABOUT_H
