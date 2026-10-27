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

package com.purevox.mic

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Shader
import android.util.AttributeSet
import android.view.View
import android.view.animation.AccelerateDecelerateInterpolator
import kotlin.math.min

/**
 * 扫码取景框：**中间一个正方形 + 四个角**（用户 2026-10-27 指定，替换库自带的细边框）。
 *
 * 纯绘制层：不做任何识别，扫描范围由 `BarcodeView` 决定（它扫整帧），这里只是引导。
 * 外观：外部压暗、四角品牌蓝、正方形内一条缓慢往返的扫描线；竖屏下正方形居中偏上，
 * 给底部的提示文字留位置。
 */
class PvViewfinderView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyle: Int = 0,
) : View(context, attrs, defStyle) {

    private val density = resources.displayMetrics.density
    private fun dp(v: Float) = v * density

    private val mask = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0xB3000000.toInt() }
    private val corner = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.parseColor("#4F8CFF")
        style = Paint.Style.STROKE
        strokeWidth = dp(3.2f)
        strokeCap = Paint.Cap.ROUND
    }
    private val line = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.FILL
        strokeWidth = dp(2f)
    }

    private val box = RectF()
    /** 0..1 往返：扫描线的位置 */
    private var phase = 0f

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        // 只在这块界面可见时重绘（离开即停，别在后台烧 CPU）
        postInvalidateOnAnimation()
    }

    private fun layoutBox(w: Int, h: Int) {
        val side = min(w, h) * 0.64f
        val left = (w - side) / 2f
        val top = h * 0.5f - side / 2f - h * 0.04f
        box.set(left, top, left + side, top + side)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val w = width.toFloat()
        val h = height.toFloat()
        if (w <= 0f || h <= 0f) return
        layoutBox(width, height)

        // 1) 正方形外压暗（分四块画，省得用 saveLayer）
        canvas.drawRect(0f, 0f, w, box.top, mask)
        canvas.drawRect(0f, box.bottom, w, h, mask)
        canvas.drawRect(0f, box.top, box.left, box.bottom, mask)
        canvas.drawRect(box.right, box.top, w, box.bottom, mask)

        // 2) 四个角（L 形线段，长度约正方形边长的 16%）
        val len = box.width() * 0.16f
        val l = box.left
        val t = box.top
        val r = box.right
        val b = box.bottom
        canvas.drawLine(l, t + len, l, t, corner)
        canvas.drawLine(l, t, l + len, t, corner)
        canvas.drawLine(r - len, t, r, t, corner)
        canvas.drawLine(r, t, r, t + len, corner)
        canvas.drawLine(l, b - len, l, b, corner)
        canvas.drawLine(l, b, l + len, b, corner)
        canvas.drawLine(r - len, b, r, b, corner)
        canvas.drawLine(r, b, r, b - len, corner)

        // 3) 扫描线：在正方形里上下缓慢往返（扫描界面才有的「活着」的感觉）
        val y = box.top + box.height() * phase
        val gradient = LinearGradient(
            box.left, y, box.right, y,
            intArrayOf(0x004F8CFF, 0xCC4F8CFF.toInt(), 0x004F8CFF),
            floatArrayOf(0f, 0.5f, 1f),
            Shader.TileMode.CLAMP,
        )
        line.shader = gradient
        canvas.drawRect(box.left + dp(2f), y - dp(0.8f), box.right - dp(2f), y + dp(0.8f), line)

        val now = System.currentTimeMillis() % 2400L
        val raw = now / 2400f
        // 三角波：0→1→0
        phase = if (raw < 0.5f) {
            AccelerateDecelerateInterpolator().getInterpolation(raw * 2f)
        } else {
            AccelerateDecelerateInterpolator().getInterpolation((1f - raw) * 2f)
        }
        postInvalidateOnAnimation()
    }
}
