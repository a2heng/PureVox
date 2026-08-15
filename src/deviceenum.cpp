// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "deviceenum.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>

#include <QVector>

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

}  // namespace

namespace DeviceEnum {

QStringList listSources() {
    QStringList out;
    for (const NodeInfo &n : listNodes()) {
        if (n.mediaClass != "Audio/Source") continue;
        if (n.name.isEmpty() || n.name.startsWith("PureVox-")) continue;
        if (n.name.startsWith("purevox")) continue;
        if (n.state == "error") continue;
        if (!out.contains(n.name)) out.append(n.name);
    }
    return out;
}

QStringList listDestinations() {
    QStringList out;
    for (const NodeInfo &n : listNodes()) {
        if (n.mediaClass != "Audio/Sink") continue;
        if (n.name.isEmpty() || n.name.startsWith("PureVox-")) continue;
        if (!out.contains(n.name)) out.append(n.name);
    }
    return out;
}

QString nodeDescription(const QString &name) {
    if (name == "purevox_out.monitor") return "PureVox out";
    for (const NodeInfo &n : listNodes()) {
        if (n.name == name) return n.description.isEmpty() ? name : n.description;
    }
    return name;
}

QString sourceLabel(const QString &name) {
    if (name == "purevox_mic") return "PureVox mic（虚拟麦克风）";
    if (name.startsWith("purevox")) return "PureVox out";
    return QString("麦克风 · %1").arg(nodeDescription(name));
}

QString destLabel(const QString &name) {
    if (name == "purevox_out") return "PureVox out（默认）";
    return QString("播放 · %1").arg(nodeDescription(name));
}

}  // namespace DeviceEnum
