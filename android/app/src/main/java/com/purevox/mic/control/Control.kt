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
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.service.quicksettings.TileService

/**
 * 三个控制面（通知 / 快捷设置磁贴 / 桌面小组件）共用的一套动作与状态。
 *
 * **只有一个状态源**：界面在 `syncSessionService` 里把连接 / 麦克风 / 扬声器发布到这里，
 * 通知图标、磁贴、小组件都读这一份，不会出现「通知说开着、磁贴说关着」。
 *
 * 点击也只发四个动作（麦克风 / 扬声器 / 重新连接 / 打开界面），由 [ControlReceiver]
 * 转给 [com.purevox.mic.MainActivity] —— 界面仍是唯一编排者，服务不认识协议。
 */
object Control {

    const val ACTION_MIC = "com.purevox.mic.control.MIC"
    const val ACTION_SPK = "com.purevox.mic.control.SPK"
    const val ACTION_RECONNECT = "com.purevox.mic.control.RECONNECT"

    /** 连接状态（通知状态图标 / 磁贴副标题 / 小组件状态图标共用）。 */
    const val CONN_OFFLINE = 0
    const val CONN_CONNECTING = 1
    const val CONN_ONLINE = 2

    @Volatile
    var conn = CONN_OFFLINE
        private set

    @Volatile
    var mic = false
        private set

    @Volatile
    var spk = false
        private set

    /** 四个磁贴：状态一变化就 requestListeningState，让系统重新回调 onStartListening。 */
    private val tiles = listOf(
        StatusTile::class.java,
        MicTile::class.java,
        SpkTile::class.java,
        ReconnectTile::class.java,
    )

    /** 按钮 → [ControlReceiver] 的广播（通知 / 小组件共用的 PendingIntent）。 */
    fun broadcast(ctx: Context, action: String): PendingIntent {
        val intent = Intent(ctx, ControlReceiver::class.java).setAction(action)
        return PendingIntent.getBroadcast(
            ctx,
            action.hashCode(),
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    /** 磁贴点击走这里（同一个接收端，和通知按钮完全等价）。 */
    fun send(ctx: Context, action: String) {
        ctx.sendBroadcast(Intent(ctx, ControlReceiver::class.java).setAction(action))
    }

    /** 打开界面（状态按钮 / 状态磁贴）。 */
    fun open(ctx: Context): PendingIntent? {
        val launch = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName) ?: return null
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        return PendingIntent.getActivity(ctx, 0, launch, PendingIntent.FLAG_IMMUTABLE)
    }

    /**
     * 发布当前状态（只在真正变化时刷新磁贴 / 小组件）。
     *
     * 由界面调用，与通知图标同源；不在服务里调用 —— 服务只负责画通知。
     */
    fun publish(ctx: Context, conn: Int, mic: Boolean, spk: Boolean) {
        if (this.conn == conn && this.mic == mic && this.spk == spk) return
        this.conn = conn
        this.mic = mic
        this.spk = spk
        for (tile in tiles) {
            // 应用刚启动 / 磁贴没加时不报错；requestListeningState 本身是尽力而为
            runCatching {
                TileService.requestListeningState(ctx, ComponentName(ctx, tile))
            }
        }
        ControlWidgetProvider.updateAll(ctx)
    }
}
