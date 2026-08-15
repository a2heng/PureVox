// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "eqcurve.h"

#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QWheelEvent>

#include <algorithm>
#include <cmath>

namespace {

const double kFreqs[EQCurveWidget::kNumBands] = {
    20, 22.4, 25, 28, 31.5, 35.5, 40, 45, 50, 56, 63, 71, 80, 90, 100, 112, 125, 140, 160,
    180, 200, 224, 250, 280, 315, 355, 400, 450, 500, 560, 630, 710, 800, 900, 1000, 1120,
    1250, 1400, 1600, 1800, 2000, 2240, 2500, 2800, 3150, 3550, 4000, 4500, 5000, 5600, 6300,
    7100, 8000, 9000, 10000, 11200, 12500, 14000, 16000, 18000, 20000,
};

const double kLogMin = std::log10(20.0);
const double kLogMax = std::log10(20000.0);

QString formatFreq(double f) {
    if (f >= 1000) return QString("%1k").arg(f / 1000.0, 0, 'g', f >= 10000 ? 2 : 3);
    return QString::number((int)f);
}

}  // namespace

EQCurveWidget::EQCurveWidget(QWidget *parent) : QWidget(parent), values_(kNumBands, 0.0) {
    setMinimumSize(480, 200);
}

QVector<double> EQCurveWidget::gains() const { return values_; }

void EQCurveWidget::setGains(const QVector<double> &gains) {
    values_ = gains;
    if (values_.size() != kNumBands) values_.resize(kNumBands, 0.0);
    update();
}

double EQCurveWidget::freqX(double freq, int gw) const {
    // 与 Python 一致：3% 边距 + 对数映射
    const double m = 0.03;
    double u = gw * (1 - 2 * m);
    double n = (std::log10(freq) - std::log10(20.0)) / (std::log10(20000.0) - std::log10(20.0));
    return gw * m + n * u;
}

int EQCurveWidget::bandAt(int x) const {
    int w = width();
    int L = 28, R = 10;
    int gw = w - L - R;
    x = std::max(L, std::min(w - R, x)) - L;
    const double m = 0.03;
    double u = gw * (1 - 2 * m);
    double n = (x - gw * m) / u;
    double freq = std::pow(10.0, std::log10(20.0) + n * (std::log10(20000.0) - std::log10(20.0)));
    double bestDist = 1e18;
    int best = 0;
    for (int i = 0; i < kNumBands; ++i) {
        double d = std::fabs(std::log10(freq) - std::log10(kFreqs[i]));
        if (d < bestDist) { bestDist = d; best = i; }
    }
    return best;
}

double EQCurveWidget::responseAt(double freq) const {
    // 与 Python 一致：全部频段 peaking biquad 幅频响应之和
    const double q = 1.414;
    const double sr = 48000.0;
    double total = 0.0;
    for (int i = 0; i < kNumBands; ++i) {
        double gain = values_[i];
        if (std::fabs(gain) < 0.1) continue;
        double A = std::pow(10.0, gain / 40.0);
        double cf = kFreqs[i];
        double w0 = 2.0 * M_PI * cf / sr;
        double alpha = std::sin(w0) / (2.0 * q);
        double cosW0 = std::cos(w0);
        double b0 = 1.0 + alpha * A, b1 = -2.0 * cosW0, b2 = 1.0 - alpha * A;
        double a0 = 1.0 + alpha / A, a1 = -2.0 * cosW0, a2 = 1.0 - alpha / A;
        b0 /= a0; b1 /= a0; b2 /= a0; a1 /= a0; a2 /= a0;
        double w = 2.0 * M_PI * freq / sr;
        double c = std::cos(w), s = std::sin(w);
        double c2 = std::cos(2 * w), s2 = std::sin(2 * w);
        double nr = b0 + b1 * c + b2 * c2;
        double ni = -(b1 * s + b2 * s2);
        double dr = 1.0 + a1 * c + a2 * c2;
        double di = -(a1 * s + a2 * s2);
        double mag = std::sqrt((nr * nr + ni * ni) / (dr * dr + di * di));
        total += 20.0 * std::log10(mag);
    }
    return total;
}

void EQCurveWidget::drawResponse(QPainter &p, int L, int T, int R, int B, int gw, int gh) {
    // 与 Python 一致：200 点真实响应计算
    QColor accent = palette().highlight().color();
    QPen pen(accent, 2.5);
    p.setPen(pen);
    QPainterPath path;
    bool first = true;
    for (int i = 0; i < 200; ++i) {
        double freq = 20.0 * std::pow(10.0, (double)i / 199 * 3.0);
        double resp = responseAt(freq);
        double x = L + freqX(freq, gw);
        double y = T + gh / 2.0 - (resp / kYRange) * (gh / 2.0);
        if (first) { path.moveTo(x, y); first = false; }
        else path.lineTo(x, y);
    }
    p.drawPath(path);
}

