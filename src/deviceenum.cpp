// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "deviceenum.h"

#include <QtGlobal>

// ── 平台头文件 ──
#ifdef Q_OS_WIN
#include <windows.h>
#include <mmdeviceapi.h>
#include <functiondiscoverykeys_devpkey.h>
#endif  // Q_OS_WIN

#ifdef Q_OS_LINUX
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QVector>

#include <algorithm>
#endif  // Q_OS_LINUX

// ── 平台私有实现（各自独立块，不混排）──

#ifdef Q_OS_LINUX
namespace {

struct NodeInfo {
    QString name;
    QString description;
    QString mediaClass;
    QString state;
};

// 解析 `pw-dump` JSON，返回节点列表
QVector<NodeInfo> listNodes() {
    QVector<NodeInfo> out;
    QProcess proc;
    proc.start("pw-dump", QStringList());
    if (!proc.waitForFinished(5000)) return out;
    QByteArray raw = proc.readAllStandardOutput();
    QJsonParseError err;
    QJsonDocument doc = QJsonDocument::fromJson(raw, &err);
    if (err.error != QJsonParseError::NoError) return out;
    if (!doc.isArray()) return out;

    for (const QJsonValue &v : doc.array()) {
        if (!v.isObject()) continue;
        QJsonObject obj = v.toObject();
        QJsonObject info = obj.value("info").toObject();
        QJsonObject props = info.value("props").toObject();
        QString name = props.value("node.name").toString();
        if (name.isEmpty()) continue;
        NodeInfo n;
        n.name = name;
        n.description = props.value("node.description").toString();
        n.mediaClass = props.value("media.class").toString();
        n.state = info.value("state").toString();
        if (!n.mediaClass.isEmpty()) out.append(n);
    }
    return out;
}

// 按媒体类别过滤节点名
QStringList nodeNamesOfClass(const QString &mediaClass) {
    QStringList out;
    for (const NodeInfo &n : listNodes()) {
        if (n.mediaClass != mediaClass) continue;
        if (n.name.isEmpty() || n.name.startsWith("PureVox-")) continue;
        if (!out.contains(n.name)) out.append(n.name);
    }
    return out;
}

QString describeNode(const QString &name) {
    if (name == "purevox_out.monitor") return "PureVox out";
    for (const NodeInfo &n : listNodes())
        if (n.name == name) return n.description.isEmpty() ? name : n.description;
    return name;
}

}  // namespace
#endif  // Q_OS_LINUX

#ifdef Q_OS_WIN
#include <mmsystem.h>

namespace {

// MME 设备枚举：返回设备 friendly name（供 MmeBackend 按名称匹配）
QStringList mmeEndpointNames(bool capture) {
    QStringList out;
    if (capture) {
        UINT n = waveInGetNumDevs();
        for (UINT i = 0; i < n; ++i) {
            WAVEINCAPS caps;
            if (waveInGetDevCaps(i, &caps, sizeof(caps)) == MMSYSERR_NOERROR)
                out.append(QString::fromWCharArray(caps.szPname));
        }
    } else {
        UINT n = waveOutGetNumDevs();
        for (UINT i = 0; i < n; ++i) {
            WAVEOUTCAPS caps;
            if (waveOutGetDevCaps(i, &caps, sizeof(caps)) == MMSYSERR_NOERROR)
                out.append(QString::fromWCharArray(caps.szPname));
        }
    }
    return out;
}

// WASAPI MMDevice 枚举：返回设备 friendly name（供 WasapiBackend 按名称匹配）
QStringList endpointNames(bool capture) {
    QStringList out;
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    IMMDeviceEnumerator *en = nullptr;
    if (CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                         __uuidof(IMMDeviceEnumerator), (void **)&en) != S_OK)
        return out;
    IMMDeviceCollection *coll = nullptr;
    if (en->EnumAudioEndpoints(capture ? eCapture : eRender, DEVICE_STATE_ACTIVE, &coll) == S_OK) {
        UINT count = 0;
        coll->GetCount(&count);
        for (UINT i = 0; i < count; ++i) {
            IMMDevice *d = nullptr;
            if (coll->Item(i, &d) != S_OK) continue;
            IPropertyStore *ps = nullptr;
            if (d->OpenPropertyStore(STGM_READ, &ps) == S_OK) {
                PROPVARIANT pv;
                PropVariantInit(&pv);
                if (ps->GetValue(PKEY_Device_FriendlyName, &pv) == S_OK &&
                    pv.vt == VT_LPWSTR && pv.pwszVal) {
                    QString n = QString::fromWCharArray(pv.pwszVal);
                    if (!out.contains(n)) out.append(n);
                    PropVariantClear(&pv);
                }
                ps->Release();
            }
            d->Release();
        }
        coll->Release();
    }
    en->Release();
    return out;
}

}  // namespace
#endif  // Q_OS_WIN

// ── 公共接口 ──

namespace DeviceEnum {

QStringList listSources(Api api) {
#ifdef Q_OS_WIN
    if (api == Api::Mme) return mmeEndpointNames(true);
    return endpointNames(true);
#else
    (void)api;
    QStringList out = nodeNamesOfClass("Audio/Source");
    // 排除虚拟麦克风与错误节点
    out.erase(std::remove_if(out.begin(), out.end(),
                             [](const QString &n) {
                                 return n.startsWith("purevox") ||
                                        n == "purevox_mic";
                             }),
              out.end());
    return out;
#endif
}

QStringList listDestinations(Api api) {
#ifdef Q_OS_WIN
    if (api == Api::Mme) return mmeEndpointNames(false);
    return endpointNames(false);
#else
    (void)api;
    return nodeNamesOfClass("Audio/Sink");
#endif
}

QString nodeDescription(const QString &name) {
#ifdef Q_OS_WIN
    // Windows：设备标识即 friendly name
    return name;
#else
    return describeNode(name);
#endif
}

QString sourceLabel(const QString &name) {
#ifdef Q_OS_WIN
    return QStringLiteral("麦克风 · %1").arg(name);
#else
    if (name == "purevox_mic") return "PureVox mic（虚拟麦克风）";
    if (name.startsWith("purevox")) return "PureVox out";
    return QStringLiteral("麦克风 · %1").arg(nodeDescription(name));
#endif
}

QString destLabel(const QString &name) {
#ifdef Q_OS_WIN
    return QStringLiteral("播放 · %1").arg(name);
#else
    if (name == "purevox_out") return "PureVox out（默认）";
    return QStringLiteral("播放 · %1").arg(nodeDescription(name));
#endif
}

}  // namespace DeviceEnum
