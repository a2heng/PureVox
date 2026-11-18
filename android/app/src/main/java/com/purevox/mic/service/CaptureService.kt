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

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import android.widget.RemoteViews
import androidx.core.app.NotificationCompat
import com.purevox.mic.MainActivity
import com.purevox.mic.R
import com.purevox.mic.control.Control
import com.purevox.mic.control.ControlViews

/**
 * 会话前台服务（**保活**）。
 *
 * 职责很窄：挂一个常驻通知 + 持一把 PARTIAL_WAKE_LOCK，让系统别把本进程冻结 ——
 * 音频与网络线程都在界面进程里，没有前台服务，Android 9+ 后台拿不到麦克风、Android 12+
 * 还会被系统直接掐掉。
 *
 * **服务范围 = 整个会话**（不是只有麦克风）：只要「已连接」「正在推流」「连接中」或**界面在前台**
 * 就常驻通知。2026-10-18 用户实测结论：**撤掉通知之后系统很快就把进程掐了** ——
 * 界面开着就得把通知挂着，别等「连上了再挂」（后台里起前台服务会被系统拒绝，
 * 那一次的失败要等到界面在前台时补挂，见 [MainActivity.onResume]）。
 * 通知里的按钮（连接状态 / 麦克风 / 扬声器 / 重新连接）由界面（[MainActivity]）接收并处理 ——
 * 服务本身不认识协议，**界面是唯一的编排者**。
 *
 * 前台服务类型：麦克风在用时 `microphone|dataSync`，只是保活连接时 `dataSync`。
 * 哪个在用就报哪个（Android 14 起按实际用途校验，不能超报）。
 *
 * **通知里没有文字，只有一行四个图标**（`res/layout/notif_controls.xml`）。
 * 标准 `addAction` 在 Android 7+ 只画文字、忽略图标，四个中文标签在窄屏会被压缩；
 * 自定义 RemoteViews 是唯一能保证「等宽、固定一行、状态即图标」的做法。
 */
class CaptureService : Service() {

