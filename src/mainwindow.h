// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_MAINWINDOW_H
#define PUREVOX_MAINWINDOW_H

#include <QMainWindow>
#include <QWidget>

struct AudioProcessor;

class QVBoxLayout;
class QLabel;

class MainWindow : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(QWidget *parent = nullptr);
    ~MainWindow() override;

private:
    void buildUi();
    void initEngine();

    QWidget *central_;
    QVBoxLayout *root_;
    QLabel *statusLabel_;
    AudioProcessor *proc_ = nullptr;
};

#endif // PUREVOX_MAINWINDOW_H
