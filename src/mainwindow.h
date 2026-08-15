// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_MAINWINDOW_H
#define PUREVOX_MAINWINDOW_H

#include <QMainWindow>
#include <QWidget>

#include "audioengine.h"

class QVBoxLayout;
class QHBoxLayout;
class QLabel;
class QComboBox;
class QCheckBox;
class QPushButton;
class QSlider;
class VUBar;
class AudioThread;

class MainWindow : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(QWidget *parent = nullptr);
    ~MainWindow() override;

private:
    void buildUi();
    void initEngine();
    void refreshDevices();
    void applyMode(int mode);
    void saveGains(int mode);
    void loadGains(int mode);
    void onStartStop();
    void onModeChanged(int val);
    void updateModeUi();
    void saveConfig();
    void loadConfig();

    // UI 控件
    QWidget *central_;
    QVBoxLayout *root_;
    QLabel *statusLabel_;
    VUBar *vuBar_;

    QWidget *segWidget_;
    QComboBox *apiCombo_;
    QComboBox *inputCombo_;
    QComboBox *outputCombo_;
    QCheckBox *monitorCb_;
    QComboBox *monitorCombo_;
    QCheckBox *compCb_;
    QCheckBox *agcCb_;
    QCheckBox *vadCb_;
    QPushButton *refBtn_;
    QPushButton *startBtn_;
    QPushButton *quitBtn_;
    QSlider *preSlider_;
    QLabel *preLabel_;

    // 状态
    AudioEngine engine_;
    AudioThread *thread_ = nullptr;
    int mode_ = AudioEngine::ModeDenoise;
    double preGain_ = 0.0;
    bool processing_ = false;

    static constexpr const char *kModelDenoise = "v9_fft2048_band256_epoch_261.onnx";
    static constexpr const char *kModelTse = "tse15_stream_ep_0673.onnx";
    static constexpr const char *kModelAec = "aec9_ep0544.onnx";
};

#endif // PUREVOX_MAINWINDOW_H
