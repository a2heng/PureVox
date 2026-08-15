// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "segmented.h"

#include <QButtonGroup>
#include <QEvent>
#include <QHBoxLayout>
#include <QPushButton>

SegmentedControl::SegmentedControl(const QVector<Item> &items, QWidget *parent)
    : QWidget(parent), items_(items), current_(items.isEmpty() ? 0 : items[0].value) {
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    group_ = new QButtonGroup(this);
    group_->setExclusive(true);

    for (const Item &item : items_) {
        auto *btn = new QPushButton(item.text, this);
        btn->setCheckable(true);
        btn->setCursor(Qt::PointingHandCursor);
        btn->setFixedHeight(24);
        btn->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
        group_->addButton(btn);
        buttons_.append(btn);
        layout->addWidget(btn);
        connect(btn, &QPushButton::clicked, this,
                [this, val = item.value]() { onClicked(val); });
    }
    if (!buttons_.isEmpty()) buttons_.first()->setChecked(true);
    applyStyle();
}

int SegmentedControl::value() const { return current_; }

void SegmentedControl::setValue(int val) {
    if (val != current_) {
        current_ = val;
        for (int i = 0; i < buttons_.size(); ++i) {
            if (items_[i].value == val) {
                buttons_[i]->setChecked(true);
                break;
            }
        }
    }
}

void SegmentedControl::onClicked(int val) {
    if (val != current_) {
        current_ = val;
        applyStyle();
        emit valueChanged(val);
    }
}

void SegmentedControl::changeEvent(QEvent *event) {
    if (event->type() == QEvent::PaletteChange) applyStyle();
    QWidget::changeEvent(event);
}

void SegmentedControl::applyStyle() {
    if (updatingStyle_) return;
    updatingStyle_ = true;
    QColor accent = palette().highlight().color();
    QColor sep = palette().mid().color();
    QColor fg = palette().windowText().color();

    QString sepStr = sep.name();
    QString css = QString(
        "QWidget { background: transparent; border: 1px solid %1; }"
        "QWidget QPushButton { background: transparent; border: none;"
        "  border-right: 1px solid %2; padding: 2px 4px; color: %3; font-size: 10pt; }"
        "QWidget QPushButton:last-child { border-right: none; }"
        "QWidget QPushButton:checked { background: %1; color: #ffffff; font-weight: bold; }"
        "QWidget QPushButton:hover:!checked { background: rgba(128,128,128,0.15); }")
        .arg(accent.name())
        .arg(sepStr)
        .arg(fg.name());
    setStyleSheet(css);
    updatingStyle_ = false;
}
