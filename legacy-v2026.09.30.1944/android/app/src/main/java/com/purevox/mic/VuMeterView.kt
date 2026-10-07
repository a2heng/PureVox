/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
# 
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
# 
# SPDX-License-Identifier: GPL-3.0-or-later
 */

package com.purevox.mic

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.util.AttributeSet
import android.view.View

class VuMeterView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : View(context, attrs, defStyleAttr) {

    companion object {
        private const val DB_MIN = -60f
        private const val DB_MAX = 0f
        private const val G1_DB = -20f  // 绿→黄分界
        private const val G2_DB = -9f   // 黄→红分界
    }

    private var db = DB_MIN
    private var peakDb = DB_MIN
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    // 主题自适应
    private val isDark: Boolean
        get() = (context.resources.configuration.uiMode and
            android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES

    // 三段背景色（亮色下浅，深色下暗，始终可见）
    private val sectionBgGreen:  Int get() = if (isDark) Color.parseColor("#1a4a2a") else Color.parseColor("#c8e8d0")
    private val sectionBgYellow: Int get() = if (isDark) Color.parseColor("#4a4a20") else Color.parseColor("#e8e8c8")
    private val sectionBgRed:    Int get() = if (isDark) Color.parseColor("#4a2020") else Color.parseColor("#e8d0d0")

    // 点亮色（不随主题变，保持高对比）
    private val greenBright  = Color.parseColor("#44bb66")
    private val yellowBright = Color.parseColor("#cccc44")
    private val redBright    = Color.parseColor("#cc5544")

    // 峰值色
    private val peakColor: Int get() = if (isDark) Color.parseColor("#c0c0c0") else Color.parseColor("#3a3530")

    fun updateLevel(db: Float, peakDb: Float) {
        this.db = db
        this.peakDb = peakDb
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        if (w < 10 || h < 4) return

        val range = DB_MAX - DB_MIN
        val g1x = ((G1_DB - DB_MIN) / range) * w
        val g2x = ((G2_DB - DB_MIN) / range) * w
        val fillW = ((db - DB_MIN) / range).coerceIn(0f, 1f) * w

        // 1. 三段背景色——始终可见（不激活时也能看到分区）
        fillRect(canvas, 0f, g1x, sectionBgGreen)
        fillRect(canvas, g1x, g2x, sectionBgYellow)
        fillRect(canvas, g2x, w, sectionBgRed)

        // 2. 点亮段——在背景上叠加更亮的颜色
        if (fillW > 0f)   fillRect(canvas, 0f, minOf(fillW, g1x), greenBright)
        if (fillW > g1x)  fillRect(canvas, g1x, minOf(fillW, g2x), yellowBright)
        if (fillW > g2x)  fillRect(canvas, g2x, fillW, redBright)

        // 3. 峰值指示
        if (peakDb > DB_MIN + 0.5f) {
            val px = ((peakDb - DB_MIN) / range).coerceIn(0f, 1f) * w
            if (px > 1f) {
                paint.color = peakColor
                canvas.drawRect(px - 1.5f, 0f, px + 1.5f, h, paint)
            }
        }
    }

    private fun fillRect(canvas: Canvas, left: Float, right: Float, color: Int) {
        if (right <= left) return
        paint.color = color
        canvas.drawRect(left, 0f, right, height.toFloat(), paint)
    }
}
