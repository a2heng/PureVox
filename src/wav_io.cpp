// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "wav_io.h"

#include <QFile>
#include <QDataStream>

#include <algorithm>
#include <cstring>

namespace WavIO {

bool writeF32ToWav(const QString &path, const std::vector<float> &samples, int sampleRate) {
    QFile f(path);
    if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate)) return false;
    QDataStream ds(&f);
    ds.setByteOrder(QDataStream::LittleEndian);

    const int channels = 1;
    const int bitsPerSample = 16;
    const int byteRate = sampleRate * channels * bitsPerSample / 8;
    const int blockAlign = channels * bitsPerSample / 8;
    const quint32 dataSize = (quint32)samples.size() * blockAlign;

    // RIFF header
    f.write("RIFF", 4);
    ds << (quint32)(36 + dataSize);
    f.write("WAVE", 4);
    // fmt chunk
    f.write("fmt ", 4);
    ds << (quint32)16;                       // fmt size
    ds << (quint16)1;                        // PCM
    ds << (quint16)channels;
    ds << (quint32)sampleRate;
    ds << (quint32)byteRate;
    ds << (quint16)blockAlign;
    ds << (quint16)bitsPerSample;
    // data chunk
    f.write("data", 4);
    ds << dataSize;
    // samples (16-bit)
    for (float s : samples) {
        float v = std::max(-1.0f, std::min(1.0f, s));
        qint16 pcm = (qint16)(v * 32767.0f);
        ds << pcm;
    }
    f.close();
    return true;
}

}  // namespace WavIO
