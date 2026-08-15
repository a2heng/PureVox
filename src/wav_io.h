// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_WAV_IO_H
#define PUREVOX_WAV_IO_H

#include <QString>

#include <cstdint>
#include <vector>

namespace WavIO {

// 把 float 单声道样本写为 16-bit PCM WAV 文件；返回是否成功
bool writeF32ToWav(const QString &path, const std::vector<float> &samples, int sampleRate);

}  // namespace WavIO

#endif // PUREVOX_WAV_IO_H
