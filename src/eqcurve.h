// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_EQCURVE_H
#define PUREVOX_EQCURVE_H

#include <QWidget>

#include <QVector>

// 均衡器曲线控件：对数频响曲线 + 拖拽/滚轮调整各频段增益
class EQCurveWidget : public QWidget {
    Q_OBJECT

public:
    static constexpr int kYRange = 30;
    static constexpr int kYLimit = 20;
    static constexpr int kNumBands = 61;

    explicit EQCurveWidget(QWidget *parent = nullptr);

    QVector<double> gains() const;
    void setGains(const QVector<double> &gains);

signals:
    void gainsChanged(const QVector<double> &gains);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;

private:
    void drawResponse(QPainter &p, int L, int T, int R, int B, int gw, int gh);
    double freqX(double freq, int gw) const;
    int bandAt(int x) const;
    double responseAt(double freq) const;
    void emitDebounced();

    QVector<double> values_;
    bool dragging_ = false;
    int dragBand_ = -1;
    double dragStartY_ = 0;
    double dragStartGain_ = 0;
};

#endif // PUREVOX_EQCURVE_H
