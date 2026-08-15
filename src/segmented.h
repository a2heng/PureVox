// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_SEGMENTED_H
#define PUREVOX_SEGMENTED_H

#include <QWidget>

#include <QVector>

class QHBoxLayout;
class QPushButton;
class QButtonGroup;

// 分段选择控件：一串 checkable 按钮，互斥选中，高亮当前项
class SegmentedControl : public QWidget {
    Q_OBJECT

public:
    struct Item {
        QString text;
        int value;
    };

    explicit SegmentedControl(const QVector<Item> &items, QWidget *parent = nullptr);

    int value() const;
    void setValue(int val);

signals:
    void valueChanged(int value);

protected:
    void changeEvent(QEvent *event) override;

private:
    void onClicked(int val);
    void applyStyle();

    QVector<Item> items_;
    int current_;
    QVector<QPushButton *> buttons_;
    QButtonGroup *group_;
    bool updatingStyle_ = false;
};

#endif // PUREVOX_SEGMENTED_H
