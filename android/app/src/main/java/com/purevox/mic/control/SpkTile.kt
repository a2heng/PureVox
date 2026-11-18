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
import com.purevox.mic.R

/** 快捷设置磁贴：扬声器开关（与界面 / 通知栏的扬声器按钮等价）。 */
class SpkTile : ControlTile() {

    override val action: String = Control.ACTION_SPK

    override fun render(tile: Tile) {
        tile.state = if (Control.spk) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = getString(R.string.tile_spk)
        tile.icon = android.graphics.drawable.Icon.createWithResource(this, R.drawable.ic_notif_spk)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            tile.subtitle = when (Control.conn) {
                Control.CONN_ONLINE -> getString(if (Control.spk) R.string.tile_on else R.string.tile_off)
                Control.CONN_CONNECTING -> getString(R.string.tile_connecting)
                else -> getString(R.string.tile_offline)
            }
        }
        tile.updateTile()
    }
}
