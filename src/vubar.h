// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_VUBAR_H
#define PUREVOX_VUBAR_H

#include <QWidget>

class QPixmap;

// 横向电平条：暗色背景分区 + 亮色进度条 + 峰值缓慢回落 + dB 刻度
class VUBar : public QWidget {
    Q_OBJECT

public:
    explicit VUBar(QWidget *parent = nullptr);

    void updateLevelDb(double db);  // 直接从 dBFS 更新
    void updateLevel(const float *samples, int n);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    void updateLevelFromDb(double db);
    void ensureBgCache(int w, int h);

    double db_ = -60.0;
    double peak_ = -60.0;
    double peakTime_ = 0.0;
    double t_ = 0.0;
    double lastPainted_ = -70.0;
    QPixmap *bgCache_ = nullptr;
    QSize cacheSize_;
};

#endif // PUREVOX_VUBAR_H
