// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "spectrum.h"

#include <QPainter>
#include <QPen>
#include <QTimer>

#include <algorithm>
#include <cmath>

namespace {
constexpr double kDbRng = SpectrumWidget::kDbMax - SpectrumWidget::kDbMin;// 频率刻度（硬编码像素偏移：bar_w=3, gap=1, 从 L 起算）
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

// 累积输入帧，滚动到最近 maxLen 个；返回最近 fftSize 个（若够）
static bool accumulate(std::vector<float> &acc, const float *data, int n, int fftSize,
                       std::vector<float> &out) {
    if (n <= 0) return false;
    acc.insert(acc.end(), data, data + n);
    // 滚动到最近 fftSize*2，避免无限增长
    if ((int)acc.size() > fftSize * 2)
        acc.erase(acc.begin(), acc.end() - fftSize * 2);
    if ((int)acc.size() < fftSize) return false;
    out.resize(fftSize);
    std::copy_n(acc.end() - fftSize, fftSize, out.begin());
    return true;
}

void SpectrumWidget::updateSpectrum(const float *input, int inN, const float *output,
                                    int outN) {
    bool updated = false;
    // 输入帧累积 → 达 FFT_SIZE 算输入 Mel 频谱
    if (accumulate(inputAccum_, input, inN, kFftSize, fftBuf_)) {
        // 去 DC（去除麦克风低频漂移，避免频谱只有低频）
        float mean = 0.0f;
        for (auto v : fftBuf_) mean += v;
        mean /= fftBuf_.size();
        for (auto &v : fftBuf_) v -= mean;
        QVector<float> spec(kNumBands);
        size_t got = mel_.compute(fftBuf_.data(), kFftSize, spec.data());
        if (got > 0) {
            for (int i = 0; i < (int)got && i < kNumBands; ++i) inputBands_[i] = spec[i];
        }
        // 留半重叠
        if ((int)inputAccum_.size() > kFftSize / 2)
            inputAccum_.erase(inputAccum_.begin(), inputAccum_.end() - kFftSize / 2);
        updated = true;
    }
    // 输出帧累积 → 达 FFT_SIZE 算输出 Mel 频谱
    if (accumulate(outputAccum_, output, outN, kFftSize, fftBuf_)) {
        float mean = 0.0f;
        for (auto v : fftBuf_) mean += v;
        mean /= fftBuf_.size();
        for (auto &v : fftBuf_) v -= mean;
        QVector<float> spec(kNumBands);
        size_t got = mel_.compute(fftBuf_.data(), kFftSize, spec.data());
        if (got > 0) {
            for (int i = 0; i < (int)got && i < kNumBands; ++i) outputBands_[i] = spec[i];
        }
        if ((int)outputAccum_.size() > kFftSize / 2)
            outputAccum_.erase(outputAccum_.begin(), outputAccum_.end() - kFftSize / 2);
        updated = true;
    }

    if (!updated) return;

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
            if (isVisible()) update();  // 仅窗口可见时重绘
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
