// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "mainwindow.h"

#include <QApplication>
#include <QCheckBox>
#include <QCloseEvent>
#include <QComboBox>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QProcess>
#include <QPushButton>
#include <QSettings>
#include <QSlider>
#include <QStyle>
#include <QSystemTrayIcon>
#include <QTimer>
#include <QVBoxLayout>

#include <QVector>

#include "audiobackend.h"
#include "audiothread.h"
#ifdef Q_OS_LINUX
#include "alsabackend.h"
#include "pwbackend.h"
#include "virtualmic.h"
#endif
#include "deviceenum.h"
#include "dialog_about.h"
#include "dialog_tse_reference.h"
#ifdef Q_OS_WIN
#include "dialog_vbcable.h"
#endif
#include "eqcurve.h"
#include "networkserver.h"
#include "segmented.h"
#include "spectrum.h"
#include "vubar.h"

namespace {
constexpr int kModeOff = 0;
constexpr int kModeDenoise = 1;
constexpr int kModeAec = 2;
constexpr int kModeTse = 3;
const char *kModeNames[] = {"直通", "降噪", "AEC", "TSE"};
constexpr int kApiPipeWire = 17;  // 与旧版 api_type 一致
constexpr int kApiAlsa = 18;
constexpr int kApiNetwork = 19;
}  // namespace

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle("PureVox");
    buildUi();
    createMenu();
    createTray();
    startWatchdog();
    initEngine();
    loadConfig();
    refreshDevices();
    setFixedSize(kPanelW + kSpectrumW, kBaseH + kEqH);
    show();
}

MainWindow::~MainWindow() {
    if (thread_) {
        thread_->stop();
        delete thread_;
        thread_ = nullptr;
    }
}

void MainWindow::closeEvent(QCloseEvent *event) {
    // 关闭窗口 = 最小化到托盘，不退出
    event->ignore();
    hide();
}

void MainWindow::changeEvent(QEvent *event) {
    QMainWindow::changeEvent(event);
}

