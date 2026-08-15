// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "vubar.h"

#include <QColor>
#include <QEvent>
#include <QPainter>
#include <QPixmap>
#include <QResizeEvent>

#include <QPen>

#include <algorithm>
#include <chrono>
#include <cmath>

namespace {
const double kDbMin = -60.0;
const double kDbMax = 0.0;
const double kDbRng = kDbMax - kDbMin;
const double kPeakHold = 10.0;
const double kPeakFall = 20.0;
const QColor kGreen("#00cc44");
const QColor kYellow("#cccc00");
const QColor kRed("#cc2200");
const int kTicks[] = {-60, -54, -48, -42, -36, -30, -24, -18, -12, -6, 0};
}  // namespace

VUBar::VUBar(QWidget *parent) : QWidget(parent) {
    setFixedHeight(28);
    setMinimumWidth(100);
    t_ = std::chrono::steady_clock::now().time_since_epoch().count() / 1e9;
}

void VUBar::updateLevel(const float *samples, int n) {
    double peak = 1e-10;
    for (int i = 0; i < n; ++i) {
        peak = std::max(peak, (double)std::fabs(samples[i]));
    }
    double db = 20.0 * std::log10(peak);
    updateLevelFromDb(db);
}

void VUBar::updateLevelDb(double db) { updateLevelFromDb(db); }

void VUBar::updateLevelFromDb(double db) {
    double now = std::chrono::steady_clock::now().time_since_epoch().count() / 1e9;
    double dt = now - t_;
    t_ = now;
    db_ = db;
    if (db_ > peak_) {
        peak_ = db_;
        peakTime_ = now;
    } else if (now - peakTime_ > kPeakHold) {
        peak_ = std::max(kDbMin, peak_ - kPeakFall * dt);
    }
    if (std::fabs(db - lastPainted_) >= 0.3) {
        lastPainted_ = db;
        update();
    }
}

void VUBar::ensureBgCache(int w, int h) {
    if (bgCache_ && cacheSize_ == QSize(w, h)) return;
    delete bgCache_;
    bgCache_ = new QPixmap(w, h);
    bgCache_->fill(Qt::transparent);
    cacheSize_ = QSize(w, h);

    QColor vuBg = palette().base().color();
    QPainter p(bgCache_);
    p.setRenderHint(QPainter::Antialiasing);
    p.fillRect(0, 0, w, h, vuBg);

    int barLeft = 6, barRight = w - 4;
    int barTop = 2, barH = 12;
    int barBottom = barTop + barH;
    double barW = barRight - barLeft;

    double g1r = (-20.0 - kDbMin) / kDbRng;
    double g2r = (-9.0 - kDbMin) / kDbRng;
    int x1 = barLeft + (int)(g1r * barW);
    int x2 = barLeft + (int)(g2r * barW);

    p.fillRect(barLeft, barTop, x1 - barLeft, barH, QColor(0, 40, 24));
    p.fillRect(x1, barTop, x2 - x1, barH, QColor(60, 60, 0));
    p.fillRect(x2, barTop, barRight - x2, barH, QColor(70, 20, 0));
    p.end();
}

void VUBar::paintEvent(QPaintEvent *) {
    int w = width(), h = height();
    if (w < 40 || h < 16) return;
    ensureBgCache(w, h);

    int barLeft = 6, barRight = w - 4;
    int barTop = 2, barH = 12;
    double barW = barRight - barLeft;

    double g1r = (-20.0 - kDbMin) / kDbRng;
    double g2r = (-9.0 - kDbMin) / kDbRng;
    int x1 = barLeft + (int)(g1r * barW);
    int x2 = barLeft + (int)(g2r * barW);

    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.drawPixmap(0, 0, *bgCache_);

    double rNow = std::max(0.0, std::min(1.0, (db_ - kDbMin) / kDbRng));
    int fillX = barLeft + (int)(rNow * barW);
    if (fillX > barLeft) {
        p.fillRect(barLeft, barTop, std::min(fillX, x1) - barLeft, barH, kGreen);
    }
    if (fillX > x1) {
        p.fillRect(x1, barTop, std::min(fillX, x2) - x1, barH, kYellow);
    }
    if (fillX > x2) {
        p.fillRect(x2, barTop, fillX - x2, barH, kRed);
    }

    if (peak_ > kDbMin + 0.5) {
        double rPeak = std::max(0.0, std::min(1.0, (peak_ - kDbMin) / kDbRng));
        int px = barLeft + (int)(rPeak * barW);
        if (px > barLeft + 1) {
            QColor pk = rPeak < g1r ? kGreen : (rPeak < g2r ? kYellow : kRed);
            p.fillRect(QRectF(px - 1, barTop, 3, barH), pk);
        }
    }

    QFont tickFont = p.font();
    tickFont.setPointSize(7);
    p.setFont(tickFont);
    QColor vuText = palette().placeholderText().color();
    for (int tick : kTicks) {
        double r = std::max(0.0, std::min(1.0, (tick - kDbMin) / kDbRng));
        int x = barLeft + (int)(r * barW);
        p.setPen(QPen(vuText, 0.5));
        p.drawLine(x, barTop + barH, x, barTop + barH + 3);
        p.setPen(vuText);
        p.drawText(QRectF(x - 20, barTop + barH, 40, 16), Qt::AlignHCenter | Qt::AlignVCenter,
                   QString::number(tick));
    }
    p.end();
}

void VUBar::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    delete bgCache_;
    bgCache_ = nullptr;
}

void VUBar::changeEvent(QEvent *event) {
    if (event->type() == QEvent::PaletteChange) {
        delete bgCache_;
        bgCache_ = nullptr;
        update();
    }
    QWidget::changeEvent(event);
}
