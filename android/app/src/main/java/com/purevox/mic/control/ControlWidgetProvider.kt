/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
 * PureVox is licensed under the GNU General Public License v3.0 or
 * later (GPL-3.0-or-later).  See LICENSE for details.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * The built-in AI models are NOT covered by the GPL; they are the
 * property of a2heng and may only be used with PureVox under
 * authorization.  See MODEL-LICENSE.md for details.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

package com.purevox.mic.control

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import com.purevox.mic.R

/**
 * 桌面小组件：4×1 控制条，按钮与通知栏完全同一套（[ControlViews]）。
 *
 * 状态变化由 [Control.publish] 主动推（`updateAll`）；`onUpdate` 只处理刚加到桌面 /
 * 系统请求刷新。没有周期刷新 —— 小组件不是自己轮询的第二个状态源。
 */
class ControlWidgetProvider : AppWidgetProvider() {

    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray,
    ) {
        val views = ControlViews.build(context, R.layout.widget_controls, Control.open(context))
        appWidgetManager.updateAppWidget(appWidgetIds, views)
    }

    companion object {
        /** 把当前状态推到所有已添加的小组件（没有添加时零开销）。 */
        fun updateAll(context: Context) {
            val app = context.applicationContext
            val manager = AppWidgetManager.getInstance(app)
            val ids = manager.getAppWidgetIds(
                ComponentName(app, ControlWidgetProvider::class.java),
            )
            if (ids.isEmpty()) return
            val views = ControlViews.build(app, R.layout.widget_controls, Control.open(app))
            manager.updateAppWidget(ids, views)
        }
    }
}