void MainWindow::buildUi() {
    central_ = new QWidget(this);
    root_ = new QVBoxLayout(central_);
    root_->setSpacing(4);
    root_->setContentsMargins(4, 4, 4, 4);

    // 左列控制面板 + 右列频谱图
    auto *topContainer = new QWidget(central_);
    auto *topLayout = new QHBoxLayout(topContainer);
    topLayout->setContentsMargins(0, 0, 0, 0);
    topLayout->setSpacing(0);

    auto *panel = new QWidget(topContainer);
    auto *panelLayout = new QVBoxLayout(panel);
    panelLayout->setSpacing(4);
    panelLayout->setContentsMargins(0, 0, 4, 0);

    // 第1行：模式选择
    QVector<SegmentedControl::Item> items = {
        {QStringLiteral("直通"), kModeOff},
        {QStringLiteral("降噪"), kModeDenoise},
        {QStringLiteral("AEC"), kModeAec},
        {QStringLiteral("TSE"), kModeTse},
    };
    auto *seg = new SegmentedControl(items, panel);
    seg->setToolTip(QStringLiteral(
        "直通 — 原始麦克风信号，轻处理（前增益+EQ+AGC+VAD）\n"
        "降噪 — AI 深度学习降噪，消除键盘、风扇、空调等噪声\n"
        "AEC  — 回声消除 + 降噪，消除扬声器回声\n"
        "TSE  — 目标说话人提取，只保留指定人的声音"));
    segWidget_ = seg;
    connect(seg, &SegmentedControl::valueChanged, this, &MainWindow::onModeChanged);
    panelLayout->addWidget(seg);

    // 第2行：压缩/AGC/VAD + 参考音频
    auto *optsRow = new QHBoxLayout();
    optsRow->setSpacing(6);
    compCb_ = new QCheckBox(QStringLiteral("压缩"), panel);
    compCb_->setFixedHeight(22);
    compCb_->setToolTip(QStringLiteral("压缩器：动态压缩大音量、提升小音量，缩小音量动态范围。"));
    connect(compCb_, &QCheckBox::toggled, this, [this](bool on) {
        engine_.setCompressorEnabled(on);
        saveConfig();
    });
    optsRow->addWidget(compCb_);
    agcCb_ = new QCheckBox(QStringLiteral("AGC"), panel);
    agcCb_->setFixedHeight(22);
    agcCb_->setToolTip(QStringLiteral("AGC 自动增益控制：自动调节增益，让声音始终稳定在合适音量。"));
    connect(agcCb_, &QCheckBox::toggled, this, [this](bool on) {
        double initDb = on ? preGain_ : 0.0;
        engine_.setAgcEnabled(on, initDb);
        if (on) {
            preGainSlider_->setEnabled(false);
            preGainSlider_->setStyleSheet(QStringLiteral("QSlider { opacity: 0.5; }"));
            if (processing_ && agcPollTimer_) agcPollTimer_->start();
        } else {
            preGainSlider_->setEnabled(true);
            preGainSlider_->setStyleSheet(QString());
            if (agcPollTimer_) agcPollTimer_->stop();
        }
        saveConfig();
    });
    optsRow->addWidget(agcCb_);
    vadCb_ = new QCheckBox(QStringLiteral("VAD"), panel);
    vadCb_->setFixedHeight(22);
    vadCb_->setToolTip(QStringLiteral("VAD 语音活性检测：不说话时自动静音，消除残留噪声。"));
    connect(vadCb_, &QCheckBox::toggled, this, [this](bool on) {
        engine_.setVadEnabled(on);
        saveConfig();
    });
    optsRow->addWidget(vadCb_);
    optsRow->addStretch();
    refBtn_ = new QPushButton(QStringLiteral("参考音频…"), panel);
    refBtn_->setFixedHeight(22);
    refBtn_->setMinimumWidth(86);
    refBtn_->setVisible(false);
    connect(refBtn_, &QPushButton::clicked, this, [this]() {
        TseReferenceDialog dlg(this);
        connect(&dlg, &TseReferenceDialog::referenceRecorded, this,
                [this](const std::vector<float> &samples, int) {
                    if (!samples.empty()) {
                        engine_.setTseReference(samples.data(), samples.size());
                    }
                });
        dlg.exec();
    });
    optsRow->addWidget(refBtn_);
    panelLayout->addLayout(optsRow);

    // 第3行：pre 增益 + post 增益
    auto *gainGrid = new QGridLayout();
    gainGrid->setSpacing(4);

    auto *lblPre = new QLabel(QStringLiteral("Pre 增益"), panel);
    lblPre->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    lblPre->setToolTip(QStringLiteral(
        "Pre 增益 — 输入增益，位于处理链首\n"
        "提高 Pre 增益 → 信号更强，降噪效果更好。\n"
        "但过高会导致削波失真。\n"
        "建议: 正常说话时峰值在 -12 ~ -6 dBFS 为最佳。"));
    gainGrid->addWidget(lblPre, 0, 0);
    preGainSlider_ = new QSlider(Qt::Horizontal, panel);
    preGainSlider_->setRange(-30, 30);
    preGainSlider_->setValue((int)preGain_);
    gainGrid->addWidget(preGainSlider_, 0, 1);
    preGainLabel_ = new QLabel("+0 dB", panel);
    preGainLabel_->setFixedWidth(44);
    preGainLabel_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    gainGrid->addWidget(preGainLabel_, 0, 2);
    connect(preGainSlider_, &QSlider::valueChanged, this, [this](int v) {
        preGain_ = v;
        preGainLabel_->setText(QString("%1 dB").arg((double)v));
        engine_.setPreGain(preGain_);
        saveConfig();
    });

    auto *lblPost = new QLabel(QStringLiteral("Post 增益"), panel);
    lblPost->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    lblPost->setToolTip(QStringLiteral(
        "Post 增益 — 输出增益，位于处理链尾\n"
        "调整降噪后的整体响度。"));
    gainGrid->addWidget(lblPost, 1, 0);
    postGainSlider_ = new QSlider(Qt::Horizontal, panel);
    postGainSlider_->setRange(-30, 30);
    postGainSlider_->setValue((int)postGain_);
    gainGrid->addWidget(postGainSlider_, 1, 1);
    postGainLabel_ = new QLabel("+0 dB", panel);
    postGainLabel_->setFixedWidth(44);
    postGainLabel_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    gainGrid->addWidget(postGainLabel_, 1, 2);
    connect(postGainSlider_, &QSlider::valueChanged, this, [this](int v) {
        postGain_ = v;
        postGainLabel_->setText(QString("%1 dB").arg((double)v));
        engine_.setPostGain(postGain_);
        saveConfig();
    });
    panelLayout->addLayout(gainGrid);

    // 第4行：设备
    auto *devGrid = new QGridLayout();
    devGrid->setSpacing(4);
    devGrid->setColumnMinimumWidth(0, 56);
    devGrid->setColumnStretch(1, 1);

    auto *lblApi = new QLabel(QStringLiteral("音频接口"), panel);
    lblApi->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    devGrid->addWidget(lblApi, 0, 0);
    apiCombo_ = new QComboBox(panel);
    apiCombo_->addItem(QStringLiteral("PipeWire"), kApiPipeWire);
    apiCombo_->addItem(QStringLiteral("ALSA"), kApiAlsa);
    apiCombo_->addItem(QStringLiteral("网络"), kApiNetwork);
    connect(apiCombo_, &QComboBox::currentIndexChanged, this, [this](int) {
        refreshDevices();
        saveConfig();
    });
    devGrid->addWidget(apiCombo_, 0, 1);

    auto *lblIn = new QLabel(QStringLiteral("输入设备"), panel);
    lblIn->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    devGrid->addWidget(lblIn, 1, 0);
    inputCombo_ = new QComboBox(panel);
    devGrid->addWidget(inputCombo_, 1, 1);

    auto *lblOut = new QLabel(QStringLiteral("输出设备"), panel);
    lblOut->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    devGrid->addWidget(lblOut, 2, 0);
    outputCombo_ = new QComboBox(panel);
    devGrid->addWidget(outputCombo_, 2, 1);

    monitorCb_ = new QCheckBox(QStringLiteral("监听"), panel);
    monitorCb_->setToolTip(QStringLiteral(
        "监听（耳返）：\n"
        "将处理后的声音实时回放到指定设备。"));
    connect(monitorCb_, &QCheckBox::toggled, this, [this](bool on) {
        saveConfig();
    });
    devGrid->addWidget(monitorCb_, 3, 0);
    monitorCombo_ = new QComboBox(panel);
    devGrid->addWidget(monitorCombo_, 3, 1);
    panelLayout->addLayout(devGrid);

    // VU
    vuBar_ = new VUBar(panel);
    panelLayout->addWidget(vuBar_);

    // 第5行：启动/退出
    auto *btnRow = new QHBoxLayout();
    btnRow->setSpacing(4);
    startBtn_ = new QPushButton(QStringLiteral("启动音频处理"), panel);
    startBtn_->setFixedHeight(38);
    startBtn_->setToolTip(QStringLiteral("启动/停止音频处理引擎。\n快捷键: 右 Alt + >"));
    connect(startBtn_, &QPushButton::clicked, this, &MainWindow::onStartStop);
    btnRow->addWidget(startBtn_);
    quitBtn_ = new QPushButton(QStringLiteral("退出"), panel);
    quitBtn_->setFixedHeight(38);
    connect(quitBtn_, &QPushButton::clicked, this, &MainWindow::quitApp);
    btnRow->addWidget(quitBtn_);
    panelLayout->addLayout(btnRow);

    statusLabel_ = new QLabel(QStringLiteral("PureVox"), panel);
    statusLabel_->setAlignment(Qt::AlignCenter);
    panelLayout->addWidget(statusLabel_);

    // 面板固定在左列（与 Python 版一致：面板320 / 频谱551）
    panel->setFixedWidth(kPanelW);
    topLayout->addWidget(panel, 0);

    // 右列：频谱图
    spectrum_ = new SpectrumWidget(topContainer);
    spectrum_->setFixedWidth(kSpectrumW);
    topLayout->addWidget(spectrum_, 0);
    root_->addWidget(topContainer);

    // EQ 面板（下方横贯，总高 kEqH）：EQ 曲线在上，按钮行在下
    eqPanel_ = new QWidget(central_);
    auto *eqLayout = new QVBoxLayout(eqPanel_);
    eqLayout->setContentsMargins(0, 0, 0, 0);
    eqLayout->setSpacing(4);

    eqCurve_ = new EQCurveWidget(eqPanel_);
    eqCurve_->setMinimumHeight(kEqH - 60);
    eqLayout->addWidget(eqCurve_, 1);
    eqPanel_->setFixedHeight(kEqH);
    root_->addWidget(eqPanel_);
    setupEqPanel();

    setCentralWidget(central_);
}

