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

import android.os.Bundle
import android.view.ViewGroup
import com.journeyapps.barcodescanner.CaptureActivity
import com.journeyapps.barcodescanner.DecoratedBarcodeView

/**
 * 扫码界面：**竖屏 + 正方形四角取景框**（用户 2026-10-27 指定）。
 *
 * 为什么自己写一个：库自带的 `CaptureActivity` 是横屏、取景框只有一条细边，观感很糙。
 * 这里继承它（结果回传 / 权限 / 生命周期都照旧由库处理），只替换 `initializeContent()`
 * 返回的取景框：
 * - 竖屏由 Manifest 的 `screenOrientation="portrait"` 钉死；
 * - 去掉库自带的 `ViewfinderView`，换成 [PvViewfinderView]（压暗 + 四角 + 扫描线）；
 * - 底部提示文字（`ScanOptions.setPrompt`）保留在最上层。
 */
class PvScanActivity : CaptureActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // 三板斧：竖屏、黑底、状态栏透明（全屏由主题负责）
        requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
    }

    override fun initializeContent(): DecoratedBarcodeView {
        val view = super.initializeContent()
        // 库自带取景框：整层移除，换成我们的四角框
        val old = view.getViewFinder()
        (old.parent as? ViewGroup)?.removeView(old)
        val overlay = PvViewfinderView(this)
        view.addView(
            overlay,
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        // 提示文字必须在取景框之上（后加的子项会盖住它）
        view.getStatusView().bringToFront()
        return view
    }
}
