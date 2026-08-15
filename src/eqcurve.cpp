// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "eqcurve.h"

#include <QMouseEvent>
#include <QPainter>
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
    double r = (std::log10(freq) - kLogMin) / (kLogMax - kLogMin);
    return r * gw;
}

int EQCurveWidget::bandAt(int x) const {
    int w = width();
    int L = 38, R = 14;
    int gw = w - L - R;
    double logDist = 1e9;
    int best = 0;
    for (int i = 0; i < kNumBands; ++i) {
        double d = std::fabs(freqX(kFreqs[i], gw) + L - x);
        if (d < logDist) {
            logDist = d;
            best = i;
        }
    }
    return best;
}

double EQCurveWidget::responseAt(double freq) const {
    double acc = 0;
    double wSum = 0;
    for (int i = 0; i < kNumBands; ++i) {
        double a = std::fabs(values_[i]);
        if (a < 0.1) continue;
        double diff = std::fabs(std::log10(freq) - std::log10(kFreqs[i]));
        double w = a / (1.0 + diff * diff * 20.0);
        acc += values_[i] * w;
        wSum += w;
    }
    return wSum > 1e-9 ? acc / wSum : 0.0;
}

void EQCurveWidget::drawResponse(QPainter &p, int L, int T, int R, int B, int gw, int gh) {
    QColor accent = palette().highlight().color();
    QPen pen(accent, 2);
    p.setPen(pen);
    QPointF prev;
    for (int px = 0; px <= gw; px += 2) {
        double freq = std::pow(10, kLogMin + (double)px / gw * (kLogMax - kLogMin));
        double resp = responseAt(freq);
        double y = T + gh / 2.0 - (resp / kYRange) * (gh / 2.0);
        QPointF pt(L + px, y);
        if (px > 0) p.drawLine(prev, pt);
        prev = pt;
    }
}

void EQCurveWidget::paintEvent(QPaintEvent *) {
    int w = width(), h = height();
    int L = 38, R = 14, T = 18, B = 18;
    int gw = w - L - R, gh = h - T - B;

    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    QColor gridColor = palette().mid().color();
    QColor textColor = palette().windowText().color();

    // 水平网格线（dB）
    for (int db = -kYRange; db <= kYRange; db += 10) {
        double y = T + gh / 2.0 - ((double)db / kYRange) * (gh / 2.0);
        p.setPen(QPen(gridColor, 1, Qt::DotLine));
        p.drawLine(L, (int)y, w - R, (int)y);
        p.setPen(textColor);
        p.drawText(QRectF(0, (int)y - 7, L - 4, 14), Qt::AlignRight | Qt::AlignVCenter,
                   QString::number(db));
    }
    // 0 dB 参考线
    double y0 = T + gh / 2.0;
    p.setPen(QPen(gridColor, 1, Qt::SolidLine));
    p.drawLine(L, (int)y0, w - R, (int)y0);

    // 垂直网格线（频点）
    for (int i = 0; i < kNumBands; ++i) {
        double x = L + freqX(kFreqs[i], gw);
        p.setPen(QPen(gridColor, 1, Qt::DotLine));
        p.drawLine((int)x, T, (int)x, h - B);
        p.setPen(textColor);
        QFont f = p.font();
        f.setPointSize(7);
        p.setFont(f);
        p.drawText(QRectF((int)x - 12, T - 12, 24, 10), Qt::AlignCenter,
                   formatFreq(kFreqs[i]));
    }

    // 频响曲线
    drawResponse(p, L, T, R, B, gw, gh);

    // 增益点 + 值
    for (int i = 0; i < kNumBands; ++i) {
        double x = L + freqX(kFreqs[i], gw);
        double y = T + gh / 2.0 - (values_[i] / kYRange) * (gh / 2.0);
        if (std::fabs(values_[i]) > 0.1) {
            QColor c = values_[i] > 0 ? QColor("#00cc44") : QColor("#cc2200");
            p.setPen(QPen(c, 2));
            p.setBrush(c);
            p.drawEllipse(QPointF(x, y), 3, 3);
            p.drawText(QRectF((int)x - 10, (int)y - 16, 20, 12), Qt::AlignCenter,
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