void MainWindow::setupEqPanel() {
    auto *eqLayout = qobject_cast<QVBoxLayout *>(eqPanel_->layout());
    if (!eqLayout) return;

    eqBtnsContainer_ = new QWidget(eqPanel_);
    auto *btnsLayout = new QVBoxLayout(eqBtnsContainer_);
    btnsLayout->setContentsMargins(4, 0, 0, 4);
    btnsLayout->setSpacing(4);

    // EQ 插槽按钮行
    auto *slotRow = new QHBoxLayout();
    slotRow->setSpacing(4);
    auto *lbl = new QLabel(QStringLiteral("插槽"), eqBtnsContainer_);
    lbl->setFixedWidth(40);
    lbl->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    slotRow->addWidget(lbl);
    eqSlotButtons_.clear();
    for (int i = 0; i < 8; ++i) {
        auto *btn = new QPushButton(QStringLiteral("S%1").arg(i + 1), eqBtnsContainer_);
        btn->setFixedHeight(24);
        btn->setFixedWidth(80);
        connect(btn, &QPushButton::clicked, this, [this, i]() { onEqSlot(i); });
        eqSlotButtons_.append(btn);
        slotRow->addWidget(btn);
    }
    slotRow->addStretch();
    btnsLayout->addLayout(slotRow);

    // EQ 预设按钮行
    struct EqPreset {
        const char *name;
        const double gains[61];
    };
    static const EqPreset kPresets[] = {
        {"默认平直", {0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0}},
        {"清晰透亮", {-4,0,-3,0,-2,0,-2,0,-1,0,-1,0,-1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,2,1,0,0,0,0,0,0,0,0,0,0,0}},
        {"温暖饱满", {-4,0,-3,0,-2,0,-2,0,-1,0,-1,0,-1,0,0,0,0,2,4,3,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,-1,-1,0,0,0,0,0,0,0}},
        {"低沉有力", {-4,0,-3,0,-2,0,-2,0,-1,0,1,4,6,4,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0}},
        {"减少齿音", {-4,0,-3,0,-2,0,-2,0,-1,0,-1,0,-1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,3,0,-15,0,3,0,0,0,0,0,0,0,0}},
        {"减少鼻音", {-4,0,-3,0,-2,0,-2,0,-1,0,-1,0,-1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,0,-14,0,2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0}},
        {"消除沉闷", {-4,0,-3,0,-2,0,-2,0,-1,0,-1,0,-1,0,0,0,0,0,0,0,2,0,-12,0,2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0}},
        {"增强临场", {-4,0,-3,0,-2,0,-2,0,-1,0,-1,0,-1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,3,4,3,1,0,0,0,0,0,0,1,2,1,0,0,0,0,0}},
    };
    auto *presetRow = new QHBoxLayout();
    presetRow->setSpacing(4);
    auto *lbl2 = new QLabel(QStringLiteral("预设"), eqBtnsContainer_);
    lbl2->setFixedWidth(40);
    lbl2->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    presetRow->addWidget(lbl2);
    for (const EqPreset &p : kPresets) {
        auto *btn = new QPushButton(QString::fromUtf8(p.name), eqBtnsContainer_);
        btn->setFixedHeight(24);
        btn->setFixedWidth(80);
        QVector<double> g;
        g.reserve(61);
        for (int i = 0; i < 61; ++i) g.append(p.gains[i]);
        connect(btn, &QPushButton::clicked, this, [this, g]() { onEqPreset(g); });
        presetRow->addWidget(btn);
    }
    presetRow->addStretch();
    btnsLayout->addLayout(presetRow);

    eqLayout->addWidget(eqBtnsContainer_, 0);

    connect(eqCurve_, &EQCurveWidget::gainsChanged, this, &MainWindow::onEqChanged);
}

