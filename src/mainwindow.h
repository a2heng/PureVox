// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_MAINWINDOW_H
#define PUREVOX_MAINWINDOW_H

#include <QMainWindow>
#include <QWidget>

#include <QVector>

#include "audioengine.h"

class AudioBackend;

class QVBoxLayout;
class QHBoxLayout;
class QLabel;
class QComboBox;
class QCheckBox;
class QPushButton;
class QSlider;
class QSystemTrayIcon;
class QMenu;
class QTimer;
class VUBar;
class SpectrumWidget;
class EQCurveWidget;
class AudioThread;

class MainWindow : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(QWidget *parent = nullptr);
    ~MainWindow() override;

protected:
    void closeEvent(QCloseEvent *event) override;
    void changeEvent(QEvent *event) override;

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
    void createMenu();
    void showAbout();
    void showVirtualMic();
    void quitApp();
    void createTray();

    // EQ 面板
    void setupEqPanel();
    void onEqSlot(int slot);
    void onEqPreset(const QVector<double> &gains);
    void saveEqPreset(int slot);
    void loadEqConfig();
    void updateEqButtons();
    void onEqChanged(const QVector<double> &gains);

    // 布局常量（与 Python 版一致）
    static constexpr int kPanelW = 320;
    static constexpr int kSpectrumW = 551;
    static constexpr int kEqH = 270;
    static constexpr int kBaseH = 350;

    // UI 控件
    QWidget *central_;
    QVBoxLayout *root_;
    QLabel *statusLabel_;
    VUBar *vuBar_;
    SpectrumWidget *spectrum_;
    EQCurveWidget *eqCurve_;
    QWidget *eqPanel_;
    QWidget *eqBtnsContainer_;
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

    // 托盘
    QSystemTrayIcon *trayIcon_ = nullptr;
    QMenu *trayMenu_ = nullptr;

    // EQ 状态
    QVector<QPushButton *> eqSlotButtons_;
    QVector<QVector<double>> eqPresets_;  // 8 槽位
    int eqActiveSlot_ = 0;
    bool updatingEqUi_ = false;

    // 状态
    AudioEngine engine_;
    AudioThread *thread_ = nullptr;
    AudioBackend *backend_ = nullptr;
    int mode_ = AudioEngine::ModeDenoise;
    double preGain_ = 0.0;
    bool processing_ = false;

    static constexpr const char *kModelDenoise = "v9_fft2048_band256_epoch_261.onnx";
    static constexpr const char *kModelTse = "tse15_stream_ep_0673.onnx";
    static constexpr const char *kModelAec = "aec9_ep0544.onnx";
};

#endif // PUREVOX_MAINWINDOW_H
