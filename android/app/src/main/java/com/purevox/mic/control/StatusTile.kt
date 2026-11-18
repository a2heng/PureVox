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

import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import com.purevox.mic.R

/**
 * 快捷设置磁贴：连接状态（只显示 + 点按打开界面）。
 *
 * 不在磁贴里做断开/连接：连接需要地址 / 权限 / 发现编排，界面才是唯一编排者。
 */
class StatusTile : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        qsTile?.apply {
            state = if (Control.conn == Control.CONN_ONLINE) {
                Tile.STATE_ACTIVE
            } else {
                Tile.STATE_INACTIVE
            }
            label = getString(R.string.tile_status)
            icon = android.graphics.drawable.Icon.createWithResource(
                this@StatusTile,
                when (Control.conn) {
                    Control.CONN_ONLINE -> R.drawable.ic_notif_link
                    Control.CONN_CONNECTING -> R.drawable.ic_notif_search
                    else -> R.drawable.ic_notif_link_off
                },
            )
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                subtitle = when (Control.conn) {
                    Control.CONN_ONLINE -> getString(R.string.notif_conn_on)
                    Control.CONN_CONNECTING -> getString(R.string.tile_connecting)
                    else -> getString(R.string.tile_offline)
                }
            }
            updateTile()
        }
    }

    @Suppress("DEPRECATION")
    override fun onClick() {
        super.onClick()
        Control.open(this)?.let { startActivityAndCollapse(it) }
    }
}
