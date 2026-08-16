// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "dialog_vbcable.h"

#ifdef Q_OS_WIN

#include <QCheckBox>
#include <QDesktopServices>
#include <QFrame>
#include <QHBoxLayout>
#include <QLabel>
#include <QProcess>
#include <QPushButton>
#include <QSettings>
#include <QUrl>
#include <QVBoxLayout>

#include <windows.h>
#include <mmdeviceapi.h>
#include <functiondiscoverykeys_devpkey.h>
#include <objbase.h>

#include <QDebug>

namespace {
const char *kDlUrl = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";
const char *kTutUrl = "https://www.bilibili.com/video/BV1i2bazGEKe/";
}  // namespace

// 用 Windows Core Audio（MMDevice）枚举端点，检查 CABLE Input/Output 是否出现
bool vbcableInstalled() {
    bool hasInput = false, hasOutput = false;
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    IMMDeviceEnumerator *enumerator = nullptr;
    if (CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                         __uuidof(IMMDeviceEnumerator), (void **)&enumerator) != S_OK) {
        CoUninitialize();
        return false;
    }
    // 枚举所有端点（render + capture）
    IMMDeviceCollection *devices = nullptr;
    if (enumerator->EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE, &devices) == S_OK) {
        UINT count = 0;
        devices->GetCount(&count);
        for (UINT i = 0; i < count; ++i) {
            IMMDevice *dev = nullptr;
            if (devices->Item(i, &dev) != S_OK) continue;
            IPropertyStore *props = nullptr;
            if (dev->OpenPropertyStore(STGM_READ, &props) == S_OK) {
                PROPVARIANT name;
                PropVariantInit(&name);
                if (props->GetValue(PKEY_Device_FriendlyName, &name) == S_OK &&
                    name.vt == VT_LPWSTR && name.pwszVal) {
                    QString n = QString::fromWCharArray(name.pwszVal);
                    if (n.contains(QStringLiteral("CABLE Output"))) hasOutput = true;
                    if (n.contains(QStringLiteral("CABLE Input"))) hasInput = true;
                    PropVariantClear(&name);
                }
                props->Release();
            }
            dev->Release();
        }
        devices->Release();
    }
    enumerator->Release();
    CoUninitialize();
    return hasInput && hasOutput;
}