void MainWindow::createMenu() {
    QMenuBar *mb = menuBar();

    QMenu *settingsMenu = mb->addMenu(QStringLiteral("设置"));
    QAction *hk = settingsMenu->addAction(QStringLiteral("快捷键 (右Alt+>)"));
    hk->setCheckable(true);
    hk->setChecked(QSettings().value("hotkey_enabled", true).toBool());
    connect(hk, &QAction::toggled, [](bool on) { QSettings().setValue("hotkey_enabled", on); });

    QAction *autoRun = settingsMenu->addAction(QStringLiteral("启动时自动运行"));
    autoRun->setCheckable(true);
    autoRun->setChecked(QSettings().value("auto_start", false).toBool());
    connect(autoRun, &QAction::toggled, [](bool on) {
        QSettings().setValue("auto_start", on);
    });

    // 开机自启（Linux：~/.config/autostart/purevox.desktop）
    QAction *boot = settingsMenu->addAction(QStringLiteral("开机自启"));
    boot->setCheckable(true);
    const QString autostartDir = QDir::homePath() + "/.config/autostart";
    const QString autostartFile = autostartDir + "/purevox.desktop";
    boot->setChecked(QFileInfo::exists(autostartFile));
    connect(boot, &QAction::toggled, [autostartDir, autostartFile](bool on) {
        if (on) {
            QDir().mkpath(autostartDir);
            QFile f(autostartFile);
            if (f.open(QIODevice::WriteOnly | QIODevice::Text)) {
                QTextStream ts(&f);
                ts << "[Desktop Entry]\n"
                   << "Type=Application\n"
                   << "Name=PureVox\n"
                   << "Exec=/usr/bin/purevox\n"
                   << "X-GNOME-Autostart-enabled=true\n";
                f.close();
            }
        } else {
            QFile::remove(autostartFile);
        }
    });

    QMenu *themeMenu = settingsMenu->addMenu(QStringLiteral("主题"));
    struct ThemeItem {
        const char *label;
        const char *value;
    };
    const ThemeItem themes[] = {{"系统", "system"}, {"白天", "light"}, {"黑夜", "dark"}};
    QSettings st;
    QString curTheme = st.value("theme", "system").toString();
    for (const ThemeItem &t : themes) {
        QAction *a = themeMenu->addAction(QString::fromUtf8(t.label));
        a->setCheckable(true);
        a->setChecked(curTheme == QString::fromUtf8(t.value));
        connect(a, &QAction::triggered, this, [this, t]() {
            QSettings().setValue("theme", QString::fromUtf8(t.value));
            applyTheme(QString::fromUtf8(t.value));
        });
    }

    QAction *snd = mb->addAction(QStringLiteral("系统声音"));
    connect(snd, &QAction::triggered, this, []() {
        // 优先 pavucontrol，回退 gnome-control-center / systemsettings
        const char *cmds[][2] = {
            {"pavucontrol", ""},
            {"gnome-control-center", "sound"},
            {"systemsettings", ""},
        };
        for (auto &c : cmds) {
            if (QProcess::startDetached(QString::fromUtf8(c[0]),
                                        c[1][0] ? QStringList{QString::fromUtf8(c[1])} : QStringList()))
                return;
        }
    });

    QAction *vmic = mb->addAction(QStringLiteral("虚拟声卡"));
    connect(vmic, &QAction::triggered, this, &MainWindow::showVirtualMic);

    QAction *about = mb->addAction(QStringLiteral("关于"));
    connect(about, &QAction::triggered, this, &MainWindow::showAbout);
}

