// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "virtualmic.h"

#include <QProcess>
#include <QThread>

namespace {

const char *kSink = "purevox_out";
const char *kSource = "purevox_out.monitor";
const char *kMic = "purevox_mic";
const char *kSinkLabel = "PureVox out";
const char *kMicLabel = "PureVox mic";

// 返回指定 node.name 的本地 object id；不存在返回 -1
int pwNodeId(const QString &nodeName) {
    QProcess proc;
    proc.start("pw-cli", {"ls", "Node"});
    if (!proc.waitForFinished(5000)) return -1;
    QString out = QString::fromUtf8(proc.readAllStandardOutput());
    int curId = -1;
    for (const QString &line : out.split('\n')) {
        QString s = line.trimmed();
        if (s.startsWith("id ") && s.contains("Node")) {
            QStringList parts = s.split(' ');
            if (parts.size() >= 2) {
                QString id = parts[1];
                id.chop(1);  // 去掉逗号
                curId = id.toInt();
            }
        } else if (s.contains(nodeName)) {
            return curId;
        }
    }
    return -1;
}

bool runCommand(const QString &prog, const QStringList &args) {
    QProcess proc;
    proc.start(prog, args);
    return proc.waitForFinished(10000) && proc.exitStatus() == QProcess::NormalExit;
}

}  // namespace

namespace VirtualMic {

bool ready() { return pwNodeId(kSink) >= 0; }

bool ensure() {
    // 清理旧架构残留 loopback
    QProcess pkill;
    pkill.start("pkill", {"-9", "-x", "pw-loopback"});
    pkill.waitForFinished(5000);

    if (pwNodeId(kSink) < 0) {
        QString args = QString("{ factory.name=support.null-audio-sink node.name=%1 "
                               "media.class=Audio/Sink object.linger=true "
                               "audio.position=[MONO] monitor.mode=disabled "
                               "node.description=\"%2\" }")
                           .arg(kSink, kSinkLabel);
        if (!runCommand("pw-cli", {"create-node", "adapter", args})) return false;
        QThread::msleep(500);
    }
    if (!ready()) return false;

    // 设为默认 sink
    runCommand("pactl", {"set-default-sink", kSink});

    // 建真源 purevox_mic（module-remap-source）
    if (pwNodeId(kMic) < 0) {
        QString props = QString("device.description=%1").arg(kMicLabel);
        runCommand("pactl", {"load-module", "module-remap-source",
                             "master=" + QString(kSource),
                             "source_name=" + QString(kMic),
                             "channel_map=mono",
                             "source_properties=" + props});
        QThread::msleep(600);
    }
    return true;
}

bool remove() {
    // 卸载真源（按 module 列表定位 module-remap-source）
    QProcess list;
    list.start("pactl", {"list", "short", "modules"});
    if (list.waitForFinished(5000)) {
        for (const QString &line : QString::fromUtf8(list.readAllStandardOutput()).split('\n')) {
            if (line.contains(kMic)) {
                QString mid = line.trimmed().split(' ').first();
                runCommand("pactl", {"unload-module", mid});
                break;
            }
        }
    }
    // 防御：真源节点残留直接 destroy
    int micId = pwNodeId(kMic);
    if (micId >= 0) runCommand("pw-cli", {"destroy", QString::number(micId)});
    QThread::msleep(300);
    // destroy sink 节点
    int id = pwNodeId(kSink);
    if (id >= 0) runCommand("pw-cli", {"destroy", QString::number(id)});
    return true;
}

}  // namespace VirtualMic
