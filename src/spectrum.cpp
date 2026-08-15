// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "spectrum.h"

#include <QPainter>
#include <QPen>
#include <QTimer>

#include <aimic.h>

#include <algorithm>
#include <cmath>

namespace {
constexpr double kDbRng = SpectrumWidget::kDbMax - SpectrumWidget::kDbMin;
// 频率刻度（硬编码像素偏移：bar_w=3, gap=1, 从 L 起算）
struct Tick {
    int px;
    const char *label;
};
const Tick kTicks[] = {{0, "20"},  {12, "100"}, {32, "200"},  {76, "500"},  {128, "1k"},
                       {200, "2k"}, {312, "5k"}, {412, "10k"}, {511, "20k"}};
}  // namespace

SpectrumWidget::SpectrumWidget(QWidget *parent)
    : QWidget(parent), inputBands_(kNumBands, kDbMin), outputBands_(kNumBands, kDbMin),
      smoothedIn_(kNumBands, kDbMin), smoothedOut_(kNumBands, kDbMin) {
    setMinimumSize(180, 160);
}

void SpectrumWidget::reset() {
    std::fill(inputBands_.begin(), inputBands_.end(), kDbMin);
    std::fill(outputBands_.begin(), outputBands_.end(), kDbMin);
    std::fill(smoothedIn_.begin(), smoothedIn_.end(), kDbMin);
    std::fill(smoothedOut_.begin(), smoothedOut_.end(), kDbMin);
    inputAccum_.clear();
    outputAccum_.clear();
    update();
}

void SpectrumWidget::computeBands(const float *samples, int n, QVector<double> &out,
                                  QVector<float> &accum) {
    if (n <= 0) return;
    for (int i = 0; i < n; ++i) accum.append(samples[i]);
    int take = (int)accum.size() - ((int)accum.size() % kFftSize);
    int count = take / kFftSize;
    for (int c = 0; c < count; ++c) {
        QVector<float> buf(accum.begin() + c * kFftSize, accum.begin() + (c + 1) * kFftSize);
        QVector<float> spec(kNumBands);
        size_t got = compute_spectrum(buf.data(), kFftSize, spec.data());
        if (got > 0) {
            for (int i = 0; i < (int)got && i < kNumBands; ++i) out[i] = spec[i];
        }
    }
    accum.erase(accum.begin(), accum.begin() + take);
}

void SpectrumWidget::updateSpectrum(const float *input, int inN, const float *output,
                                    int outN) {
    computeBands(input, inN, inputBands_, inputAccum_);
    computeBands(output, outN, outputBands_, outputAccum_);

    // 滑动平滑
    const double alpha = 0.3;
    int n = std::min({inputBands_.size(), outputBands_.size(), smoothedIn_.size(),
                      smoothedOut_.size()});
    for (int i = 0; i < n; ++i) {
        smoothedIn_[i] += alpha * (inputBands_[i] - smoothedIn_[i]);
        smoothedOut_[i] += alpha * (outputBands_[i] - smoothedOut_[i]);
    }

    if (!pending_) {
        pending_ = true;
        QTimer::singleShot(16, this, [this]() {
            pending_ = false;
            update();
        });
    }
}

void SpectrumWidget::paintEvent(QPaintEvent *) {
    int w = width(), h = height();
    if (w < 40 || h < 20) return;

    QPainter p(this);
    p.fillRect(0, 0, w, h, palette().base().color());

    const int L = 28, R = 12, T = 18, B = 16;
    const int gw = w - L - R, gh = h - T - B;
    if (gw < 20 || gh < 10) return;

    QColor grid = palette().mid().color();
    QColor textC = palette().windowText().color();
    QColor cOut = palette().highlight().color();
    QColor cMore = QColor(128, 128, 128);
    QColor cLess = QColor(0, 204, 68);

    // dB 刻度线
    QFont f = p.font();
    f.setPointSize(6);
    p.setFont(f);
    p.setPen(QPen(grid, 0.5));
    for (int db : {-90, -80, -70, -60, -50, -40, -30, -20}) {
        int y = (int)(T + gh * (1.0 - (db - kDbMin) / kDbRng));
        p.drawLine(L, y, L + gw, y);
    }

    // 频率刻度线
    f.setPointSize(5);
    p.setFont(f);
    p.setPen(QPen(grid, 0.5));
    for (const Tick &t : kTicks) {
        int x = L + t.px;
        p.drawLine(x, T, x, T + gh);
    }
    p.setPen(textC);
    for (const Tick &t : kTicks) {
        int x = L + t.px;
        QRectF r;
        if (QString::fromUtf8(t.label) == QStringLiteral("20"))
            r = QRectF(x - 20, T + gh + 1, 20, 12);
        else
            r = QRectF(x - 12, T + gh + 1, 24, 12);
        p.drawText(r, Qt::AlignCenter, QString::fromUtf8(t.label));
    }

    // 频谱条
    const double barW = 3, gap = 1, step = barW + gap;
    p.setPen(Qt::NoPen);
    for (int i = 0; i < kNumBands; ++i) {
        double inDb = std::max(kDbMin, smoothedIn_[i]);
        double outDb = std::max(kDbMin, smoothedOut_[i]);
        double inH = (inDb - kDbMin) / kDbRng * gh;
        double outH = (outDb - kDbMin) / kDbRng * gh;
        if (outH < 1 && inH < 1) continue;

        double bx = L + i * step;
        double yOut = T + gh - outH;
        double yIn = T + gh - inH;

        if (outH > 1) {
            p.setBrush(cOut);
            p.drawRect(QRectF(bx, yOut, barW, outH));
        }
        if (inDb > outDb && inH > 1) {
            p.setBrush(cMore);
            p.drawRect(QRectF(bx, yIn, barW, inH - outH));
        } else if (inDb < outDb && outH > 1) {
            p.setBrush(cLess);
            p.drawRect(QRectF(bx, yOut, barW, outH - inH));
        }
    }
    p.end();
}
