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

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.widget.Toast
import com.purevox.mic.MainActivity
import com.purevox.mic.R

/**
 * 通知按钮 / 磁贴 / 小组件按钮的唯一入口。
 *
 * 三者都只发 [Control.ACTION_MIC] / [Control.ACTION_SPK] / [Control.ACTION_RECONNECT]；
 * 这里把它们转给界面（[MainActivity.handleControl]）。界面不在（进程被系统回收、用户划掉了
 * 应用）时**不能静默**：弹一条「先打开 PureVox」，否则用户会以为按钮坏了。
 *
 * 故意不在后台拉起前台服务 / 采集：没有界面的会话状态无法确认，Android 12+ 也会拒绝
 * 后台起前台服务。需要先连接时本来就得打开界面。
 */
class ControlReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        val action = intent?.action ?: return
        if (action != Control.ACTION_MIC &&
            action != Control.ACTION_SPK &&
            action != Control.ACTION_RECONNECT
        ) {
            return
        }
        val a = MainActivity.current
        if (a == null || a.isDestroyed || a.isFinishing) {
            Toast.makeText(context, R.string.control_open_app, Toast.LENGTH_SHORT).show()
            return
        }
        a.handleControl(action)
    }
}