void EQCurveWidget::paintEvent(QPaintEvent *) {
    int w = width(), h = height();
    int L = 22, R = 10, T = 12, B = 18;
    int gw = w - L - R, gh = h - T - B;

    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    QColor gridColor = palette().mid().color();
    QColor textColor = palette().windowText().color();
    QColor numColor = palette().placeholderText().color();
    QColor accent = palette().highlight().color();

    // 背景
    p.fillRect(0, 0, w, h, palette().base().color());

    QFont smallFont = p.font();
    smallFont.setPointSize(6);
    p.setFont(smallFont);

    // 水平网格线（每 10dB）
    for (int db = -kYRange; db <= kYRange; db += 10) {
        double y = T + gh / 2.0 - ((double)db / kYRange) * (gh / 2.0);
        p.setPen(QPen(gridColor, 0.5));
        p.drawLine(L, (int)y, w - R, (int)y);
        p.setPen(QPen(numColor));
        p.drawText(QRectF(0, (int)y - 7, L - 4, 14), Qt::AlignRight | Qt::AlignVCenter,
                   QString::number(db));
    }
    // 0 dB 中线（稍粗）
    double y0 = T + gh / 2.0;
    p.setPen(QPen(gridColor, 1));
    p.drawLine(L, (int)y0, w - R, (int)y0);

    // 垂直网格线（频点），每 5 个标一个标签
    const int labelStep = 5;
    for (int i = 0; i < kNumBands; ++i) {
        double x = L + freqX(kFreqs[i], gw);
        p.setPen(QPen(gridColor, 0.5));
        p.drawLine((int)x, T, (int)x, h - B);
        if (i % labelStep == 0) {
            p.setPen(QPen(numColor));
            p.drawText(QRectF((int)x - 12, T - 12, 24, 10), Qt::AlignCenter,
                       formatFreq(kFreqs[i]));
        }
    }

    // 频响曲线（200 点）
    drawResponse(p, L, T, R, B, gw, gh);

    // 增益圆点 + 值（透明度随正负/零）
    double radius = std::max(3.0, 4.5 * w / 800.0);
    for (int i = 0; i < kNumBands; ++i) {
        double x = L + freqX(kFreqs[i], gw);
        double y = T + gh / 2.0 - (values_[i] / kYRange) * (gh / 2.0);
        QColor dot;
        if (values_[i] > 0) dot = QColor(accent.red(), accent.green(), accent.blue(), 200);
        else if (values_[i] < 0) dot = QColor(accent.red(), accent.green(), accent.blue(), 120);
        else dot = QColor(textColor.red(), textColor.green(), textColor.blue(), 60);
        p.setPen(Qt::NoPen);
        p.setBrush(dot);
        p.drawEllipse(QPointF(x, y), radius, radius);
        if (values_[i] != 0.0) {
            p.setPen(QPen(numColor));
            p.drawText(QRectF(x - 10, h - B + 1, 20, 10), Qt::AlignCenter,
                       QString("%1").arg(values_[i], 0, 'f', 0));
        }
    }
    p.end();
}

void EQCurveWidget::mousePressEvent(QMouseEvent *event) {
    if (event->button() == Qt::LeftButton) {
        dragging_ = true;
        dragBand_ = bandAt(event->pos().x());
        dragStartY_ = event->pos().y();
        dragStartGain_ = values_[dragBand_];
        event->accept();
    }
}

void EQCurveWidget::mouseMoveEvent(QMouseEvent *event) {
    if (!dragging_) return;
    int h = height();
    int T = 18, B = 18;
    int gh = h - T - B;
    double dy = event->pos().y() - dragStartY_;
    double newGain = dragStartGain_ - (dy / (gh / 2.0)) * kYRange;
    newGain = std::max(-(double)kYLimit, std::min((double)kYLimit, newGain));
    if (std::fabs(newGain - values_[dragBand_]) > 0.05) {
        values_[dragBand_] = newGain;
        update();
        emitDebounced();
    }
    event->accept();
}

void EQCurveWidget::mouseReleaseEvent(QMouseEvent *event) {
    if (dragging_) {
        dragging_ = false;
        emit gainsChanged(values_);
    }
    QWidget::mouseReleaseEvent(event);
}

void EQCurveWidget::wheelEvent(QWheelEvent *event) {
    int band = bandAt(event->position().toPoint().x());
    double delta = event->angleDelta().y() > 0 ? 1.0 : -1.0;
    double newGain = values_[band] + delta;
    newGain = std::max(-(double)kYLimit, std::min((double)kYLimit, newGain));
    if (newGain != values_[band]) {
        values_[band] = newGain;
        update();
        emitDebounced();
    }
    event->accept();
}

void EQCurveWidget::emitDebounced() {
    // 简单直接发出（后续可加定时器去抖）
    emit gainsChanged(values_);
}
