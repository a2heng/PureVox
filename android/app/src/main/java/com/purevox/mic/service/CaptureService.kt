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

package com.purevox.mic.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import com.purevox.mic.MainActivity
import com.purevox.mic.R

/**
 * 麦克风推流的前台服务。
 *
 * 职责很窄，就三件事（Android 9+ 没有前台服务就拿不到后台麦克风）：
 * 1. 挂一个常驻通知（用户可见的「正在用麦克风」提示）；
 * 2. 持一个 PARTIAL_WAKE_LOCK，保证息屏时音频线程不被挂起；
 * 3. 声明 `foregroundServiceType="microphone"`（Android 14+ 强制要求）。
 *
 * **音频管线不在这里**（在 `MainActivity` 持有的 `MicUplink` 里）。这是刻意的：
 * 界面是唯一的编排者，服务只提供「合法地继续用麦克风」的环境。
 */
class CaptureService : Service() {

    companion object {
        private const val TAG = "PureVoxSvc"

        const val ACTION_START = "com.purevox.mic.action.START"
        const val ACTION_UPDATE = "com.purevox.mic.action.UPDATE"
        const val ACTION_STOP = "com.purevox.mic.action.STOP"

        /** 通知正文（由界面刷新，用来显示已推包数 / 采集格式）。 */
        const val EXTRA_TEXT = "text"

        const val CHANNEL_ID = "purevox_mic_stream"
        const val NOTIFICATION_ID = 1001

        private const val WAKE_LOCK_TAG = "PureVoxMic:AudioWakeLock"

        fun start(ctx: Context, text: String) {
            val intent = Intent(ctx, CaptureService::class.java).apply {
                action = ACTION_START
                putExtra(EXTRA_TEXT, text)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                ctx.startForegroundService(intent)
            } else {
                ctx.startService(intent)
            }
        }

        fun update(ctx: Context, text: String) {
            val intent = Intent(ctx, CaptureService::class.java).apply {
                action = ACTION_UPDATE
                putExtra(EXTRA_TEXT, text)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                ctx.startForegroundService(intent)
            } else {
                ctx.startService(intent)
            }
        }

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, CaptureService::class.java))
        }
    }

    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START, ACTION_UPDATE -> {
                val text = intent.getStringExtra(EXTRA_TEXT) ?: getString(R.string.notif_default)
                acquireWakeLock()
                startForegroundCompat(text)
            }
            ACTION_STOP -> {
                releaseWakeLock()
                stopForegroundCompat()
                stopSelf()
            }
        }
        // 不复活：麦克风不能悄悄重启，用户得自己再开一次
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        releaseWakeLock()
        stopForegroundCompat()
        super.onDestroy()
    }

    // ---------------------------------------------------------------- 内部

    @Suppress("DEPRECATION")
    private fun startForegroundCompat(text: String) {
        val notification = buildNotification(text)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
        } catch (e: Throwable) {
            // Android 12+ 从后台起前台服务会抛；界面会把它显示出来，不静默
            Log.e(TAG, "startForeground 失败", e)
            stopSelf()
        }
    }

    @Suppress("DEPRECATION")
    private fun stopForegroundCompat() {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                stopForeground(Service.STOP_FOREGROUND_REMOVE)
            } else {
                stopForeground(true)
            }
        } catch (e: Throwable) {
            Log.w(TAG, "stopForeground 异常", e)
        }
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notif_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.notif_channel_desc)
            setShowBadge(false)
        }
        getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val pi = launch?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE)
        }
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_stat_mic)
            .setContentIntent(pi)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
    }

    private fun acquireWakeLock() {
        if (wakeLock != null) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG).apply {
            setReferenceCounted(false)
            // 长推流场景不给上限，但设一个远大于任何会话的兜底，避免永久持有
            acquire(4 * 60 * 60 * 1000L)
        }
    }

    private fun releaseWakeLock() {
        val l = wakeLock ?: return
        wakeLock = null
        try {
            if (l.isHeld) l.release()
        } catch (e: Throwable) {
            Log.w(TAG, "释放 WakeLock 异常", e)
        }
    }
}