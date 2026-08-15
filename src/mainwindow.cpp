// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "mainwindow.h"

#include <QCheckBox>
#include <QComboBox>
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
#include <QVBoxLayout>

#include <QVector>

#include "audiobackend.h"
#include "audiothread.h"
#include "alsabackend.h"
#include "deviceenum.h"
#include "dialog_about.h"
#include "eqcurve.h"
#include "pwbackend.h"
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
}  // namespace

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle("PureVox");
    setMinimumSize(720, 560);
    buildUi();
    createMenu();
    initEngine();
    loadConfig();
    refreshDevices();
}

MainWindow::~MainWindow() {
    if (thread_) {
        thread_->stop();
        delete thread_;
        thread_ = nullptr;
    }
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
    optsRow->addWidget(compCb_);
    agcCb_ = new QCheckBox(QStringLiteral("AGC"), panel);
    agcCb_->setFixedHeight(22);
    agcCb_->setToolTip(QStringLiteral("AGC 自动增益控制：自动调节增益，让声音始终稳定在合适音量。"));
    optsRow->addWidget(agcCb_);
    vadCb_ = new QCheckBox(QStringLiteral("VAD"), panel);
    vadCb_->setFixedHeight(22);
    vadCb_->setToolTip(QStringLiteral("VAD 语音活性检测：不说话时自动静音，消除残留噪声。"));
    optsRow->addWidget(vadCb_);
    optsRow->addStretch();
    refBtn_ = new QPushButton(QStringLiteral("参考音频…"), panel);
    refBtn_->setFixedHeight(22);
    refBtn_->setMinimumWidth(86);
    refBtn_->setVisible(false);
    optsRow->addWidget(refBtn_);
    panelLayout->addLayout(optsRow);

    // 第3行：前增益
    auto *gainGrid = new QGridLayout();
    gainGrid->setSpacing(4);
    auto *lblPre = new QLabel(QStringLiteral("前增益"), panel);
    lblPre->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    lblPre->setToolTip(QStringLiteral(
        "前增益 — 麦克风输入增益\n"
        "提高前增益 → 信号更强，降噪效果更好。\n"
        "但过高会导致削波失真。\n"
        "建议: 正常说话时峰值在 -12 ~ -6 dBFS 为最佳。"));
    gainGrid->addWidget(lblPre, 0, 0);
    preSlider_ = new QSlider(Qt::Horizontal, panel);
    preSlider_->setRange(-30, 30);
    preSlider_->setValue((int)preGain_);
    gainGrid->addWidget(preSlider_, 0, 1);
    preLabel_ = new QLabel("+0 dB", panel);
    preLabel_->setFixedWidth(44);
    preLabel_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    gainGrid->addWidget(preLabel_, 0, 2);
    connect(preSlider_, &QSlider::valueChanged, this, [this](int v) {
        preGain_ = v;
        preLabel_->setText(QString("%1 dB").arg((double)v));
        engine_.setPreGain(preGain_);
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
    connect(quitBtn_, &QPushButton::clicked, this, &QWidget::close);
    btnRow->addWidget(quitBtn_);
    panelLayout->addLayout(btnRow);

    statusLabel_ = new QLabel(QStringLiteral("PureVox"), panel);
    statusLabel_->setAlignment(Qt::AlignCenter);
    panelLayout->addWidget(statusLabel_);

    // 面板固定在左列
    panel->setFixedWidth(520);
    topLayout->addWidget(panel, 0);

    // 右列：频谱图
    spectrum_ = new SpectrumWidget(topContainer);
    spectrum_->setFixedWidth(460);
    topLayout->addWidget(spectrum_, 0);
    root_->addWidget(topContainer);

    // EQ 面板（下方横贯）
    eqCurve_ = new EQCurveWidget(central_);
    root_->addWidget(eqCurve_);
    connect(eqCurve_, &EQCurveWidget::gainsChanged, this, [this](const QVector<double> &g) {
        engine_.applyEqGains(g);
        saveConfig();
    });

    setCentralWidget(central_);
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
            // 简化：仅保存，主题应用由系统调色板驱动
        });
    }

    QAction *snd = mb->addAction(QStringLiteral("系统声音"));
    connect(snd, &QAction::triggered, this, []() {
        QProcess::startDetached("pw-control", QStringList());
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
    QMessageBox::information(
        this, QStringLiteral("虚拟声卡"),
        QStringLiteral(
            "Linux 虚拟麦克风（PipeWire）\n\n"
            "创建：purevox_out（null-sink）+ purevox_mic（remap-source）\n"
            "本版本暂未集成虚拟声卡管理，将在后续版本实现。"));
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
    preSlider_->setValue((int)preGain_);
    preLabel_->setText(QString("%1 dB").arg(preGain_, 0, 'f', 0));
    engine_.setPreGain(preGain_);

    compCb_->setChecked(s.value("compressor_enabled", false).toBool());
    agcCb_->setChecked(s.value("agc_enabled", false).toBool());
    vadCb_->setChecked(s.value("vad_enabled", false).toBool());
    monitorCb_->setChecked(s.value("monitor_enabled", false).toBool());
    engine_.setCompressorEnabled(compCb_->isChecked());
    engine_.setAgcEnabled(agcCb_->isChecked(), 0.0);
    engine_.setVadEnabled(vadCb_->isChecked());

    // EQ 配置
    QVector<double> eq;
    int eqBands = pv::Processor::eqBandCount();
    if (eqBands > 0) {
        eq.resize(eqBands, 0.0);
        for (int i = 0; i < eqBands; ++i) {
            eq[i] = s.value(QString("eq_band_%1").arg(i), 0.0).toDouble();
        }
        eqCurve_->setGains(eq);
        engine_.applyEqGains(eq);
    }

    static_cast<SegmentedControl *>(segWidget_)->setValue(mode_);
    updateModeUi();
}

void MainWindow::saveConfig() {
    QSettings s;
    s.setValue("mode", mode_);
    s.setValue("pre_gain_db", preGain_);
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
    engine_.setMode(mode_);
    engine_.setPreGain(preGain_);
    if (processing_ && oldMode != mode_) {
        // 需重启以重载模型
        onStartStop();
        onStartStop();
        return;
    }
}

void MainWindow::saveGains(int mode) {
    QSettings s;
    s.setValue(QString("pre_gain_%1").arg(mode), preGain_);
}

void MainWindow::loadGains(int mode) {
    QSettings s;
    double g = s.value(QString("pre_gain_%1").arg(mode), 0.0).toDouble();
    preGain_ = g;
    preSlider_->setValue((int)g);
    preLabel_->setText(QString("%1 dB").arg(g, 0, 'f', 0));
    engine_.setPreGain(g);
}

void MainWindow::onStartStop() {
    if (processing_) {
        if (thread_) {
            thread_->stop();
            delete thread_;
            thread_ = nullptr;
        }
        processing_ = false;
        startBtn_->setText(QStringLiteral("启动音频处理"));
        statusLabel_->setText(QStringLiteral("已停止"));
        return;
    }

    if (!engine_.ready()) {
        QMessageBox::warning(this, QStringLiteral("PureVox"),
                             QStringLiteral("音频引擎未就绪"));
        return;
    }
    QString input = inputCombo_->currentData().toString();
    QString output = outputCombo_->currentData().toString();
    if (input.isEmpty() || output.isEmpty()) {
        QMessageBox::warning(this, QStringLiteral("PureVox"),
                             QStringLiteral("请选择输入与输出设备"));
        return;
    }
    QString monitor = monitorCb_->isChecked() && !monitorCombo_->currentData().isNull()
                          ? monitorCombo_->currentData().toString()
                          : QString();

    // 根据音频接口选择后端
    int api = apiCombo_->currentData().toInt();
    if (api == kApiPipeWire) {
        backend_ = new PwBackend();
    } else if (api == kApiAlsa) {
        backend_ = new AlsaBackend();
    } else {
        QMessageBox::warning(this, QStringLiteral("PureVox"),
                             QStringLiteral("未知音频接口"));
        return;
    }

    thread_ = new AudioThread(&engine_, backend_, input, output, monitor, mode_, this);
    connect(thread_, &AudioThread::levelUpdated, vuBar_, &VUBar::updateLevelDb);
    connect(thread_, &AudioThread::spectrumData, this,
            [this](const QVector<float> &in, const QVector<float> &out) {
                spectrum_->updateSpectrum(in.constData(), in.size(), out.constData(), out.size());
            });
    connect(thread_, &AudioThread::errorOccurred, this,
            [this](const QString &msg) { QMessageBox::critical(this, QStringLiteral("PureVox"), msg); });
    connect(thread_, &AudioThread::finished, this, [this]() {
        delete backend_;
        backend_ = nullptr;
        if (processing_) {
            processing_ = false;
            startBtn_->setText(QStringLiteral("启动音频处理"));
            statusLabel_->setText(QStringLiteral("已停止"));
        }
    });
    processing_ = true;
    startBtn_->setText(QStringLiteral("停止"));
    statusLabel_->setText(QStringLiteral("运行中…"));
    thread_->start();
    saveConfig();
}