void MainWindow::showAbout() {
    AboutDialog dlg(this);
    dlg.exec();
}

void MainWindow::showVirtualMic() {
#ifdef Q_OS_LINUX
    QString msg;
    if (VirtualMic::ready()) {
        msg = QStringLiteral(
            "虚拟麦克风已就绪。\n\n"
            "出口：purevox_out.monitor（宽口径源）/ purevox_mic（真源）\n\n"
            "要清理吗？");
        auto ret = QMessageBox::question(
            this, QStringLiteral("虚拟声卡"), msg, QMessageBox::Yes | QMessageBox::No);
        if (ret == QMessageBox::Yes) {
            VirtualMic::remove();
            refreshDevices();
        }
    } else {
        msg = QStringLiteral(
            "Linux 虚拟麦克风（PipeWire）\n\n"
            "创建：purevox_out（null-sink）+ purevox_mic（remap-source）\n\n"
            "创建虚拟麦克风？");
        auto ret = QMessageBox::question(
            this, QStringLiteral("虚拟声卡"), msg, QMessageBox::Yes | QMessageBox::No);
        if (ret == QMessageBox::Yes) {
            bool ok = VirtualMic::ensure();
            QMessageBox::information(this, QStringLiteral("虚拟声卡"),
                                     ok ? QStringLiteral("虚拟麦克风已创建。")
                                        : QStringLiteral("创建失败，请检查 PipeWire 是否运行。"));
            refreshDevices();
        }
    }
#else
#ifdef Q_OS_WIN
    VbCableDialog dlg(this);
    dlg.exec();
#else
    QMessageBox::information(
        this, QStringLiteral("虚拟声卡"),
        QStringLiteral("虚拟声卡功能在 Linux（PipeWire）与 Windows（VB-CABLE）提供。"));
#endif
#endif
}

void MainWindow::createTray() {
    if (!QSystemTrayIcon::isSystemTrayAvailable()) return;
    trayIcon_ = new QSystemTrayIcon(this);
    trayIcon_->setIcon(windowIcon());
    trayMenu_ = new QMenu(this);
    trayMenu_->addAction(QStringLiteral("退出"), this, &MainWindow::quitApp);
    trayIcon_->setContextMenu(trayMenu_);
    connect(trayIcon_, &QSystemTrayIcon::activated, this,
            [this](QSystemTrayIcon::ActivationReason reason) {
                if (reason == QSystemTrayIcon::Trigger) {
                    if (isVisible()) hide();
                    else {
                        show();
                        activateWindow();
                    }
                }
            });
    trayIcon_->show();
}

void MainWindow::startWatchdog() {
    watchdogTimer_ = new QTimer(this);
    watchdogTimer_->setInterval(3000);
    connect(watchdogTimer_, &QTimer::timeout, this, [this]() {
        if (!processing_ || !thread_) return;
        // 线程意外结束（run 返回且非用户停止）→ 自动重启
        if (thread_->isFinished()) {
            delete thread_;
            thread_ = nullptr;
            processing_ = false;
            startBtn_->setText(QStringLiteral("启动音频处理"));
            // 延迟重启一次
            QTimer::singleShot(500, this, [this]() {
                if (!processing_) onStartStop();
            });
        }
    });
    watchdogTimer_->start();

    // AGC 增益滑块跟随轮询（16ms）
    agcPollTimer_ = new QTimer(this);
    agcPollTimer_->setInterval(16);
    connect(agcPollTimer_, &QTimer::timeout, this, &MainWindow::updateAgcSlider);
}

void MainWindow::updateAgcSlider() {
    if (!agcCb_->isChecked() || !processing_) return;
    double db = engine_.agcGainDb();
    int v = (int)db;
    if (v != preGainSlider_->value()) {
        preGainSlider_->blockSignals(true);
        preGainSlider_->setValue(v);
        preGainSlider_->blockSignals(false);
        preGainLabel_->setText(QString("%1 dB").arg(db, 0, 'f', 0));
    }
}

void MainWindow::updateRunningState() {
    // 更新窗口/托盘图标与提示（运行=亮，停止=暗）
    QString tip = processing_ ? QStringLiteral("PureVox - 运行中") : QStringLiteral("PureVox - 未运行");
    if (trayIcon_) {
        trayIcon_->setToolTip(tip);
    }
    statusLabel_->setText(processing_ ? QStringLiteral("运行中…")
                                      : QStringLiteral("PureVox 引擎就绪"));
}

void MainWindow::quitApp() {
    if (trayIcon_) trayIcon_->hide();
    if (thread_) {
        thread_->stop();
        delete thread_;
        thread_ = nullptr;
    }
    QApplication::quit();
}

