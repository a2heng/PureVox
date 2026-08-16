// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DEVICEENUM_H
#define PUREVOX_DEVICEENUM_H

#include <QtGlobal>

#include <QString>
#include <QStringList>

// 设备枚举：Windows 下 Wasapi/Mme 用各自的底层 API；Linux 下 PipeWire/Alsa 共用
namespace DeviceEnum {

// 音频接口（对应 UI「音频接口」下拉与后端选择）
enum class Api {
    PipeWire,
    Alsa,
    Wasapi,
    Mme,
};

// 输入 = 物理麦克风，排除 PureVox 自身流/虚拟麦克风/error
QStringList listSources(Api api);
// 输出 = 扬声器（+ purevox_out）
QStringList listDestinations(Api api);
// 节点名 → node.description（无则返回节点名）
QString nodeDescription(const QString &name);
// 输入下拉显示名
QString sourceLabel(const QString &name);
// 输出下拉显示名
QString destLabel(const QString &name);

}  // namespace DeviceEnum

#endif // PUREVOX_DEVICEENUM_H