    companion object {
        private const val TAG = "PureVoxSvc"

        const val ACTION_START = "com.purevox.mic.action.START"
        const val ACTION_UPDATE = "com.purevox.mic.action.UPDATE"
        const val ACTION_STOP = "com.purevox.mic.action.STOP"

        /** 通知被用户划掉后由 `deleteIntent` 拉起：用最近一次状态立刻重挂。 */
        const val ACTION_REPOST = "com.purevox.mic.action.REPOST"

        /** 本条通知是否包含 `microphone` 前台服务类型（也决定按钮图标「麦克风开/关」）。 */
        const val EXTRA_MIC = "mic"

        /** 扬声器是否在放（决定按钮图标「扬声器开/关」；不影响前台服务类型）。 */
        const val EXTRA_SPK = "spk"

        /**
         * 当前连接状态（决定通知里状态按钮的图标，取值见 [Control.CONN_OFFLINE] 等）。
         */
        const val EXTRA_CONN = "conn"

        /** 当前是否真的有音频在跑（mic 或 speaker）；只有这时才持 WakeLock。
         *  仅仅「连着/界面在前台」不需要 CPU 常亮 —— 通知挂着即可。 */
        const val EXTRA_AUDIO = "audio"

        const val CHANNEL_ID = "purevox_mic_stream"
        const val NOTIFICATION_ID = 1001

        /**
         * 最近一次「挂通知」失败的原因（null = 没失败过）。
         *
         * **必须能被界面读到**：挂不上通知 = 系统随时可以掐掉本进程（用户的实测结论，
         * 2026-10-18：撤掉通知之后很快就掉线）。失败本身不炸界面（保活是尽力而为），
         * 但绝不能**静默** —— 界面「后台保活」卡片与诊断里会写出来。
         */
        @Volatile
        var lastError: String? = null
            private set

        /** 本进程当前是否真的挂着前台通知（由服务在成功/停止时维护）。 */
        @Volatile
        var foreground = false

        /** 当前是否持着高性能 WifiLock（**省电模式是音频流的最大杀手**，必须可见）。 */
        @Volatile
        var wifiLockHeld = false
            private set

        internal fun noteError(msg: String) {
            lastError = msg
            foreground = false
        }

        internal fun noteForeground() {
            lastError = null
            foreground = true
        }

        private const val WAKE_LOCK_TAG = "PureVoxMic:AudioWakeLock"

        /** Wi-Fi 高性能锁：关掉省电模式（它会攒包，实测 RTT 5→55 ms、抖动 25 ms）。 */
        private const val WIFI_LOCK_TAG = "PureVoxMic:WifiHighPerf"

        fun start(ctx: Context, mic: Boolean, audio: Boolean, spk: Boolean = false, conn: Int) =
            deliver(ctx, ACTION_START, mic, audio, spk, conn)

        fun update(ctx: Context, mic: Boolean, audio: Boolean, spk: Boolean = false, conn: Int) =
            deliver(ctx, ACTION_UPDATE, mic, audio, spk, conn)

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, CaptureService::class.java))
        }

        private fun deliver(
            ctx: Context,
            action: String,
            mic: Boolean,
            audio: Boolean,
            spk: Boolean,
            conn: Int,
        ) {
            val intent = Intent(ctx, CaptureService::class.java).apply {
                this.action = action
                putExtra(EXTRA_MIC, mic)
                putExtra(EXTRA_AUDIO, audio)
                putExtra(EXTRA_SPK, spk)
                putExtra(EXTRA_CONN, conn)
            }
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    ctx.startForegroundService(intent)
                } else {
                    ctx.startService(intent)
                }
            } catch (e: Throwable) {
                // Android 12+ 从后台起前台服务会抛 ForegroundServiceStartNotAllowedException。
                // 这里不能让它炸掉界面：保活失败只是「没通知」，连接与推流照常。日志留痕，
                // **并且写进界面/诊断** —— 看不见的「保活没生效」等于没有保活。
                Log.e(TAG, "起前台服务失败（保活未生效）", e)
                lastError = ctx.getString(R.string.err_service_start, e.javaClass.simpleName)
            }
        }
    }

    private var wakeLock: PowerManager.WakeLock? = null

    /** 高性能 WifiLock（拿不到时为 null；界面/诊断里要看得见，不能静默）。 */
    private var wifiLock: WifiManager.WifiLock? = null

    /** 本次会话当前是否含麦克风（决定前台服务类型掩码）。 */
    @Volatile
    private var micActive = false

    /** 扬声器是否在放（只影响通知按钮图标，不改变前台服务类型）。 */
    @Volatile
    private var spkActive = false

    /** 连接状态（只影响通知里状态按钮的图标）。 */
    @Volatile
    private var connActive = Control.CONN_OFFLINE

    /** 当前是否真的有音频流（决定持不持 WakeLock）。 */
    @Volatile
    private var audioActive = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START, ACTION_UPDATE -> {
                micActive = intent.getBooleanExtra(EXTRA_MIC, false)
                spkActive = intent.getBooleanExtra(EXTRA_SPK, false)
                connActive = intent.getIntExtra(EXTRA_CONN, Control.CONN_OFFLINE)
                audioActive = intent.getBooleanExtra(EXTRA_AUDIO, micActive)
                if (audioActive) acquireWakeLock() else releaseWakeLock()
                startForegroundCompat(micActive)
            }
            ACTION_REPOST -> {
                // 用户划掉了常驻通知（Android 13+ 允许，NO_CLEAR 拦不住）。
                // 不重新挂 = 系统随时可以掐进程，所以用最近一次状态立刻补挂。
                Log.i(TAG, "常驻通知被划掉，补挂")
                startForegroundCompat(micActive)
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
    @SuppressLint("NewApi") // type != 0 只在 SDK >= Q 时成立，lint 看不到分支
    private fun startForegroundCompat(mic: Boolean) {
        val notification = buildNotification(mic, spkActive, connActive)
        val type = when {
            Build.VERSION.SDK_INT < Build.VERSION_CODES.Q -> 0   // 走两参重载
            mic -> ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            else -> ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        }
        try {
            if (type != 0) {
                startForeground(NOTIFICATION_ID, notification, type)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            noteForeground()
        } catch (e: Throwable) {
            // Android 12+ 从后台起前台服务会抛；界面会把它显示出来，不静默
            Log.e(TAG, "startForeground 失败", e)
            noteError(getString(R.string.err_start_foreground, e.javaClass.simpleName))
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
        foreground = false
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

    /**
     * 常驻通知：**没有文字，只有一行四个图标**（连接状态 / 麦克风 / 扬声器 / 重新连接）。
     *
     * 状态全部由图标表达：
     * - 连接状态：链环 = 已连接，放大镜 = 查找/连接中，断链 = 未连接（点按打开界面）；
     * - 麦克风 / 扬声器：有斜线 = 关，无斜线 = 开；
     * - 重新连接：刷新箭头（先拆旧流再重连，见 [Control.ACTION_RECONNECT]）。
     *
     * 图标与点击动作由 [ControlViews] 统一构建（桌面小组件用同一份）。
     * 每次状态变化都重建整条通知（`setOnlyAlertOnce` 保证不响铃、不闪）。
     */
    private fun buildNotification(mic: Boolean, spk: Boolean, conn: Int): Notification {
        val contentPi = Control.open(this)
        val row = ControlViews.build(this, R.layout.notif_controls, contentPi, conn, mic, spk)

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stat_mic)
            .setContentIntent(contentPi)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setCustomContentView(row)
            .setCustomBigContentView(row)
            .setStyle(NotificationCompat.DecoratedCustomViewStyle())
            .setDeleteIntent(repostIntent())
            .build()
    }

    /**
     * 通知被划掉时的回调。
     *
     * 用 `getService` 而不是广播：接收端就是本服务，不依赖界面还活着；服务已经在跑，
     * 收到 [ACTION_REPOST] 后 `startForeground` 一次就能把通知挂回去。
     */
    private fun repostIntent(): PendingIntent = PendingIntent.getService(
        this,
        4,
        Intent(this, CaptureService::class.java).setAction(ACTION_REPOST),
        PendingIntent.FLAG_IMMUTABLE,
    )

    private fun acquireWakeLock() {
        if (wakeLock == null) {
            val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG).apply {
                setReferenceCounted(false)
                // 长推流场景不给上限，但设一个远大于任何会话的兜底，避免永久持有
                acquire(4 * 60 * 60 * 1000L)
            }
        }
        acquireWifiLock()
    }

    /**
     * 屏幕关掉后 Wi-Fi **省电模式**会把包攒起来一批批送（实测 RTT 从 5 ms 涨到 55 ms、
     * 抖动 25 ms、最坏 74 ms），接收端 20 ms 的环直接被冲爆 + 饿死 —— 听感就是不断断续续。
     * 手机揣兜里放音频正是本应用的主场景，所以只要有音频在跑就持一把高性能 WifiLock。
     *
     * 失败不致命，但要能被界面/诊断看到（`CaptureService.wifiLockHeld`）。
     */
    private fun acquireWifiLock() {
        if (wifiLock != null) return
        try {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            @Suppress("DEPRECATION")
            val l = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, WIFI_LOCK_TAG)
            l.setReferenceCounted(false)
            l.acquire()
            wifiLock = l
            wifiLockHeld = l.isHeld
        } catch (e: Throwable) {
            Log.w(TAG, "WifiLock 获取失败", e)
        }
    }

    private fun releaseWakeLock() {
        val l = wakeLock ?: null
        wakeLock = null
        if (l != null) {
            try {
                if (l.isHeld) l.release()
            } catch (e: Throwable) {
                Log.w(TAG, "释放 WakeLock 异常", e)
            }
        }
        val w = wifiLock
        wifiLock = null
        wifiLockHeld = false
        if (w != null) {
            try {
                if (w.isHeld) w.release()
            } catch (e: Throwable) {
                Log.w(TAG, "释放 WifiLock 异常", e)
            }
        }
    }
}