void MainWindow::applyTheme(const QString &mode) {
    bool dark;
    if (mode == QStringLiteral("light")) dark = false;
    else if (mode == QStringLiteral("dark")) dark = true;
    else {
        // 系统：按调色板亮度判定
        dark = QApplication::palette().window().color().lightness() < 128;
    }
    if (dark) {
        QPalette p;
        p.setColor(QPalette::Window, QColor(45, 45, 48));
        p.setColor(QPalette::WindowText, QColor(220, 220, 220));
        p.setColor(QPalette::Base, QColor(30, 30, 32));
        p.setColor(QPalette::AlternateBase, QColor(42, 42, 45));
        p.setColor(QPalette::Text, QColor(220, 220, 220));
        p.setColor(QPalette::Button, QColor(55, 55, 58));
        p.setColor(QPalette::ButtonText, QColor(220, 220, 220));
        p.setColor(QPalette::Highlight, QColor(0, 122, 204));
        p.setColor(QPalette::HighlightedText, Qt::white);
        p.setColor(QPalette::PlaceholderText, QColor(150, 150, 150));
        QApplication::setPalette(p);
    } else {
        QApplication::setPalette(QApplication::style()->standardPalette());
    }
}

void MainWindow::onEqSlot(int slot) {
    if (updatingEqUi_ || slot < 0 || slot >= eqSlotButtons_.size()) return;
    // 保存当前展示的到旧槽位
    eqPresets_[eqActiveSlot_] = eqCurve_->gains();
    eqActiveSlot_ = slot;
    // 加载新槽位
    if (slot < eqPresets_.size()) {
        eqCurve_->setGains(eqPresets_[slot]);
        engine_.applyEqGains(eqPresets_[slot]);
        saveConfig();
    }
    updateEqButtons();
}

void MainWindow::onEqPreset(const QVector<double> &gains) {
    if (gains.isEmpty()) return;
    eqPresets_[eqActiveSlot_] = gains;
    eqCurve_->setGains(gains);
    engine_.applyEqGains(gains);
    saveConfig();
}

void MainWindow::saveEqPreset(int slot) {
    if (slot >= 0 && slot < eqPresets_.size()) {
        eqPresets_[slot] = eqCurve_->gains();
    }
}

void MainWindow::loadEqConfig() {
    QSettings s;
    eqPresets_.resize(8);
    int eqBands = pv::Processor::eqBandCount();
    for (int i = 0; i < 8; ++i) {
        eqPresets_[i].resize(eqBands, 0.0);
        for (int b = 0; b < eqBands; ++b) {
            eqPresets_[i][b] = s.value(QString("eq_preset_%1_band_%2").arg(i).arg(b), 0.0).toDouble();
        }
    }
    // 恢复当前 EQ 曲线（活动槽位）
    QVector<double> eq;
    for (int b = 0; b < eqBands; ++b) {
        eq.append(s.value(QString("eq_band_%1").arg(b), 0.0).toDouble());
    }
    eqCurve_->setGains(eq);
    engine_.applyEqGains(eq);
}

void MainWindow::updateEqButtons() {
    if (updatingEqUi_) return;
    updatingEqUi_ = true;
    for (int i = 0; i < eqSlotButtons_.size(); ++i) {
        QString txt = eqSlotButtons_[i]->text();
        // 简单高亮当前活动槽位
        eqSlotButtons_[i]->setProperty("active", i == eqActiveSlot_);
        eqSlotButtons_[i]->style()->unpolish(eqSlotButtons_[i]);
        eqSlotButtons_[i]->style()->polish(eqSlotButtons_[i]);
    }
    updatingEqUi_ = false;
}

void MainWindow::onEqChanged(const QVector<double> &gains) {
    eqPresets_[eqActiveSlot_] = gains;
    engine_.applyEqGains(gains);
    saveConfig();
}

void MainWindow::initEngine() {    if (!QFileInfo::exists(kModelDenoise)) {
        statusLabel_->setText(QStringLiteral("未找到模型文件，请从仓库根目录运行"));
        return;
    }
    QString err;
    if (!engine_.init(QString::fromUtf8(kModelDenoise), QString::fromUtf8(kModelTse),
                      QString::fromUtf8(kModelAec), &err)) {
        statusLabel_->setText(err);
        return;
    }
    int be = engine_.backendEffective();
    QString backendName;
    if (be == 0) backendName = QStringLiteral("AVX");
    else if (be == 1) backendName = QStringLiteral("SSE");
    else backendName = QStringLiteral("NPU");
    statusLabel_->setText(QStringLiteral("PureVox 引擎就绪（推理后端: %1）").arg(backendName));
}

void MainWindow::refreshDevices() {
    QStringList srcs = DeviceEnum::listSources();
    QStringList dsts = DeviceEnum::listDestinations();

    inputCombo_->clear();
    for (const QString &s : srcs) {
        inputCombo_->addItem(DeviceEnum::sourceLabel(s), s);
    }
    outputCombo_->clear();
    for (const QString &d : dsts) {
        outputCombo_->addItem(DeviceEnum::destLabel(d), d);
    }
    monitorCombo_->clear();
    for (const QString &d : dsts) {
        monitorCombo_->addItem(DeviceEnum::destLabel(d), d);
    }

    // 恢复上次选择
    QSettings s;
    QString lastIn = s.value("input_device").toString();
    QString lastOut = s.value("output_device").toString();
    int ix = inputCombo_->findData(lastIn);
    if (ix >= 0) inputCombo_->setCurrentIndex(ix);
    ix = outputCombo_->findData(lastOut);
    if (ix >= 0) outputCombo_->setCurrentIndex(ix);
}

