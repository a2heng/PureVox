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

import android.app.PendingIntent
import android.content.Context
import android.widget.RemoteViews
import com.purevox.mic.R

/**
 * 通知控制条与桌面小组件共用的 RemoteViews 构建器。
 *
 * **图标与状态只有这一份实现**：连接 / 麦克风 / 扬声器的图标选择、点击动作、
 * 无障碍描述都在这里；通知和小组件只是换一个布局 id。
 */
object ControlViews {

    fun build(
        ctx: Context,
        layout: Int,
        contentPi: PendingIntent?,
        conn: Int = Control.conn,
        mic: Boolean = Control.mic,
        spk: Boolean = Control.spk,
    ): RemoteViews {
        val views = RemoteViews(ctx.packageName, layout)

        views.setImageViewResource(
            R.id.notifConn,
            when (conn) {
                Control.CONN_ONLINE -> R.drawable.ic_notif_link
                Control.CONN_CONNECTING -> R.drawable.ic_notif_search
                else -> R.drawable.ic_notif_link_off
            },
        )
        views.setContentDescription(
            R.id.notifConn,
            ctx.getString(
                when (conn) {
                    Control.CONN_ONLINE -> R.string.notif_conn_on
                    Control.CONN_CONNECTING -> R.string.notif_conn_wait
                    else -> R.string.notif_conn_off
                },
            ),
        )
        if (contentPi != null) views.setOnClickPendingIntent(R.id.notifConn, contentPi)

        views.setImageViewResource(
            R.id.notifMic,
            if (mic) R.drawable.ic_notif_mic else R.drawable.ic_notif_mic_off,
        )
        views.setContentDescription(
            R.id.notifMic,
            ctx.getString(
                if (mic) R.string.notif_action_mic_off else R.string.notif_action_mic_on,
            ),
        )
        views.setOnClickPendingIntent(
            R.id.notifMic,
            Control.broadcast(ctx, Control.ACTION_MIC),
        )

        views.setImageViewResource(
            R.id.notifSpk,
            if (spk) R.drawable.ic_notif_spk else R.drawable.ic_notif_spk_off,
        )
        views.setContentDescription(
            R.id.notifSpk,
            ctx.getString(
                if (spk) R.string.notif_action_spk_off else R.string.notif_action_spk_on,
            ),
        )
        views.setOnClickPendingIntent(
            R.id.notifSpk,
            Control.broadcast(ctx, Control.ACTION_SPK),
        )

        views.setImageViewResource(R.id.notifReconnect, R.drawable.ic_notif_refresh)
        views.setContentDescription(
            R.id.notifReconnect,
            ctx.getString(R.string.notif_action_reconnect),
        )
        views.setOnClickPendingIntent(
            R.id.notifReconnect,
            Control.broadcast(ctx, Control.ACTION_RECONNECT),
        )

        return views
    }
}
