// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "dialog_about.h"

#include <QDialogButtonBox>
#include <QLabel>
#include <QTabWidget>
#include <QTextBrowser>
#include <QVBoxLayout>

namespace {

QTextBrowser *makeDoc(const QString &md) {
    auto *tb = new QTextBrowser;
    tb->setMarkdown(md);
    return tb;
}

const QString kIntro =
    "# PureVox — AI 麦克风降噪\n\n"
    "实时 AI 音频降噪 / 目标说话人提取 / 回声消除。\n\n"
    "本版本为 Qt C++ 重构版（移除 Python 依赖，改用系统库）。\n"
    "最后支持 Windows 7 的版本见 win7 分支。\n\n"
    "**说明**：内置 AI 模型（*.onnx）归 a2heng 所有，禁止提取用于其他项目，"
    "仅随 PureVox 经授权使用（见 MODEL-LICENSE.md）。";

const QString kChangelog =
    "# 更新日志\n\n"
    "- 2026-08-15 — 开始 Qt C++ 全重构：移除 Python 依赖，改为系统库\n"
    "  （libpipewire / libasound / onnxruntime），重构为 CMake 工程。\n"
    "  核心主面板 / 频谱图 / EQ 已用 C++ 还原。\n"
    "- 2026-08-14 — v2026.08.14.1643 为最后支持 Windows 7 的版本。";

const QString kLicense =
    "## 许可证\n\n"
    "- 源码：GPL-3.0（SPDX: GPL-3.0-or-later）\n"
    "- 内置 AI 模型（*.onnx）：不随 GPL 授权，归 a2heng 所有，"
    "仅随 PureVox 经授权使用（见 MODEL-LICENSE.md）";

}  // namespace

AboutDialog::AboutDialog(QWidget *parent) : QDialog(parent) {
    setWindowTitle(QStringLiteral("关于 PureVox"));
    setMinimumSize(560, 420);

    auto *layout = new QVBoxLayout(this);

    auto *tabs = new QTabWidget(this);
    tabs->addTab(makeDoc(kIntro), QStringLiteral("介绍"));
    tabs->addTab(makeDoc(QStringLiteral("## Windows 使用说明\n\n见 win7 分支或原 README")),
                 QStringLiteral("使用说明"));
    tabs->addTab(makeDoc(kChangelog), QStringLiteral("更新日志"));
    tabs->addTab(makeDoc(kLicense), QStringLiteral("许可证"));
    layout->addWidget(tabs);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::close);
    connect(buttons, &QDialogButtonBox::accepted, this, &QDialog::close);
    layout->addWidget(buttons);
}