void MainWindow::loadConfig() {
    QSettings s;
    mode_ = s.value("mode", kModeDenoise).toInt();
    preGain_ = s.value("pre_gain_db", 0.0).toDouble();
    postGain_ = s.value("post_gain_db", 0.0).toDouble();
    preGainSlider_->setValue((int)preGain_);
    preGainLabel_->setText(QString("%1 dB").arg(preGain_, 0, 'f', 0));
    postGainSlider_->setValue((int)postGain_);
    postGainLabel_->setText(QString("%1 dB").arg(postGain_, 0, 'f', 0));
    engine_.setPreGain(preGain_);
    engine_.setPostGain(postGain_);

    compCb_->setChecked(s.value("compressor_enabled", false).toBool());
    agcCb_->setChecked(s.value("agc_enabled", false).toBool());
    vadCb_->setChecked(s.value("vad_enabled", false).toBool());
    monitorCb_->setChecked(s.value("monitor_enabled", false).toBool());
    engine_.setCompressorEnabled(compCb_->isChecked());
    engine_.setAgcEnabled(agcCb_->isChecked(), 0.0);
    engine_.setVadEnabled(vadCb_->isChecked());

    // EQ 配置（含 8 槽位）
    loadEqConfig();

    static_cast<SegmentedControl *>(segWidget_)->setValue(mode_);
    updateModeUi();
}

void MainWindow::saveConfig() {
    QSettings s;
    s.setValue("mode", mode_);
    s.setValue("pre_gain_db", preGain_);
    s.setValue("post_gain_db", postGain_);
    s.setValue("compressor_enabled", compCb_->isChecked());
    s.setValue("agc_enabled", agcCb_->isChecked());
    s.setValue("vad_enabled", vadCb_->isChecked());
    s.setValue("monitor_enabled", monitorCb_->isChecked());
    if (inputCombo_->currentData().isValid())
        s.setValue("input_device", inputCombo_->currentData().toString());
    if (outputCombo_->currentData().isValid())
        s.setValue("output_device", outputCombo_->currentData().toString());
    QVector<double> eq = eqCurve_->gains();
    for (int i = 0; i < eq.size(); ++i)
        s.setValue(QString("eq_band_%1").arg(i), eq[i]);
    // 保存 8 个 EQ 槽位
    for (int slot = 0; slot < eqPresets_.size(); ++slot) {
        for (int b = 0; b < eqPresets_[slot].size(); ++b) {
            s.setValue(QString("eq_preset_%1_band_%2").arg(slot).arg(b), eqPresets_[slot][b]);
        }
    }
}

void MainWindow::updateModeUi() {
    bool inAec = mode_ == kModeAec;
    refBtn_->setVisible(mode_ == kModeTse);
    monitorCb_->setText(inAec ? QStringLiteral("AEC") : QStringLiteral("监听"));
    monitorCb_->setEnabled(!inAec);
}

void MainWindow::onModeChanged(int val) {
    if (val == mode_) return;
    int old = mode_;
    mode_ = val;
    saveConfig();
    saveGains(old);
    loadGains(mode_);
    updateModeUi();
    applyMode(old);
}

void MainWindow::applyMode(int oldMode) {
    // 未运行时：参数保存即可（启动时会 replay）
    if (!processing_) {
        engine_.setMode(mode_);
        engine_.setPreGain(preGain_);
        engine_.setPostGain(postGain_);
        return;
    }
    // 运行中切换模式：异步停止（销毁处理器/桥）→ 延迟重启（重载模型）
    // 异步避免同步 close+new PipeWire 桥竞态（旧桥未完全释放就重建）
    if (oldMode != mode_) {
        onStartStop();  // 停止 + cleanup
        QTimer::singleShot(300, this, [this]() {
            if (!processing_) onStartStop();  // 启动 + init + replay
        });
    }
}

void MainWindow::saveGains(int mode) {
    QSettings s;
    s.setValue(QString("pre_gain_%1").arg(mode), preGain_);
    s.setValue(QString("post_gain_%1").arg(mode), postGain_);
}

void MainWindow::loadGains(int mode) {
    QSettings s;
    double g = s.value(QString("pre_gain_%1").arg(mode), 0.0).toDouble();
    preGain_ = g;
    preGainSlider_->setValue((int)g);
    preGainLabel_->setText(QString("%1 dB").arg(g, 0, 'f', 0));
    engine_.setPreGain(g);
    double pg = s.value(QString("post_gain_%1").arg(mode), 0.0).toDouble();
    postGain_ = pg;
    postGainSlider_->setValue((int)pg);
    postGainLabel_->setText(QString("%1 dB").arg(pg, 0, 'f', 0));
    engine_.setPostGain(pg);
}

