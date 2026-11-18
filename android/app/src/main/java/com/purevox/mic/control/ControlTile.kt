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

import android.content.Context
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import com.purevox.mic.R

/**
 * 「开关型」磁贴的公共骨架：点按 = 发一个控制动作，读 [Control] 的状态画自己。
 *
 * 三个开关磁贴（麦克风 / 扬声器 / 重新连接）都是同一条路径；状态由界面发布，
 * 磁贴自己不维护第二份。
 */
abstract class ControlTile : TileService() {

    protected abstract val action: String

    /** 用当前状态重画磁贴（`onStartListening` 与点按后都走这里）。 */
    protected abstract fun render(tile: Tile)

    override fun onStartListening() {
        super.onStartListening()
        render(qsTile)
    }

    override fun onClick() {
        super.onClick()
        Control.send(this, action)
        // 界面处理后会 publish 新状态；这里先重画一次给即时反馈（离线时也不会卡在旧状态）
        render(qsTile)
    }

    /** 副标题：离线时说明要先打开应用，在线时直接显示连接状态。 */
    protected fun connSubtitle(ctx: Context): String = when (Control.conn) {
        Control.CONN_ONLINE -> ctx.getString(R.string.notif_conn_on)
        Control.CONN_CONNECTING -> ctx.getString(R.string.tile_connecting)
        else -> ctx.getString(R.string.tile_offline)
    }
}
