// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "mainwindow.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QVBoxLayout>

#include <aimic.h>

#include <QFileInfo>

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle("PureVox");
    setMinimumSize(720, 480);
    buildUi();
    initEngine();
}

void MainWindow::initEngine() {
    const char *denoise = "v9_fft2048_band256_epoch_261.onnx";
    const char *tse = "tse15_stream_ep_0673.onnx";
    const char *aec = "aec9_ep0544.onnx";
    if (!QFileInfo::exists(denoise)) {
        statusLabel_->setText("未找到模型文件，请从仓库根目录运行");
        return;
    }
    proc_ = audio_processor_new(0.0f, denoise, tse, aec);
    if (!proc_) {
        statusLabel_->setText("音频引擎初始化失败");
        return;
    }
    int backend = audio_processor_backend_effective(proc_);
    statusLabel_->setText(QString("PureVox 引擎就绪（后端 %1）").arg(backend));
}

void MainWindow::buildUi() {
    central_ = new QWidget(this);
    root_ = new QVBoxLayout(central_);
    root_->setSpacing(4);
    root_->setContentsMargins(4, 4, 4, 4);

    statusLabel_ = new QLabel("PureVox (Qt C++ 重构版)", central_);
    statusLabel_->setAlignment(Qt::AlignCenter);
    root_->addWidget(statusLabel_);

    setCentralWidget(central_);
}

MainWindow::~MainWindow() {
    if (proc_) {
        audio_processor_free(proc_);
    }
}
