// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_SPECTRUM_H
#define PUREVOX_SPECTRUM_H

#include <QWidget>

#include <QVector>

#include "dsp/melspectrum.h"

// 频谱直方图：128 段 Mel 实时输入/输出频谱重叠对比
// 降噪输出为基准，多=灰(噪声已消除)，少=亮(增强)
class SpectrumWidget : public QWidget {
    Q_OBJECT

public:
    static constexpr double kDbMin = -90.0;
    static constexpr double kDbMax = -20.0;
    static constexpr int kNumBands = 128;
    static constexpr int kFftSize = 2048;

    explicit SpectrumWidget(QWidget *parent = nullptr);

    void updateSpectrum(const float *input, int inN, const float *output, int outN);
    void reset();

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void computeBands(const float *samples, int n, QVector<double> &out, QVector<float> &accum);

    QVector<double> inputBands_;   // 原始（未平滑）
    QVector<double> outputBands_;
    QVector<double> smoothedIn_;
    QVector<double> smoothedOut_;
    QVector<float> inputAccum_;
    QVector<float> outputAccum_;
    pv::MelSpectrum mel_;
    bool pending_ = false;
};

#endif // PUREVOX_SPECTRUM_H