VbCableDialog::VbCableDialog(QWidget *parent) : QDialog(parent) {
    setWindowTitle(QStringLiteral("PureVox - 虚拟声卡（VB-CABLE）"));
    setMinimumWidth(460);
    setWindowModality(Qt::ApplicationModal);
    auto *layout = new QVBoxLayout(this);
    layout->setSpacing(10);
    layout->setContentsMargins(16, 16, 16, 12);

    // 状态行
    auto *stateRow = new QHBoxLayout();
    dotLabel_ = new QLabel(this);
    dotLabel_->setFixedSize(12, 12);
    stateLabel_ = new QLabel(this);
    stateLabel_->setStyleSheet(QStringLiteral("font-size: 11pt;"));
    stateRow->addWidget(dotLabel_);
    stateRow->addWidget(stateLabel_);
    stateRow->addStretch();
    layout->addLayout(stateRow);

    // 双端点说明
    auto *tips = new QLabel(
        QStringLiteral(
            "VB-CABLE 是 VB-Audio 的虚拟声卡，安装后提供一对端点，采样率均设置为 48kHz：\n"
            "\n"
            "• CABLE Input（输入端）—— 接收 PureVox 处理后的音频，经驱动转发到输出端。\n"
            "  请在 PureVox「输出设备」中选择它（本软件的输出写入这里）。\n"
            "\n"
            "• CABLE Output（输出端）—— 作为虚拟麦克风使用，可设置为系统默认麦克风，\n"
            "  供 OBS、直播、聊天、会议等软件选用。\n"
            "\n"
            "数据流向：PureVox → CABLE Input →（驱动转发）→ CABLE Output → 其它软件。"),
        this);
    tips->setWordWrap(true);
    tips->setStyleSheet(QStringLiteral("font-size: 10pt;"));
    layout->addWidget(tips);

    // 驱动卡片
    auto *cardFrame = new QFrame(this);
    cardFrame->setStyleSheet(QStringLiteral(
        "QFrame { border: 1px solid palette(mid); border-radius: 4px; background: palette(base); padding: 4px 6px; }"));
    auto *cardLayout = new QVBoxLayout(cardFrame);
    cardLayout->setSpacing(4);

    auto *guide = new QLabel(
        QStringLiteral("未检测到 VB-CABLE 驱动：请先下载官方驱动包并安装，装好后本面板会自动刷新。"),
        cardFrame);
    guide->setWordWrap(true);
    guide->setStyleSheet(QStringLiteral("font-size: 10pt; color: #cc2200;"));
    cardLayout->addWidget(guide);

    auto *row = new QHBoxLayout();
    auto *tag = new QLabel(QStringLiteral("  VB-CABLE 驱动 "), cardFrame);
    tag->setStyleSheet(QStringLiteral("border: 1px solid #888; border-radius: 3px; font-size: 9pt; padding: 1px 4px;"));
    row->addWidget(tag);
    row->addStretch();

    panelBtn_ = new QPushButton(QStringLiteral("打开控制面板"), cardFrame);
    panelBtn_->setFixedHeight(26);
    panelBtn_->setToolTip(QStringLiteral("打开 VB-CABLE 控制面板（需先安装驱动）"));
    connect(panelBtn_, &QPushButton::clicked, this, []() {
        QProcess::startDetached(QStringLiteral("control.exe"), QStringList());
    });
    row->addWidget(panelBtn_);

    auto *dlBtn = new QPushButton(QStringLiteral("下载官方驱动包"), cardFrame);
    dlBtn->setFixedHeight(26);
    connect(dlBtn, &QPushButton::clicked, this, []() {
        QDesktopServices::openUrl(QUrl(QString::fromUtf8(kDlUrl)));
    });
    row->addWidget(dlBtn);

    auto *videoBtn = new QPushButton(QStringLiteral("安装视频教程"), cardFrame);
    videoBtn->setFixedHeight(26);
    connect(videoBtn, &QPushButton::clicked, this, []() {
        QDesktopServices::openUrl(QUrl(QString::fromUtf8(kTutUrl)));
    });
    row->addWidget(videoBtn);

    cardLayout->addLayout(row);
    layout->addWidget(cardFrame);

    // 检测开关
    QSettings s;
    bool enabled = s.value("vbcable_check_enabled", true).toBool();
    checkCb_ = new QCheckBox(QStringLiteral("启动时检测虚拟麦克风（未安装才提醒）"), this);
    checkCb_->setChecked(enabled);
    checkCb_->setToolTip(QStringLiteral("默认开启检测：仅在 VB-CABLE 未安装时弹出面板。"));
    checkCb_->setStyleSheet(QStringLiteral("font-size: 10pt;"));
    layout->addWidget(checkCb_);

    auto *btnRow = new QHBoxLayout();
    btnRow->addStretch();
    auto *okBtn = new QPushButton(QStringLiteral("确定"), this);
    okBtn->setFixedHeight(28);
    okBtn->setFixedWidth(88);
    connect(okBtn, &QPushButton::clicked, this, [this]() {
        QSettings().setValue("vbcable_check_enabled", checkCb_->isChecked());
        accept();
    });
    btnRow->addWidget(okBtn);
    layout->addLayout(btnRow);

    refresh();
}

void VbCableDialog::refresh() {
    bool installed = vbcableInstalled();
    if (installed) {
        dotLabel_->setStyleSheet(QStringLiteral(
            "background: #00cc44; border-radius: 6px;"));
        stateLabel_->setText(QStringLiteral("已安装 VB-CABLE 虚拟声卡"));
    } else {
        dotLabel_->setStyleSheet(QStringLiteral(
            "background: #cc2200; border-radius: 6px;"));
        stateLabel_->setText(QStringLiteral("未检测到 VB-CABLE 驱动"));
    }
    panelBtn_->setEnabled(installed);
}

#endif // Q_OS_WIN
