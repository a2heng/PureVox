// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include <QApplication>
#include <QIcon>

#include "mainwindow.h"

int main(int argc, char *argv[]) {
    QApplication app(argc, argv);
    app.setApplicationName("PureVox");
    app.setApplicationDisplayName("PureVox");

    QIcon icon(":/purevox_icon.png");
    app.setWindowIcon(icon);

    MainWindow window;
    window.show();

    return app.exec();
}
