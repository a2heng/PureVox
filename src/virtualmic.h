// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_VIRTUALMIC_H
#define PUREVOX_VIRTUALMIC_H

#include <QString>

// Linux 虚拟麦克风（PipeWire）管理：purevox_out null-sink + purevox_mic 真源
namespace VirtualMic {

// 虚拟麦克风是否已创建（purevox_out sink 存在）
bool ready();
// 确保虚拟麦克风存在（已存在则幂等不重建），返回是否可用
bool ensure();
// 清理虚拟麦克风（卸载真源 + destroy sink），返回是否成功
bool remove();

}  // namespace VirtualMic

#endif // PUREVOX_VIRTUALMIC_H