void MainWindow::runLifecycleSelfTest() {
    // 与真实用户一致：启动(直通) → 运行中切降噪(applyMode 重启) → 停止
    setModeInternal(0);
    onStartStop();  // 启动直通
    QTimer::singleShot(2000, this, [this]() {
        setModeInternal(1);  // 运行中切降噪（applyMode 内部 stop+reinit+start）
        QTimer::singleShot(2000, this, [this]() {
            onStartStop();  // 停止
        });
    });
}

void MainWindow::setModeInternal(int m) {
    if (m == mode_) return;
    int old = mode_;
    mode_ = m;
    saveConfig();
    saveGains(old);
    loadGains(mode_);
    updateModeUi();
    applyMode(old);
    static_cast<SegmentedControl *>(segWidget_)->setValue(mode_);
}

void MainWindow::onStartStop() {
    if (processing_) {
        if (thread_) {
            thread_->stop();
            delete thread_;
            thread_ = nullptr;
        }
        if (netServer_) {
            netServer_->stop();
            delete netServer_;
            netServer_ = nullptr;
        }
        processing_ = false;
        // 停止：销毁处理器（对齐 Python cleanup）
        engine_.cleanup();
        if (agcPollTimer_) agcPollTimer_->stop();
        startBtn_->setText(QStringLiteral("启动音频处理"));
        statusLabel_->setText(QStringLiteral("已停止"));
        updateRunningState();
        return;
    }

    // 启动：重建处理器并重放全部参数（对齐 Python 每次启动重建）
    if (!engine_.ready()) {
        QString err;
        if (!engine_.init(QString::fromUtf8(kModelDenoise), QString::fromUtf8(kModelTse),
                          QString::fromUtf8(kModelAec), &err)) {
            QMessageBox::warning(this, QStringLiteral("PureVox"), err);
            return;
        }
    }
    engine_.replayParams(mode_, preGain_, postGain_, eqCurve_->gains(),
                         compCb_->isChecked(), agcCb_->isChecked(), vadCb_->isChecked(),
                         monitorCb_->isChecked());
    QString input = inputCombo_->currentData().toString();
    QString output = outputCombo_->currentData().toString();

    int api = apiCombo_->currentData().toInt();
    // 网络推流模式：启动接收服务器，不采集本地麦克风
    if (api == kApiNetwork) {
        netServer_ = new NetworkServer(this);
        netServer_->setOutputSink(output);
        QString err;
        if (!netServer_->start(8443, &err)) {
            QMessageBox::warning(this, QStringLiteral("PureVox"),
                                 QStringLiteral("网络服务器启动失败: %1").arg(err));
            delete netServer_;
            netServer_ = nullptr;
            return;
        }
        processing_ = true;
        startBtn_->setText(QStringLiteral("停止"));
        statusLabel_->setText(QStringLiteral("网络推流监听中 (ws://本机:8443/ws/audio)"));
        updateRunningState();
        saveConfig();
        return;
    }

    if (input.isEmpty() || output.isEmpty()) {
        QMessageBox::warning(this, QStringLiteral("PureVox"),
                             QStringLiteral("请选择输入与输出设备"));
        return;
    }
    QString monitor = monitorCb_->isChecked() && !monitorCombo_->currentData().isNull()
                          ? monitorCombo_->currentData().toString()
                          : QString();

    // 根据音频接口选择后端（AudioThread 接管所有权，run 结束自行删除）
    AudioBackend *backend = nullptr;
#ifdef Q_OS_LINUX
    if (api == kApiPipeWire) {
        backend = new PwBackend();
    } else if (api == kApiAlsa) {
        backend = new AlsaBackend();
    } else {
        QMessageBox::warning(this, QStringLiteral("PureVox"),
                             QStringLiteral("未知音频接口"));
        return;
    }
#else
    QMessageBox::warning(this, QStringLiteral("PureVox"),
                         QStringLiteral("Windows 音频后端（WASAPI）尚未实现，请使用 Linux 版本。"));
    return;
#endif

    thread_ = new AudioThread(&engine_, backend, input, output, monitor, mode_, this);
    connect(thread_, &AudioThread::levelUpdated, vuBar_, &VUBar::updateLevelDb);
    connect(thread_, &AudioThread::spectrumData, this,
            [this](const QVector<float> &in, const QVector<float> &out) {
                spectrum_->updateSpectrum(in.constData(), in.size(), out.constData(), out.size());
            });
    connect(thread_, &AudioThread::errorOccurred, this,
            [this](const QString &msg) { QMessageBox::critical(this, QStringLiteral("PureVox"), msg); });
    connect(thread_, &AudioThread::finished, this, [this]() {
        if (processing_) {
            processing_ = false;
            startBtn_->setText(QStringLiteral("启动音频处理"));
            statusLabel_->setText(QStringLiteral("已停止"));
            updateRunningState();
        }
    });
    processing_ = true;
    if (agcCb_->isChecked() && agcPollTimer_) agcPollTimer_->start();
    startBtn_->setText(QStringLiteral("停止"));
    statusLabel_->setText(QStringLiteral("运行中…"));
    updateRunningState();
    thread_->start();
    saveConfig();
}
