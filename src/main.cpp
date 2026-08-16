// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include <QApplication>
#include <QIcon>
#include <QTimer>

#include "mainwindow.h"

int main(int argc, char *argv[]) {
    // PureVox 使用自带主题（QPalette），禁用系统平台主题插件
    // （KDEPlasmaPlatformTheme 会连带加载 Qt6Quick/QML/LLVM/KF6，纯浪费内存）
    // 空值会被 Qt 当"未设置"回退到 KDE；用一个无效名强制回退内置默认
    qputenv("QT_QPA_PLATFORMTHEME", QByteArrayLiteral("none"));

    QApplication app(argc, argv);
    app.setApplicationName("PureVox");
    app.setApplicationDisplayName("PureVox");

    QIcon icon(":/purevox_icon.svg");
    app.setWindowIcon(icon);

    MainWindow window;
    window.show();

    // --selftest：自动跑一次 启动(直通)→切降噪→停止 生命周期，用于验证无闪退
    if (app.arguments().contains("--selftest")) {
        QTimer::singleShot(6000, [&window]() { window.quitAppForTest(); });
    }
    // --ui-only：只显示 UI，不启动音频处理（内存二分用）
    if (app.arguments().contains("--ui-only")) {
        QTimer::singleShot(8000, [&window]() { window.quitAppForTest(); });
    }

    return app.exec();
}
