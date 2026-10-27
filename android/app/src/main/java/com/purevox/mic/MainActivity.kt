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

import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.provider.Settings
import android.text.Editable
import android.text.TextWatcher
import android.util.Log
import android.view.KeyEvent
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import android.widget.ArrayAdapter
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.widget.SwitchCompat
import androidx.core.content.ContextCompat
import com.google.android.material.bottomnavigation.BottomNavigationView
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.android.material.progressindicator.LinearProgressIndicator
import com.google.android.material.slider.Slider
import com.google.android.material.textfield.MaterialAutoCompleteTextView
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.purevox.mic.audio.MicSources
import com.purevox.mic.audio.MicUplink
import com.purevox.mic.audio.OpusSupport
import com.purevox.mic.audio.SpeakerDownlink
import com.purevox.mic.diag.DiagJson
import com.purevox.mic.diag.RtStats
import com.purevox.mic.net.Discovery
import com.purevox.mic.net.HealthProbe
import com.purevox.mic.net.WsClient
import com.purevox.mic.service.CaptureService
import com.purevox.mic.update.UpdateChecker
import com.purevox.mic.update.UpdateDownloader
import com.purevox.mic.update.UpdateInstaller
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

/**
 * 唯一的界面与编排者（单 Activity，Material 风格）。
 *
 * 它负责把三条独立能力接上电脑端的同一个 WebSocket：
 * 1. 手机麦克风 → 电脑（[MicUplink]）
 * 2. 电脑音频 → 手机扬声器（[SpeakerDownlink]）
 * 3. 远程输入：IME 已组合文本（`text`）+ 手机实体键（`key`）。电脑端**默认关闭**，
 *    没开开关时电脑会回 `err`，本界面把 `err` 原文显示出来，不静默丢弃。
 *
 * 界面按卡片分组（连接 / 功能 / 远程输入 / 后台保活 / 更新），底部「诊断」默认收起
 * （标题行点击展开）—— 迁移期调试信息仍在，只是不再一上来就占满整屏。
 *
 * 纪律：
 * - **收到 `ready` 之前不推流、不播放**（[applyState] 里 `ready` 是必要条件）。
 * - 状态区（在诊断区里，默认收起）分两块：**网络**（RTT 窗口 / 抖动 / 真实速率 / 结论）
 *   与**音频**（上/下行电平条、缓冲水位条、欠载 / 丢包 / 解码掩盖），再加几行累计读数；
 *   2 Hz 刷新。失败项显示「不可用 + 原因」，实时量没有样本时写「采样中…」——
 *   **绝不显示伪造的 0**。整块诊断可一键复制到剪贴板（[copyDiag]）。
 * - 所有 OkHttp 回调都切回主线程再动 UI。
 *
 * 会话保活：只要「已连接」或「正在推流」就由 [CaptureService] 常驻一条前台通知
 * （[syncSessionService]），通知里的「停止 / 断开」按钮经广播回到这里处理
 * （[notifReceiver]）—— 服务不认识协议，编排只此一处。
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "PureVoxUI"
        private const val PREFS = "purevox"
        private const val DEFAULT_HOST = "192.168.1.100"
        private const val MAX_LOG_LINES = 80
        private const val STATUS_INTERVAL_MS = 500L

        /**
         * 「补挂通知」的最小间隔。
         *
         * 常驻通知不能只靠开始时发的那一次：起前台服务可能被系统拒（后台）、通知可能被撤，
         * 失败一次就整个会话都在裸奔。这里每几秒重试一次，但不要每 500 ms 都发
         * （重发通知本身会让通知栏闪）。
         */
        private const val NOTIF_RETRY_MS = 5_000L

        /** 下行缓冲水位条的满量程：200 ms（稳态水位约 40 ms，上下浮动都正常）。 */
        private const val BUFFER_FULL_MS = 200

        /**
         * 不转发给电脑的系统键：这些键在本机有系统级语义，转走会让手机用不了（返回/主屏/音量…）。
         * 电脑端按 `keymap.rs` 映射的只是普通键区 + 功能键。
         */
        private val SYSTEM_KEYS = setOf(
            KeyEvent.KEYCODE_HOME,
            KeyEvent.KEYCODE_BACK,
            KeyEvent.KEYCODE_CALL,
            KeyEvent.KEYCODE_ENDCALL,
            KeyEvent.KEYCODE_APP_SWITCH,
            KeyEvent.KEYCODE_POWER,
            KeyEvent.KEYCODE_CAMERA,
            KeyEvent.KEYCODE_FOCUS,
            KeyEvent.KEYCODE_VOLUME_UP,
            KeyEvent.KEYCODE_VOLUME_DOWN,
            KeyEvent.KEYCODE_VOLUME_MUTE,
            KeyEvent.KEYCODE_HEADSETHOOK,
            KeyEvent.KEYCODE_NOTIFICATION,
            KeyEvent.KEYCODE_MENU,
            KeyEvent.KEYCODE_SEARCH,
        )
    }

    private val ui = Handler(Looper.getMainLooper())

    @Volatile
    private var ws: WsClient? = null
    private var uplink: MicUplink? = null
    private var downlink: SpeakerDownlink? = null

    private var host = DEFAULT_HOST

    /**
     * 电脑端的端口。**独立于 host 存**，因为它是用户可改的（NETWORK.md §4.1.1），
     * 而且改的是**电脑端**那个设置 —— 手机这边只能从地址里学（mDNS / 二维码），
     * 学到的必须记住，否则下次启动退回 59123 就「莫名连不上」。
     */
    private var port = Proto.PORT
    /** 选中的麦克风采集 API（`MediaRecorder.AudioSource`，见 [MicSources]）。 */
    private var micSourceId: Int = MicSources.ALL[0].id
    private var connState = "未连接"
    private var connecting = false
    private var lastErr = ""
    private var opusDetail = ""
    private var healthText = "未自检"

    /**
     * 诊断读数的时间序列（RTT 窗口 / 速率 / 结论）。**连接重建时换一个新的**，
     * 这样上一次会话的 RTT 与速率不会渗进新会话的读数里（见 [connect]）。
     */
    private var rt = RtStats()

    @Volatile
    private var ready = false

    @Volatile
    private var speakerOn = false

    @Volatile
    private var remoteTextOn = false

    @Volatile
    private var remoteKeysOn = false

    private var suppressTextSync = false
    private var applyingState = false

    /** 前台会话服务当前是否在跑 / 是否含麦克风类型（避免每次对账都重发通知）。 */
    private var svcRunning = false
    private var svcMic = false
    /** 通知服务当前是否真的有音频流（决定 WakeLock）。 */
    private var svcAudio = false

    /**
     * 界面是否在前台（onStart→onStop）。
     *
     * 用途只有一个：**界面开着就把通知挂着** —— 前台服务的第一次起必须在界面在前台时完成，
     * 后台里被拒的话整个会话都在裸奔（系统随时掐进程）。
     */
    private var visible = false

    /** 上次「补挂通知」的尝试时刻（`elapsedRealtime`），避免每 500 ms 重发一次通知。 */
    private var lastNotifTry = 0L

    // 应用内更新状态（手动检查、手动下载、手动安装）
    private var updateRelease: UpdateChecker.ReleaseInfo? = null
    private var updateApk: File? = null
    private var updateBusy = false
    private var updateMsg = ""

    /** 局域网自动发现。发现到就自动填地址并连上；找不到才让用户手动填。 */
    private var discovery: Discovery? = null
    /** 已通过 mDNS 发现到电脑（此时地址框只是「看一眼」，不是主路径）。 */
    private var found = false
    /** 发现到的协议版本；与本客户端不一致时明确告诉用户「客户端太旧」。 */
    private var foundProto: Int? = null

    // 界面控件
    private lateinit var etHost: EditText
    /** 电脑端口（默认 59123、可改）。与 [etHost] 分开两格，见 `readTarget`。 */
    private lateinit var etPort: EditText
    /** 打开相机扫二维码填地址（`zxing-android-embedded` 自带扫描界面）。 */
    private lateinit var btnScan: ImageButton
    // 连接做成开关：滑块位置 = 连没连上（不用读文字判状态）
    private lateinit var swConnect: MaterialSwitch
    private lateinit var navBottom: BottomNavigationView
    private var pages: List<Pair<Int, android.view.View>> = emptyList()
    /** 程序改开关状态时抑制监听器，避免「setChecked → 回调 → connect」自激。 */
    private var suppressConnectToggle = false
    private lateinit var btnHealth: Button
    private lateinit var btnRediscover: Button
    private lateinit var btnReconnect: Button
    private lateinit var tvStatus: TextView
    private lateinit var tvHealth: TextView
    private lateinit var tvErr: TextView
    private lateinit var tvNet: TextView
    private lateinit var tvAudio: TextView
    private lateinit var pbUpLevel: LinearProgressIndicator
    private lateinit var pbDownLevel: LinearProgressIndicator
    private lateinit var pbBuffer: LinearProgressIndicator
    private lateinit var tvUpLevel: TextView
    private lateinit var tvDownLevel: TextView
    private lateinit var tvBuffer: TextView
    private lateinit var btnCopyDiag: Button
    private lateinit var swMic: SwitchCompat
    private lateinit var spMicSource: MaterialAutoCompleteTextView
    private lateinit var tvHeroStatus: TextView
    private lateinit var swSpeaker: SwitchCompat
    private lateinit var slRing: Slider
    private lateinit var tvRing: TextView
    /** 程序改滑块时抑制监听器，避免「setValue → 回调 → 再发 settarget」自激。 */
    private var suppressRing = false
    private lateinit var swRemote: SwitchCompat
    private lateinit var swRemoteText: SwitchCompat
    private lateinit var swRemoteKeys: SwitchCompat
    private lateinit var etText: PvEditText
    private lateinit var btnSend: Button
    private lateinit var btnClear: Button
    private lateinit var tvBattery: TextView
    private lateinit var tvKeepalive: TextView
    private lateinit var tvNotifPerm: TextView
    private lateinit var btnNotifSettings: Button
    private lateinit var btnBattery: Button
    private lateinit var tvUpdate: TextView
    private lateinit var btnCheckUpdate: Button
    private lateinit var btnDownload: Button
    private lateinit var btnInstall: Button
    private lateinit var llDiag: LinearLayout
    private lateinit var tvLog: TextView

    /**
     * 通知栏按钮的接收端（前台服务发的是**广播**，不是服务调用）。注册在整个 Activity
     * 生命周期（`onCreate`→`onDestroy`），**不是** `onStart`→`onStop` —— 否则用户退到后台
     * 时按钮就失效了，而那恰恰是最需要它们的时刻。
     */
    private val notifReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            when (intent?.action) {
                // 三个按钮都是**切换 / 刷新**：与界面上那个开关完全等价（同一个入口）
                CaptureService.BROADCAST_NOTIF_MIC -> {
                    log("通知栏：麦克风 → ${if (swMic.isChecked) "关" else "开"}")
                    swMic.isChecked = !swMic.isChecked
                    applyState()
                    updateStatus()
                }
                CaptureService.BROADCAST_NOTIF_SPK -> {
                    log("通知栏：扬声器 → ${if (swSpeaker.isChecked) "关" else "开"}")
                    swSpeaker.isChecked = !swSpeaker.isChecked
                    applyState()
                    updateStatus()
                }
                CaptureService.BROADCAST_NOTIF_RECONNECT -> {
                    log("通知栏：重新连接（先拆掉已打开的流再重连）")
                    reconnect()
                }
            }
        }
    }

    private val ticker = object : Runnable {
        override fun run() {
            reportClientStats()
            updateStatus()
            // 保活状态跟着刷：挂了 / 没挂上（带原因）随时看得到
            updateKeepalive()
            ui.postDelayed(this, STATUS_INTERVAL_MS)
        }
    }

    /**
     * 把手机这一侧的播放 / 采集状态上报给电脑（2 Hz）。
     *
     * 电脑那边看不到手机环的水位、欠载和**实测时钟偏差**；没有这份上报，
     * 「扬声器为什么还是慢」只能靠猜（DEBUG.md §1.4）。
     */
    private fun reportClientStats() {
        if (!ready) return
        val d = downlink ?: return
        ws?.sendText(
            Proto.stats(
                levelMs = d.levelMs,
                targetMs = d.targetMs,
                underruns = d.underruns,
                concealed = d.concealed,
                late = d.late,
                oneWayMs = if (d.oneWayMs.isNaN()) null else d.oneWayMs,
                clockPpm = if (d.clockPpm.isNaN()) null else d.clockPpm,
            ),
        )
    }

    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
            applyState()
            updateStatus()
        }

    /** 扫码结果 → 自动拆成 IP + 端口填好（**不自动连接**：扫到别人的码不该断当前会话）。 */
    private val scanLauncher =
        registerForActivityResult(ScanContract()) { result ->
            val text = result.contents
            if (text.isNullOrBlank()) return@registerForActivityResult
            applyScannedTarget(text)
        }

    /** 相机权限（只在点「扫描」时申请；拒绝也不影响手动输入）。 */
    private val cameraPermLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) launchScan()
            else showErr(getString(R.string.msg_scan_camera_denied))
        }

    // ---------------------------------------------------------------- 生命周期

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        bindViews()
        restorePrefs()

        // 一行说明「现在在用什么 + 为什么不碰系统 codec」，不是探测结果而是编解码器的身份
        opusDetail = OpusSupport.describe()

        swConnect.setOnCheckedChangeListener { _, checked ->
            if (suppressConnectToggle) return@setOnCheckedChangeListener
            if (checked && ws == null && !connecting) connect() else if (!checked) disconnect()
        }

        // 底部选项卡：一次只显示一页（每页自己滚，互不干扰）
        // 第一页 = 连接 + 音频（一次会话的两半，分开要来回切）
        pages = listOf(
            R.id.tab_connect to findViewById(R.id.pageConnect),
            R.id.tab_input to findViewById(R.id.pageInput),
            R.id.tab_diag to findViewById(R.id.pageDiag),
            R.id.tab_more to findViewById(R.id.pageMore),
        )
        navBottom.setOnItemSelectedListener { item ->
            showPage(item.itemId)
            true
        }
        showPage(R.id.tab_connect)
        // 抖动缓冲滑块：**拖一下就生效**（不重建会话、不重连），并同步给电脑
        slRing.addOnChangeListener { _, value, fromUser ->
            val ms = value.toInt()
            tvRing.text = getString(R.string.ring_label, ms / 10, ms)
            if (fromUser && !suppressRing) {
                downlink?.setTargetMs(ms.toDouble())
                ws?.sendText(Proto.setTarget(ms))
                log("抖动缓冲 → $ms ms")
            }
        }
        tvRing.text = getString(
            R.string.ring_label,
            slRing.value.toInt() / 10,
            slRing.value.toInt(),
        )

        btnHealth.setOnClickListener { runHealthCheck() }
        // 换 Wi-Fi / 刚才没搜到 → 重新发现（停掉再 discover，不复用旧监听）
        btnRediscover.setOnClickListener { rediscover() }
        // 只是掉线、服务还在 → 只重连 WebSocket，不必重新发现（那要等好几秒）
        btnReconnect.setOnClickListener { reconnect() }

        // 三条路并存，谁先成谁上（NETWORK.md §4.1.1）：
        //
        //   1. **缓存** —— 上次连过的那台。手机和电脑通常在同一网络环境（同一个 Wi-Fi），
        //      地址根本没变，用户**不该每次都等发现**。绝大多数情况走这条，秒连。
        //   2. **mDNS** —— 换了 Wi-Fi / 手机热点 / 电脑 DHCP 换了地址，缓存失效时
        //      自动找回新地址。它**同时常开着**（不是「找到就停」），所以缓存先连上、
        //      发现结果和它一致时不会重复连。
        //   3. **扫码 / 手填** —— 首次配对、换电脑、或组播被交换机挡了（AP 隔离）。
        //
        // 三者不是三条实现，是**同一个「连上电脑」动作的三种拿地址方式**（AGENTS §2.1）。
        startDiscovery()
        // 缓存地址非空且不是出厂默认值（= 说明用户之前真的连过）就直接连，不问
        if (host.isNotEmpty() && host != DEFAULT_HOST) {
            log(getString(R.string.msg_reuse_cached, host))
            connect()
        }

        swMic.setOnCheckedChangeListener { _, _ ->
            applyState()
            updateStatus()
        }
        swSpeaker.setOnCheckedChangeListener { _, _ ->
            applyState()
            updateStatus()
        }
        swRemote.setOnCheckedChangeListener { _, _ ->
            refreshRemoteSwitches()
            applyState()
            updateStatus()
        }
        swRemoteText.setOnCheckedChangeListener { _, _ ->
            applyState()
            updateStatus()
        }
        swRemoteKeys.setOnCheckedChangeListener { _, _ ->
            applyState()
            updateStatus()
        }

        spMicSource.setOnItemClickListener { _, _, position, _ ->
            val id = MicSources.ALL.getOrNull(position)?.id ?: return@setOnItemClickListener
            if (id == micSourceId) return@setOnItemClickListener
            micSourceId = id
            savePrefs()
            log("麦克风 API：${MicSources.label(this, id)}")
            // 正在推流时立即换源；关闭状态下只记住，下次开启生效。
            if (swMic.isChecked && uplink != null) {
                uplink?.stop()
                uplink = null
                applyState()
            }
        }

        etText.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}

            override fun afterTextChanged(s: Editable?) {
                if (suppressTextSync) return
                val typed = s?.toString().orEmpty()
                if (typed.isNotEmpty() && remoteTextOn && ready) {
                    // 同步给电脑后清空：既让电脑拿到「已组合好」的文本，又不与本地输入打架
                    suppressTextSync = true
                    try {
                        sendText(typed)
                    } finally {
                        etText.setText("")
                        suppressTextSync = false
                    }
                }
                updateButtons()
            }
        })

        // 长文本发送 / 清空；空框里按退格 → 把删除键转发给电脑（见 PvEditText）
        etText.onDeleteWhenEmpty = { forwardBackspace() }
        btnSend.setOnClickListener { sendComposed() }
        btnClear.setOnClickListener { clearComposed() }

        // 诊断区默认收起，标题行点击展开/收起
        // 「复制诊断」：把整块诊断（含日志）塞进剪贴板 —— 出错时不必让人截图
        btnCopyDiag.setOnClickListener { copyDiag() }

        // 后台保活：电池优化白名单
        btnBattery.setOnClickListener { requestIgnoreBattery() }
        // 通知权限/渠道关掉了没有 —— 一键跳到本应用的通知设置（拒过之后系统不再弹窗）
        btnNotifSettings.setOnClickListener { openNotifSettings() }

        // 应用内更新：手动检查 → 下载 → 安装（不自动检查，见 UpdateChecker）
        btnCheckUpdate.setOnClickListener { checkUpdate() }
        btnDownload.setOnClickListener { downloadUpdate() }
        btnInstall.setOnClickListener { installUpdate() }

        // 通知栏按钮（前台服务发广播回来）
        ContextCompat.registerReceiver(
            this,
            notifReceiver,
            IntentFilter().apply {
                addAction(CaptureService.BROADCAST_NOTIF_MIC)
                addAction(CaptureService.BROADCAST_NOTIF_SPK)
                addAction(CaptureService.BROADCAST_NOTIF_RECONNECT)
            },
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )

        requestPermissions()
        refreshRemoteSwitches()
        refreshBattery()
        renderUpdate()
        updateButtons()
        log("PureVox 客户端就绪 · $opusDetail")
    }

    override fun onStart() {
        super.onStart()
        visible = true
        ui.post(ticker)
        // 进入前台就补挂通知：后台里起前台服务可能被系统拒（失败只留痕），
        // 这里是唯一可靠的「补挂」时机。
        syncSessionService()
    }

    override fun onResume() {
        super.onResume()
        // 从系统设置切回来时刷新电池优化状态
        refreshBattery()
        syncSessionService()
        updateKeepalive()
        maybeRequestNotifPermission()
    }

    override fun onStop() {
        visible = false
        ui.removeCallbacks(ticker)
        super.onStop()
    }

    override fun onPause() {
        super.onPause()
        savePrefs()
    }

    override fun onDestroy() {
        ui.removeCallbacks(ticker)
        // 发现监听必须停：`NsdManager` 的监听会一直占着（耗电），且下次启动重复注册会抛
        // `IllegalArgumentException`
        discovery?.stop()
        discovery = null
        try {
            unregisterReceiver(notifReceiver)
        } catch (e: IllegalArgumentException) {
            Log.w(TAG, "接收器已注销", e)
        }
        teardown()
        super.onDestroy()
    }

    // ---------------------------------------------------------------- 地址归一

    /**
     * 把用户填的东西归一成 `host[:port]`。
     *
     * 电脑端「手机」面板显示的是**网页地址** `http://192.168.1.5:59123/`
     * （二维码扫出来就是它，页面上还有「复制」按钮），不是 `ws://`。
     * 所以这里三种写法都要认：
     *   - `192.168.1.5:59123`   —— 手敲 / 老习惯
     *   - `192.168.1.5`         —— 端口用默认
     *   - `http://192.168.1.5:59123/`、`ws://…`、`https://…` —— 从电脑端复制来的
     * 空的一律返回空串，由调用方报「请先填写」。
     */
    private fun normalizeTarget(raw: String): String {
        var s = raw.trim()
        if (s.isEmpty()) return ""
        // 去协议头（大小写都认）
        s = s.replace(Regex("(?i)^(https?|wss?)://"), "")
        // 去路径 / 查询 / 片段
        s = s.substringBefore('/').substringBefore('?').substringBefore('#')
        // 结尾的冒号（`192.168.1.5:`）是常见的半截输入，去掉再用默认端口
        if (s.endsWith(":")) s = s.dropLast(1)
        // IPv6 字面量必须带方括号：`[fe80::1]:59123`
        if (s.startsWith("[")) {
            val close = s.indexOf(']')
            if (close < 0) return ""
            val hostPart = s.substring(0, close + 1)
            val rest = s.substring(close + 1)
            return if (rest.startsWith(":")) hostPart + rest else hostPart
        }
        // 多个冒号又没方括号 = 裸 IPv6，不补端口
        if (s.count { it == ':' } > 1) return s
        return s
    }

    /**
     * 读界面上的两个框 → `host[:port]`（默认端口时省略）。
     *
     * - IP 框里**粘整条网址**（`https://192.168.1.5:59200/`）仍然可用：这里把端口拆出来
     *   回填到端口框，用户看得见实际用的是哪个端口；
     * - 端口只在合法（1~65535）时采用，否则回到 59123 并明确提示；
     * - 同时更新 `host` / `port` 字段（`savePrefs` 也调它）。
     */
    private fun readTarget(): String {
        val raw = etHost.text.toString().trim()
        if (raw.isEmpty()) return ""
        val norm = normalizeTarget(raw)
        val explicitRaw = explicitPortRaw(norm)
        val explicit = explicitRaw?.toIntOrNull()?.takeIf { it in 1..65535 }
        if (explicitRaw != null && explicit == null) {
            showErr(getString(R.string.msg_port_invalid, explicitRaw))
        }
        val typed = etPort.text.toString().trim().toIntOrNull()?.takeIf { it in 1..65535 }
        val p = explicit ?: typed ?: Proto.PORT
        val h = if (explicitRaw != null) {
            // 地址里带了端口：host 只取冒号前那一段（IPv6 保留方括号）
            if (norm.startsWith("[")) {
                val close = norm.indexOf(']')
                if (close > 0) norm.substring(0, close + 1) else norm
            } else {
                norm.substringBeforeLast(':')
            }
        } else {
            norm
        }
        if (h.isEmpty()) return ""
        // 回填：粘贴完整网址 / 端口非法时，界面立刻显示真正会用的值
        etHost.setText(h)
        etPort.setText(p.toString())
        host = h
        port = p
        return if (p == Proto.PORT) h else "$h:$p"
    }

    /** `host[:port]` 里显式写的端口原文（没有端口 = null）；不校验合法性。 */
    private fun explicitPortRaw(norm: String): String? {
        if (norm.startsWith("[")) {
            val close = norm.indexOf(']')
            if (close <= 0) return null
            return norm.substring(close + 1).removePrefix(":").ifEmpty { null }
        }
        if (norm.count { it == ':' } == 1) return norm.substringAfterLast(':').ifEmpty { null }
        return null
    }

    /** 点「扫描」：先确认相机权限，再打开扫码界面。 */
    private fun startScan() {
        val granted = ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        if (granted) launchScan() else cameraPermLauncher.launch(Manifest.permission.CAMERA)
    }

    private fun launchScan() {
        val opts = ScanOptions().apply {
            setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            // 自己的取景界面：竖屏 + 正方形四角框（见 PvScanActivity）
            setCaptureActivity(PvScanActivity::class.java)
            setPrompt(getString(R.string.scan_prompt))
            setBeepEnabled(false)
            setOrientationLocked(false)
            setBarcodeImageEnabled(false)
        }
        try {
            scanLauncher.launch(opts)
        } catch (e: Exception) {
            showErr(getString(R.string.msg_scan_failed, e.message ?: e.javaClass.simpleName))
            log("扫码启动失败：${e.message}")
        }
    }

    /**
     * 扫到的内容写进两个框。电脑端二维码是 `https://<ip>:<端口>/`，走与粘贴相同的
     * 归一化：拆出 host + 端口；非法内容明确报错、不改现有输入。
     */
    private fun applyScannedTarget(text: String) {
        val norm = normalizeTarget(text)
        val explicit = explicitPortRaw(norm)?.toIntOrNull()?.takeIf { it in 1..65535 }
        val (h, p) = splitTarget(norm)
        if (h.isEmpty()) {
            showErr(getString(R.string.msg_scan_failed, text.take(80)))
            return
        }
        etHost.setText(h)
        etPort.setText((explicit ?: p).toString())
        log("扫码：${h}:${explicit ?: p} → 自动连接")
        // 扫完直接连：已经在连/已连着就先拆掉旧会话，再连新地址（同一台电脑也无碍）
        if (ws != null || connecting) disconnect()
        connect()
    }

    /**
     * 把 `host[:port]` 拆成 (host, port)，端口缺失或非法时用默认。
     *
     * **必须拆，不能把整个 target 当 host 传下去**：`Proto.wsUrl` 是 `"wss://$host:$port"`，
     * 直接传 `192.168.1.5:59200` 会拼出 `wss://192.168.1.5:59200:59123/ws` —— 不是合法 URL。
     * 而带端口的 target 恰恰是最常见的来源：电脑端二维码扫出来就是 `https://ip:端口/`，
     * mDNS 报上来的端口也是用户在电脑上自定义过的。
     *
     * 裸 IPv6（多个冒号、无方括号）整串当 host —— 它的地址里本来就有冒号。
     */
    private fun splitTarget(target: String): Pair<String, Int> {
        if (target.startsWith("[")) {
            val close = target.indexOf(']')
            if (close > 0) {
                val h = target.substring(0, close + 1)
                val rest = target.substring(close + 1)
                val p = if (rest.startsWith(":")) rest.drop(1).toIntOrNull() else null
                return h to (p?.takeIf { it in 1..65535 } ?: Proto.PORT)
            }
        }
        if (target.count { it == ':' } == 1) {
            val i = target.lastIndexOf(':')
            val p = target.substring(i + 1).toIntOrNull()
            if (p != null && p in 1..65535) return target.substring(0, i) to p
        }
        return target to Proto.PORT
    }

    // ---------------------------------------------------------------- 发现

    /**
     * 开始（或重新开始）自动发现。
     *
     * 「重新发现」按钮走这里：先 `stop()` 再 `start()`。**不复用旧监听** —— `NsdManager`
     * 的监听一旦注册就绑定了当时的网络状态，换了 Wi-Fi 之后它不会自己跟上，必须重来。
     */
    private fun rediscover() {
        startDiscovery()
        connState = getString(R.string.msg_discovering)
        updateStatus()
        log(getString(R.string.msg_discovering))
    }

    private fun startDiscovery() {
        discovery?.stop()
        found = false
        foundProto = null
        val d = Discovery(
            this,
            onFound = { h, p, name, proto ->
                runOnUiThread {
                    found = true
                    foundProto = proto
                    // 协议版本不符**不要连**：连上去会被服务端直接断开，只会让用户看到
                    // 一个没有解释的「已断开」。明确说是版本问题才有意义。
                    if (proto != null && proto != Proto.VERSION) {
                        connState = getString(R.string.msg_proto_mismatch, Proto.VERSION, proto)
                        updateStatus()
                        return@runOnUiThread
                    }
                    // **端口必须一起带上**：电脑端的端口是可编辑的（NETWORK.md §4.1.1），
                    // mDNS 报的是它在用的那个真实端口。只填 IP 会退回默认 59123，于是用户
                    // 改过端口之后手机这边「怎么都连不上」，而症状完全看不出是端口问题。
                    etHost.setText(h)
                    etPort.setText((if (p > 0) p else Proto.PORT).toString())
                    // **不锁输入框 / 扫描按钮**：发现只是「帮你填好」，用户随时可以改端口
                    // 或扫另一台电脑的码（锁上会让扫描按钮点了没反应，用户实测 2026-10-27）。
                    connState = getString(R.string.msg_found, name, h, p)
                    updateStatus()
                    // 自动连上：发现即连接，用户什么都不用点
                    if (ws == null && !connecting) connect()
                }
            },
            onLost = { name ->
                runOnUiThread {
                    found = false
                    if (ws == null) connState = getString(R.string.msg_lost, name)
                    updateStatus()
                }
            },
            onError = { msg ->
                runOnUiThread {
                    if (ws == null) connState = msg
                    updateStatus()
                }
            },
        )
        discovery = d
        if (!d.available) return
        d.start()
    }

    /** 只重连 WebSocket，不重新发现（服务还在时用这个，快得多）。 */
    private fun reconnect() {
        val target = readTarget()
        if (target.isEmpty()) {
            showErr(getString(R.string.msg_no_target))
            return
        }
        // 发现结果已经失效（例如换了 Wi-Fi）时，重连多半会失败，直接退回重新发现
        if (!found) {
            rediscover()
            return
        }
        ws?.disconnect()
        ws = null
        connect()
    }

    // ---------------------------------------------------------------- 连接

    private fun connect() {
        val target = readTarget()
        if (target.isEmpty()) {
            showErr(getString(R.string.msg_no_target))
            return
        }
        // 端口单独存：mDNS 报上来的、以及用户从二维码复制来的地址都带自定义端口，
        // 只存 host 的话下次启动就退回默认端口，用户会看到「突然连不上了」
        val (h, p) = splitTarget(target)
        host = h
        port = p
        savePrefs()
        connecting = true
        ready = false
        rt = RtStats()
        lastErr = ""
        connState = "连接中…"
        val client = WsClient(h, p, wsListener)
        ws = client
        client.connect()
        updateButtons()
        updateStatus()
        log("连接 ${Proto.wsUrl(host, port)}")
    }


    private fun disconnect() {
        val c = ws
        ws = null
        connecting = false
        c?.disconnect()
        onSessionEnded("已手动断开")
    }

    /** 断开 / 失败 / 电脑主动关闭后的统一收尾：停三条链路、清 ready。 */
    private fun onSessionEnded(reason: String) {
        ws = null
        connecting = false
        ready = false
        rt = RtStats()
        connState = "已断开"
        applyState()
        updateButtons()
        updateStatus()
        log(reason)
    }

    private val wsListener = object : WsClient.Listener {
        override fun onOpen(client: WsClient) {
            ui.post {
                if (client !== ws) return@post // 旧连接的迟到回调，丢掉
                connState = "已连接（等待 ready）"
                updateButtons()
                updateStatus()
                log("WebSocket 已连接，已发送 hello(proto=${Proto.VERSION})")
            }
        }

        override fun onMessage(client: WsClient, msg: ServerMsg) {
            ui.post {
                if (client !== ws) return@post
                when (msg) {
                    is ServerMsg.Ready -> onReady(msg)
                    is ServerMsg.Err -> showErr("电脑：${msg.msg}")
                    is ServerMsg.Pong -> Unit
                    // 时钟同步由 WsClient 自己处理（它要回 syncreply、存 offset），界面不需要知道
                    is ServerMsg.Sync, is ServerMsg.Offset -> Unit
                    is ServerMsg.Target -> {
                        // 电脑（或另一台客户端）改了环深度：这边跟着改，界面同步
                        applyRingTarget(msg.ms)
                        log("电脑把抖动缓冲改成 ${msg.ms} ms")
                    }
                    is ServerMsg.Other -> log("未识别的消息 t=${msg.type}")
                }
                updateStatus()
            }
        }

        override fun onAudioPacket(client: WsClient, seq: Int, tsUs: Int, payload: ByteArray) {
            // 旧连接可能还在吐最后一包：不能喂给新会话的下行环
            if (client !== ws) return
            val d = downlink
            // 缓冲在接收方（`audio/PlaybackRing.kt`）：这里只是「解码 → 塞进环」
            if (d != null && speakerOn) d.onPacket(seq, tsUs, payload)
        }

        override fun onPong(client: WsClient, id: Long, rtt: Double) {
            // RTT 统计与界面刷新都在主线程做：RtStats 内部不加锁（调用方保证单线程）
            ui.post {
                if (client !== ws) return@post
                rt.onRtt(rtt)
            }
        }

        override fun onClosed(client: WsClient, reason: String) {
            ui.post {
                if (client !== ws) return@post
                onSessionEnded("电脑关闭了连接：$reason")
            }
        }

        override fun onFailure(client: WsClient, reason: String) {
            ui.post {
                if (client !== ws) return@post
                onSessionEnded("连接失败：$reason")
            }
        }
    }

    private fun onReady(msg: ServerMsg.Ready) {
        ready = true
        connecting = false
        connState = "就绪"
        if (msg.proto != Proto.VERSION) {
            showErr("协议版本不符：电脑 ${msg.proto}，手机 ${Proto.VERSION}")
        }
        if (msg.codec.isNotEmpty() && msg.codec != Proto.CODEC) {
            showErr("电脑音频编码是 ${msg.codec}，手机只支持 ${Proto.CODEC}")
        }
        if (msg.rate > 0 && msg.rate != Proto.SAMPLE_RATE) {
            showErr("电脑音频是 ${msg.rate} Hz，手机播放按 ${Proto.SAMPLE_RATE} Hz 打开")
        }
        // 深度以电脑下发的为准（两侧同一个值）：手机改了电脑广播回来、电脑改了也走这里
        applyRingTarget(msg.targetMs)
        log(
            "ready · proto=${msg.proto} codec=${msg.codec} rate=${msg.rate} " +
                "ch=${msg.channels} frame=${msg.frameMs}ms 目标环=${msg.targetMs}ms " +
                "电脑侧客户端=${msg.clients}",
        )
        // sub 由 applyStateInner 在开启扬声器时发；这里不重复发（服务端虽然会替换旧订阅，
        // 但没必要每次 ready 都发一条）。
        applyState()
        updateButtons()
    }

    private fun runHealthCheck() {
        val target = readTarget()
        if (target.isNotEmpty()) host = target
        healthText = "自检中…"
        updateStatus()
        HealthProbe.check(host, port) { ok, text ->
            ui.post {
                healthText = if (ok) text else "不可用 · $text"
                log("GET /health：$healthText")
                updateStatus()
            }
        }
    }

    // ---------------------------------------------------------------- 三条链路的编排

    /**
     * 唯一的「期望 vs 实际」对账入口。所有开关变化 / 连接状态变化 / 权限变化都走这里，
     * 不在别处启停链路（一个功能只有一条实现路径）。
     *
     * 带重入保护：开关回调会再次调用本方法（改 `isChecked` 触发监听器）。
     */
    private fun applyState() {
        if (applyingState) return
        applyingState = true
        try {
            applyStateInner()
        } finally {
            applyingState = false
        }
    }

    private fun applyStateInner() {
        val wantMic = swMic.isChecked && ready && hasMicPermission()
        if (wantMic && uplink == null) {
            if (!startUplink()) {
                swMic.isChecked = false
                updateButtons()
            }
        } else if (!wantMic && uplink != null) {
            uplink?.stop()
            uplink = null
            log("麦克风上行已停止")
        }

        speakerOn = swSpeaker.isChecked && ready
        if (speakerOn && downlink == null) {
            // 单向延迟要用电脑算出的时钟偏移（`sync`/`offset`，还没测出来时返回 null）
            val d = SpeakerDownlink(clockOffsetUs = { ws?.clockOffsetUs })
            if (d.start()) {
                downlink = d
                ws?.sendText(Proto.sub())
                log("电脑音频播放已开启（解码 ${d.decoderName}）")
            } else {
                speakerOn = false
                swSpeaker.isChecked = false
                showErr(d.lastError ?: "下行播放启动失败")
            }
        } else if (!speakerOn && downlink != null) {
            downlink?.stop()
            downlink = null
            // 有 unsub 就退订，别让电脑继续往没人取的队列里推（水位顶格、白占 CPU）
            ws?.sendText(Proto.unsub())
            log("电脑音频播放已停止（已退订）")
        }

        remoteTextOn = swRemote.isChecked && swRemoteText.isChecked && ready
        remoteKeysOn = swRemote.isChecked && swRemoteKeys.isChecked && ready
        syncSessionService()
        updateButtons()
    }

    /**
     * 会话前台服务对账：**已连接 / 正在推流 / 连接中 / 界面在前台** 就常驻通知（保活）。
     *
     * 2026-10-18 用户实测结论：「手机还是要常驻通知，不然还是会被杀掉」——
     * 所以这里刻意**宽**：界面只要开着就把通知挂着（后台里起前台服务会被系统拒绝，
     * 那一次失败没有第二次机会，等于整个会话都在裸奔）。真正撤掉只在
     * [teardown]（用户主动断开 / 退出界面）时发生。
     *
     * 只在「该不该跑 / 含不含麦克风」变化时下发，避免每次对账都重发通知；
     * `force = true` 用于**补挂**（通知其实没挂上，去重逻辑会挡住重发）。
     */
    private fun syncSessionService(force: Boolean = false) {
        val mic = uplink != null
        val audio = mic || downlink != null
        val keep = ready || mic || connecting || visible
        if (!keep) {
            if (svcRunning) {
                CaptureService.stop(this)
                svcRunning = false
                svcMic = false
                svcAudio = false
            }
            return
        }
        if (!force && svcRunning && svcMic == mic && svcAudio == audio) return
        svcRunning = true
        svcMic = mic
        svcAudio = audio
        val text = when {
            mic -> getString(R.string.notif_default)
            ready -> getString(R.string.notif_connected)
            else -> getString(R.string.notif_idle)
        }
        CaptureService.update(this, text, mic, audio, spk = speakerOn)
        updateKeepalive()
    }

    /**
     * 「后台保活」卡片上的通知状态：挂了 / 没挂 / 没挂上（带原因）。
     *
     * 三种状态必须分得开：**没挂上**（系统拒绝起前台服务）和**没挂**（会话结束了）
     * 对用户来说意义完全不同 —— 前者要等切回前台补挂，后者是正常状态。
     *
     * 顺带**补挂**：会话要通知、通知却不在（被系统撤了、起前台服务被拒过、进程被掐过之后
     * 又回到前台），这里每几秒重发一次。常驻通知不能只靠「开始时发过一次」——
     * 那一次失败就整个会话都在裸奔（用户 2026-10-18 的实测结论）。
     */
    private fun updateKeepalive() {
        val want = ready || svcMic || uplink != null || connecting || visible
        if (want && !CaptureService.foreground) {
            val now = SystemClock.elapsedRealtime()
            if (now - lastNotifTry >= NOTIF_RETRY_MS) {
                lastNotifTry = now
                syncSessionService(force = true)
            }
        }
        val s = when {
            CaptureService.foreground -> getString(R.string.keepalive_notif_on)
            !svcRunning -> getString(R.string.keepalive_notif_off)
            CaptureService.lastError != null ->
                getString(R.string.keepalive_notif_failed, CaptureService.lastError)
            else -> getString(R.string.keepalive_notif_off)
        }
        tvKeepalive.text = s
        tvNotifPerm.text = getString(notifPermissionText())
    }

    /** 通知权限 / 通知渠道的**可读**结论：调不到就直说，不假装正常。 */
    private fun notifPermissionText(): Int {
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return if (nm.areNotificationsEnabled()) R.string.notif_perm_on else R.string.notif_perm_off
        }
        if (!nm.areNotificationsEnabled()) return R.string.notif_perm_off
        val ch = nm.getNotificationChannel(CaptureService.CHANNEL_ID)
        if (ch != null && ch.importance == NotificationManager.IMPORTANCE_NONE) {
            return R.string.notif_perm_channel_off
        }
        return R.string.notif_perm_on
    }

    /**
     * 打开本应用的通知设置。
     *
     * **必须能手动兜底**：Android 13+ 的通知权限只要被拒过一次，弹窗就不再出现
     * （再调 [requestPermissions] 也静默返回），此时唯一的路就是设置页 ——
     * 用户报告「没看到通知权限弹窗」（2026-10-18）正是这种情况。
     */
    private fun openNotifSettings() {
        val nm = getSystemService(NotificationManager::class.java)
        if (nm.areNotificationsEnabled()) {
            // 权限在，但渠道可能被单独关掉 —— 直接进本应用的渠道页
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                startActivity(
                    Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
                        .putExtra(Settings.EXTRA_CHANNEL_ID, CaptureService.CHANNEL_ID),
                )
                return
            }
        }
        val intent = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
        } else {
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
                .setData(Uri.fromParts("package", packageName, null))
        }
        startActivity(intent)
    }

    /** 起上行：先起前台服务（Android 9+ 后台麦克风的前提），再开 AudioRecord。 */
    private fun startUplink(): Boolean {
        CaptureService.start(
            this,
            getString(R.string.notif_starting),
            mic = true,
            audio = true,
            spk = speakerOn,
        )
        svcRunning = true
        svcMic = true
        svcAudio = true
        val u = MicUplink(
            context = this,
            onPacket = { pkt, tsUs -> ws?.sendAudio(pkt, pkt.size, tsUs) },
            onError = { reason -> ui.post { showErr(reason) } },
            onWarning = { reason ->
                ui.post {
                    log(reason)
                    Toast.makeText(this, reason, Toast.LENGTH_LONG).show()
                }
            },
            sourceId = micSourceId,
        )
        if (!u.start()) {
            CaptureService.stop(this)
            svcRunning = false
            svcMic = false
            svcAudio = false
            return false
        }
        uplink = u
        log(
            "麦克风上行已启动：${u.sourceName} · 采集 ${OpusSupport.pcmName(u.recordEncoding)} " +
                "@${MicUplink.SAMPLE_RATE}Hz → 编码 ${u.encoderName}",
        )
        return true
    }

    private fun teardown() {
        uplink?.stop()
        uplink = null
        downlink?.stop()
        downlink = null
        CaptureService.stop(this)
        svcRunning = false
        svcMic = false
        svcAudio = false
        ws?.disconnect()
        ws = null
        ready = false
        connecting = false
    }

    // ---------------------------------------------------------------- 远程输入

    /**
     * 实体键转发：只转发**硬件**事件（`deviceId >= 0`；输入法合成的键 `deviceId == -1`，
     * 那条路走 `text`，避免同一次按键既发 key 又发 text）。系统键（返回/主屏/音量…）不转发，
     * 否则手机自己就用不了了。转发成功即**吞掉**事件，交给电脑去处理。
     */
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (remoteKeysOn && ready) {
            val code = event.keyCode
            if (code != 0 && code !in SYSTEM_KEYS && event.deviceId >= 0) {
                sendKey(code, event.action == KeyEvent.ACTION_DOWN)
                return true
            }
        }
        return super.dispatchKeyEvent(event)
    }

    private fun sendKey(code: Int, down: Boolean) {
        ws?.sendText(Proto.key(code, down))
    }

    private fun sendText(s: String) {
        if (!remoteTextOn || !ready) return
        ws?.sendText(Proto.text(s))
    }

    /**
     * 长文本「发送」：把输入框里已组合的整段文本一次性交给电脑，然后清空本框。
     * 与「打字同步」的逐次转发共用同一个 `text` 消息（一条实现路径），只是时机不同：
     * 打字同步是每敲一下就发，这里是攒够了一整段再发。
     */
    private fun sendComposed() {
        val s = etText.text?.toString().orEmpty()
        if (s.isEmpty() || !remoteTextOn || !ready || ws == null) {
            showErr(getString(R.string.msg_send_blocked))
            return
        }
        ws?.sendText(Proto.text(s))
        log(getString(R.string.msg_sent, s.length))
        clearComposed()
    }

    /** 只清本机输入框（不动电脑）。走 [suppressTextSync] 避免触发自动同步。 */
    private fun clearComposed() {
        suppressTextSync = true
        try {
            etText.setText("")
        } finally {
            suppressTextSync = false
        }
        updateButtons()
    }

    /**
     * 输入框为空时按删除键 → 把 Android 删除键（`KEYCODE_DEL`）发给电脑
     * （电脑端 `keymap.rs` 映射成 PC Backspace）。返回「是否已转发」；没开远程输入时
     * 返回 `false`，交回默认行为（什么也不发生）。down+up 成对发，等同一次真实按键。
     */
    private fun forwardBackspace(): Boolean {
        if (!remoteTextOn || !ready || ws == null) return false
        sendKey(KeyEvent.KEYCODE_DEL, true)
        sendKey(KeyEvent.KEYCODE_DEL, false)
        return true
    }

    /**
     * 切页：只显示一个 ScrollView。
     *
     * **不重建任何视图**：所有页都在同一棵树上，切页只是 visibility 变化 ——
     * 这样切换不会打断正在跑的音频/日志刷新，也不需要每个页面各存一份状态。
     */
    private fun showPage(itemId: Int) {
        if (pages.isEmpty()) return
        for ((tab, page) in pages) {
            page.visibility = if (tab == itemId) android.view.View.VISIBLE else android.view.View.GONE
        }
        if (itemId == R.id.tab_diag) updateStatus()
    }

    private fun refreshRemoteSwitches() {
        swRemoteText.isEnabled = swRemote.isChecked
        swRemoteKeys.isEnabled = swRemote.isChecked
    }

    // ---------------------------------------------------------------- 权限

    private fun hasMicPermission(): Boolean =
        ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    private fun requestPermissions() {
        val want = ArrayList<String>()
        if (!hasMicPermission()) want.add(Manifest.permission.RECORD_AUDIO)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val p = Manifest.permission.POST_NOTIFICATIONS
            if (ContextCompat.checkSelfPermission(this, p) != PackageManager.PERMISSION_GRANTED) {
                want.add(p)
            }
        }
        if (want.isNotEmpty()) {
            permissionLauncher.launch(want.toTypedArray())
        }
    }

    /**
     * 通知权限的**第二次机会**。
     *
     * Android 13+ 的 POST_NOTIFICATIONS 被拒一次后系统仍允许再弹（此时
     * `shouldShowRequestPermissionRationale` = true）；再拒一次就永久静默 —— 那时唯一的
     * 出路是设置页，界面上有「通知设置」按钮兜底（[openNotifSettings]）。
     * 用户报告「从没见过通知权限弹窗」（2026-10-18），所以这条补一次是必要的。
     */
    private fun maybeRequestNotifPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val p = Manifest.permission.POST_NOTIFICATIONS
        if (ContextCompat.checkSelfPermission(this, p) == PackageManager.PERMISSION_GRANTED) return
        if (!shouldShowRequestPermissionRationale(p)) return
        permissionLauncher.launch(arrayOf(p))
    }

    // ---------------------------------------------------------------- 状态显示

    private fun updateButtons() {
        val online = ready
        // 开关 = 状态显示：连着（或正在连）就在「开」的位置
        suppressConnectToggle = true
        swConnect.isChecked = ws != null || connecting
        suppressConnectToggle = false
        swConnect.isEnabled = !connecting
        swMic.isEnabled = online
        swSpeaker.isEnabled = online
        swRemote.isEnabled = online
        swRemoteText.isEnabled = online && swRemote.isChecked
        swRemoteKeys.isEnabled = online && swRemote.isChecked
        etText.hint = if (remoteTextOn) {
            getString(R.string.hint_text_synced)
        } else {
            getString(R.string.hint_text_local)
        }
        val hasText = !etText.text.isNullOrEmpty()
        btnSend.isEnabled = hasText && remoteTextOn
        btnClear.isEnabled = hasText
        btnCheckUpdate.isEnabled = !updateBusy
        btnDownload.isEnabled = !updateBusy && updateRelease != null && updateApk == null
        btnInstall.isEnabled = !updateBusy && updateApk != null
    }

    /** 诊断区刷新：网络统计 + 音频电平/缓冲条 + 固定读数行（行数固定，数值变长也不跳）。 */
    private fun updateStatus() {
        val c = ws
        val enc = uplink
        val dec = downlink
        tvHeroStatus.text = connState
        tvHeroStatus.setTextColor(
            if (c != null) 0xFF9EF0C0.toInt() else 0xFFFFFFFF.toInt(),
        )
        // 实时量全交给 RtStats（单一来源）：界面只渲染，不自己另算一套
        val net = rt.sample(
            nowMs = System.currentTimeMillis(),
            upPackets = c?.txPackets?.get() ?: 0L,
            upBytes = c?.txBytes?.get() ?: 0L,
            downPackets = c?.rxPackets?.get() ?: 0L,
            downBytes = c?.rxBytes?.get() ?: 0L,
            underruns = dec?.underruns ?: 0L,
            dropped = dec?.dropped ?: 0L,
            concealed = dec?.concealed ?: 0L,
        )

        // ---- 网络：RTT 窗口 + 抖动 + 真实速率（字节差分）+ 结论
        val q = rt.netVerdict(net, c != null)
        tvNet.text = buildString {
            if (net.rttSamples == 0) {
                append(row(getString(R.string.status_rtt), "尚无 pong（1 s 一次）"))
                append(row(getString(R.string.diag_jitter), "—"))
            } else {
                append(
                    row(
                        getString(R.string.status_rtt),
                        "最近 ${ms(net.rttLast)} · 平均 ${ms(net.rttAvg)} · 最大 ${ms(net.rttMax)}（${net.rttSamples} 样本）",
                    ),
                )
                append(row(getString(R.string.diag_jitter), ms(net.jitter)))
            }
            append(row(getString(R.string.status_up), rateText(net.upPps, net.upKbps, c?.txPackets?.get())))
            append(row(getString(R.string.status_down), rateText(net.downPps, net.downKbps, c?.rxPackets?.get())))
            append(row(getString(R.string.diag_quality), "${q.label} · ${q.detail}"))
        }

        // ---- 音频：电平/缓冲用条，数值仍等宽写在右侧
        val upLevel = enc?.level ?: 0f
        val downPeak = dec?.peak ?: 0f
        pbUpLevel.progress = rt.levelPercent(upLevel)
        tvUpLevel.text = if (enc == null) "未开启" else dbfs(upLevel)
        pbDownLevel.progress = rt.levelPercent(downPeak)
        tvDownLevel.text = if (dec == null) "未开启" else dbfs(downPeak)

        val bufferedMs = dec?.levelMs?.toInt() ?: 0
        pbBuffer.progress = (bufferedMs * 100 / BUFFER_FULL_MS).coerceIn(0, 100)
        tvBuffer.text = if (dec == null) "未开启" else "$bufferedMs ms"

        val a = rt.audioVerdict(net, dec != null)
        tvAudio.text = buildString {
            append(
                row(
                    getString(R.string.diag_chain),
                    if (enc == null && dec == null) {
                        "上行/下行都未开启"
                    } else {
                        (enc?.let { "编码 ${it.encoderName} · 采集 ${OpusSupport.pcmName(it.recordEncoding)}" }
                            ?: "未上行") + " → " + (dec?.let { "解码 ${it.decoderName}" } ?: "未订阅")
                    },
                ),
            )
            if (enc != null) {
                append(
                    row(
                        getString(R.string.status_up),
                        "编出 ${enc.encoderPackets} 包 · 丢 ${enc.encoderDroppedSamples} 样本",
                    ),
                )
            }
            append(
                row(
                    getString(R.string.status_play),
                    if (dec == null) {
                        "未开启"
                    } else {
                        "欠载 ${dec.underruns}（${fmt1(net.underrunPps)}/s）" +
                            " · 丢包 ${dec.dropped}（${fmt1(net.droppedPps)}/s）" +
                            " · 掩盖 ${dec.concealed}（${fmt1(net.concealedPps)}/s）" +
                            " · 底层欠载 ${dec.deviceUnderruns}" +
                            (dec.lastError?.let { " · $it" } ?: "")
                    },
                ),
            )
            if (dec != null) {
                append(
                    row(
                        getString(R.string.diag_latency),
                        "环 ${bufferedMs}/${dec.targetMs.toInt()} ms · " +
                            "单向 ${if (dec.oneWayMs.isNaN()) "测量中" else fmt1(dec.oneWayMs) + " ms"} · " +
                            "底层 ${dec.trackBufferMs} ms · " +
                            "时钟 ${if (dec.clockPpm.isNaN()) "采样中" else "%+.0f ppm".format(dec.clockPpm)}",
                    ),
                )
            }
            append(row(getString(R.string.diag_audio), "${a.label} · ${a.detail}"))
        }

        // ---- 读数行（身份与累计信息；累计包数已在上面的速率行里，不重复）
        val line = StringBuilder()
        line.append(row(getString(R.string.status_conn), connState))
        line.append(row(getString(R.string.status_host), Proto.wsUrl(host, port)))
        line.append(row(getString(R.string.status_opus), opusDetail))
        tvStatus.text = line.toString()

        tvHealth.text = getString(R.string.status_health) + healthText
        tvErr.text = lastErr
        tvErr.visibility = if (lastErr.isEmpty()) android.view.View.GONE else android.view.View.VISIBLE

        // ---- 结构化诊断（DEBUG.md §1.4）：同一份读数同时落盘 + 打 logcat，
        // 脚本/AI 不必截图、不必点界面（`adb logcat -s PureVoxDiag:I` / `run-as ... cat files/diag.json`）
        val diagIssue = DiagJson.publish(this, diagJson(net, q, a), System.currentTimeMillis())
        if (diagIssue != null) log(diagIssue)
    }

    /**
     * 当前状态 → 结构化 JSON（`schema_version` + 单位写进字段名，缺测用 null 不用 0）。
     *
     * 字段名与电脑侧 `/debug` 的 `net` 段保持一致（`level_ms` / `target_ms` / `late` /
     * `underruns` / `clock_ppm` …），两边对着看不用翻译。
     */
    private fun diagJson(net: RtStats.Net, q: RtStats.Verdict, a: RtStats.Verdict): String {
        fun num(v: Double): Any = if (v.isNaN() || v.isInfinite()) JSONObject.NULL else v

        val root = JSONObject()
        root.put("schema_version", DiagJson.SCHEMA_VERSION)
        root.put("ts_ms", System.currentTimeMillis())
        root.put("app", getString(R.string.app_name))
        root.put("diag_file", DiagJson.path(this))

        root.put(
            "conn",
            JSONObject().apply {
                put("state", connState)
                put("host", host)
                put("port", port)
                put("url", Proto.wsUrl(host, port))
                put("ready", ready)
                put("connecting", connecting)
                put("found_by_mdns", found)
                put("rtt_ms", num(net.rttLast))
                put("rtt_avg_ms", num(net.rttAvg))
                put("rtt_max_ms", num(net.rttMax))
                put("rtt_jitter_ms", num(net.jitter))
                put("rtt_samples", net.rttSamples)
                put("up_packets_per_s", num(net.upPps))
                put("down_packets_per_s", num(net.downPps))
                put("up_kbps", num(net.upKbps))
                put("down_kbps", num(net.downKbps))
                put("clock_offset_us", ws?.clockOffsetUs ?: JSONObject.NULL)
                put("verdict", "${q.label} · ${q.detail}")
            },
        )

        val enc = uplink
        root.put(
            "uplink",
            JSONObject().apply {
                put("on", enc != null)
                if (enc != null) {
                    put("mic_api", MicSources.label(this@MainActivity, enc.sourceUsed))
                    put("mic_source", enc.sourceName)
                    put("capture_format", OpusSupport.pcmName(enc.recordEncoding))
                    put("encoder", enc.encoderName)
                    put("packets", enc.encoderPackets)
                    put("dropped_samples", enc.encoderDroppedSamples)
                    put("level", enc.level.toDouble())
                }
            },
        )

        val dec = downlink
        root.put(
            "downlink",
            JSONObject().apply {
                put("on", dec != null)
                if (dec != null) {
                    put("decoder", dec.decoderName)
                    put("sample_rate", dec.trackSampleRate)
                    put("buffer_ms", dec.trackBufferMs)
                    put("buffer_requested_ms", dec.bufferRequestedMs)
                    put("low_latency_path", dec.lowLatencyPath)
                    put("device_underruns", dec.deviceUnderruns)
                    put("level_ms", num(dec.levelMs))
                    put("target_ms", num(dec.targetMs))
                    put("jitter_ms", num(dec.jitterMs))
                    put("worst_jitter_ms", num(dec.worstJitterMs))
                    put("max_gap_ms", num(dec.maxGapMs))
                    put("packets", dec.packets)
                    put("hops", dec.hops)
                    put("late", dec.late)
                    put("trims", dec.trims)
                    put("dropped_samples", dec.dropped)
                    put("underruns", dec.underruns)
                    put("plc", dec.plc)
                    put("concealed", dec.concealed)
                    put("gap_over_ring", dec.gapOverRing)
                    put("spliced_hops", dec.splicedHops)
                    put("max_sample_delta", dec.maxSampleDelta.toDouble())
                    put("clock_ppm", num(dec.clockPpm))
                    put("one_way_ms", num(dec.oneWayMs))
                    put("peak", dec.peak.toDouble())
                    put("verdict", "${a.label} · ${a.detail}")
                }
            },
        )

        root.put(
            "service",
            JSONObject().apply {
                put("foreground_notification", CaptureService.foreground)
                put("wifi_lock", CaptureService.wifiLockHeld)
                put("keepalive_error", CaptureService.lastError ?: JSONObject.NULL)
            },
        )

        root.put(
            "log",
            JSONArray().also { arr ->
                val lines = tvLog.text.toString().split("\n").filter { it.isNotBlank() }
                for (line in lines.takeLast(40)) arr.put(line)
            },
        )
        return root.toString()
    }

    /**
     * 应用环深度（来自 `ready` / `target`）：环、滑块、读数一起改。
     *
     * 抑制监听器是关键：`Slider.setValue` 会触发 `OnChangeListener`，不抑制就会
     * 「setValue → 回调 → 发 settarget → 电脑再广播 → setValue」自激。
     */
    private fun applyRingTarget(ms: Int) {
        if (ms <= 0) return
        downlink?.setTargetMs(ms.toDouble())
        suppressRing = true
        try {
            slRing.value = ms.toFloat().coerceIn(slRing.valueFrom, slRing.valueTo)
        } finally {
            suppressRing = false
        }
        tvRing.text = getString(R.string.ring_label, ms / 10, ms)
    }

    /**
     * 上/下行速率。没连上就说「未连接」、还没到第一次采样就说「采样中」——
     * **不拿 0 冒充**（0 包/s 与「根本没连上」是两件事）。
     */
    private fun rateText(pps: Double, kbps: Double, total: Long?): String = when {
        total == null -> "未连接"
        pps.isNaN() -> "采样中…"
        else -> "${fmt1(pps)} 包/s · ${fmt1(kbps)} kbps · 累计 $total 包"
    }

    /** 电平（线性峰值 0..1）→ dBFS 读数；太小直接报「静音」，免得出现 -inf。 */
    private fun dbfs(level: Float): String {
        if (level <= 1e-5f) return "静音"
        return String.format("%.1f dBFS", 20.0 * kotlin.math.log10(level.toDouble()))
    }

    private fun ms(v: Double): String = if (v.isNaN()) "—" else String.format("%.1f ms", v)

    private fun fmt1(v: Double): String = if (v.isNaN()) "—" else String.format("%.1f", v)

    /** 等宽读数的列宽固定，数值变长也不跳布局（对齐靠字符数，不靠固定高度）。 */
    private fun row(label: String, value: String, width: Int = 8): String =
        String.format("%-${width}s %s%n", label, value)

    private fun showErr(msg: String) {
        lastErr = msg
        Log.w(TAG, msg)
        log("! $msg")
        updateStatus()
    }

    private fun log(msg: String) {
        ui.post {
            val t = java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault())
                .format(java.util.Date())
            tvLog.append("[$t] $msg\n")
            val lines = tvLog.text.toString().split("\n")
            if (lines.size > MAX_LOG_LINES) {
                tvLog.text = lines.subList(lines.size - MAX_LOG_LINES, lines.size).joinToString("\n")
            }
            // **不要**在这里 scrollTo：日志区在一条常显的页里，每来一行就把正文顶一下
            // 看起来就是「整块在闪」。保留最近 MAX_LOG_LINES 行、不动视口即可。
        }
    }

    // ---------------------------------------------------------------- 诊断区 / 保活 / 更新

    /**
     * 把整块诊断复制到剪贴板（网络 + 音频 + 读数 + 日志）。
     *
     * 存在的意义：手机上报问题最省事的方式就是复制一段文本，不必让人对着屏幕截图，
     * 也免得「日志只留在设备上、退一下就没了」。
     */
    private fun copyDiag() {
        val text = buildString {
            append(getString(R.string.diag_net)).append('\n')
            append(tvNet.text)
            append('\n').append(getString(R.string.diag_audio)).append('\n')
            append(tvAudio.text)
            append('\n')
            append(tvStatus.text)
            append(tvHealth.text).append('\n')
            append(tvKeepalive.text).append('\n')
            append(tvNotifPerm.text).append('\n')
            append(tvBattery.text).append('\n')
            if (lastErr.isNotEmpty()) append(lastErr).append('\n')
            append('\n').append(tvLog.text).append('\n')
        }
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("PureVox 诊断", text))
        Toast.makeText(this, R.string.diag_copied, Toast.LENGTH_SHORT).show()
        log("诊断已复制到剪贴板（${text.length} 字）")
    }

    private fun isIgnoringBattery(): Boolean {
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        return pm.isIgnoringBatteryOptimizations(packageName)
    }

    private fun refreshBattery() {
        tvBattery.setText(
            if (isIgnoringBattery()) R.string.battery_ignored else R.string.battery_not_ignored,
        )
    }

    /**
     * 请求「忽略电池优化」。用系统弹窗（`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`）
     * 而不是自己写白名单页 —— 是否放行由系统决定，我们只是发起请求。
     * 需要 `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` 权限（manifest 已声明）。
     */
    @SuppressLint("BatteryLife")
    private fun requestIgnoreBattery() {
        if (isIgnoringBattery()) {
            refreshBattery()
            return
        }
        val req = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
            .setData(Uri.parse("package:$packageName"))
        try {
            startActivity(req)
        } catch (e: Exception) {
            // 个别 ROM 没有该弹窗 → 退到电池优化列表，用户仍能手动加白
            try {
                startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
            } catch (e2: Exception) {
                showErr("无法打开电池优化设置：${e2.message ?: e2.javaClass.simpleName}")
            }
        }
    }

    /** 本机版本 + 更新状态两行（第一行恒在，第二行随检查 / 下载推进）。 */
    private fun renderUpdate() {
        val base = getString(
            R.string.update_installed,
            UpdateChecker.installedName(this),
            UpdateChecker.installedCode(this).toString(),
        )
        tvUpdate.text = if (updateMsg.isEmpty()) base else "$base\n$updateMsg"
    }

    private fun checkUpdate() {
        if (updateBusy) return
        updateBusy = true
        updateMsg = getString(R.string.update_checking)
        renderUpdate()
        updateButtons()
        UpdateChecker.check(this) { res ->
            ui.post {
                updateBusy = false
                when (res) {
                    is UpdateChecker.UpdateResult.DevBuild ->
                        updateMsg = getString(R.string.update_dev_build)

                    is UpdateChecker.UpdateResult.UpToDate ->
                        updateMsg = getString(R.string.update_uptodate)

                    is UpdateChecker.UpdateResult.Failed ->
                        updateMsg = getString(R.string.update_failed, res.reason)

                    is UpdateChecker.UpdateResult.Available -> {
                        updateRelease = res.release
                        updateApk = null
                        var m = getString(R.string.update_available, res.release.displayVersion)
                        val notes = res.release.notes.trim()
                        if (notes.isNotEmpty()) m += "\n" + notes.take(400)
                        updateMsg = m
                    }
                }
                log("更新检查：$updateMsg")
                renderUpdate()
                updateButtons()
            }
        }
    }

    private fun downloadUpdate() {
        val rel = updateRelease ?: return
        if (updateBusy) return
        updateBusy = true
        updateMsg = getString(R.string.update_downloading, 0)
        renderUpdate()
        updateButtons()
        UpdateDownloader.download(
            this,
            rel,
            onProgress = { pct ->
                ui.post {
                    updateMsg = getString(R.string.update_downloading, pct)
                    renderUpdate()
                }
            },
            onDone = { file, err ->
                ui.post {
                    updateBusy = false
                    if (file != null) {
                        updateApk = file
                        updateMsg = getString(R.string.update_downloaded)
                    } else {
                        updateMsg = getString(R.string.update_failed, err ?: "未知错误")
                    }
                    log("更新下载：${file?.name ?: err}")
                    renderUpdate()
                    updateButtons()
                }
            },
        )
    }

    private fun installUpdate() {
        val apk = updateApk ?: return
        if (!apk.exists()) {
            updateMsg = getString(R.string.update_failed, "安装包已不存在")
            renderUpdate()
            return
        }
        if (!UpdateInstaller.canInstall(this)) {
            updateMsg = getString(R.string.update_install_denied)
            renderUpdate()
            UpdateInstaller.openInstallSettings(this)
            return
        }
        try {
            UpdateInstaller.install(this, apk)
            updateMsg = getString(R.string.update_install_started)
        } catch (e: Exception) {
            updateMsg = getString(R.string.update_failed, e.message ?: e.javaClass.simpleName)
        }
        renderUpdate()
    }

    // ---------------------------------------------------------------- 偏好 & 绑定

    private fun bindViews() {
        etHost = findViewById(R.id.etHost)
        etPort = findViewById(R.id.etPort)
        btnScan = findViewById(R.id.btnScan)
        btnScan.setOnClickListener { startScan() }
        swConnect = findViewById(R.id.swConnect)
        navBottom = findViewById(R.id.navBottom)
        btnHealth = findViewById(R.id.btnHealth)
        btnRediscover = findViewById(R.id.btnRediscover)
        btnReconnect = findViewById(R.id.btnReconnect)
        tvStatus = findViewById(R.id.tvStatus)
        tvHealth = findViewById(R.id.tvHealth)
        tvErr = findViewById(R.id.tvErr)
        tvNet = findViewById(R.id.tvNet)
        tvAudio = findViewById(R.id.tvAudio)
        pbUpLevel = findViewById(R.id.pbUpLevel)
        pbDownLevel = findViewById(R.id.pbDownLevel)
        pbBuffer = findViewById(R.id.pbBuffer)
        tvUpLevel = findViewById(R.id.tvUpLevel)
        tvDownLevel = findViewById(R.id.tvDownLevel)
        tvBuffer = findViewById(R.id.tvBuffer)
        btnCopyDiag = findViewById(R.id.btnCopyDiag)
        swMic = findViewById(R.id.swMic)
        spMicSource = findViewById(R.id.spMicSource)
        tvHeroStatus = findViewById(R.id.tvHeroStatus)
        swSpeaker = findViewById(R.id.swSpeaker)
        slRing = findViewById(R.id.slRing)
        tvRing = findViewById(R.id.tvRing)
        swRemote = findViewById(R.id.swRemote)
        swRemoteText = findViewById(R.id.swRemoteText)
        swRemoteKeys = findViewById(R.id.swRemoteKeys)
        etText = findViewById(R.id.etText)
        btnSend = findViewById(R.id.btnSend)
        btnClear = findViewById(R.id.btnClear)
        tvBattery = findViewById(R.id.tvBattery)
        tvKeepalive = findViewById(R.id.tvKeepalive)
        tvNotifPerm = findViewById(R.id.tvNotifPerm)
        btnNotifSettings = findViewById(R.id.btnNotifSettings)
        btnBattery = findViewById(R.id.btnBattery)
        tvUpdate = findViewById(R.id.tvUpdate)
        btnCheckUpdate = findViewById(R.id.btnCheckUpdate)
        btnDownload = findViewById(R.id.btnDownload)
        btnInstall = findViewById(R.id.btnInstall)
        llDiag = findViewById(R.id.llDiag)
        tvLog = findViewById(R.id.tvLog)
    }

    private fun restorePrefs() {
        val p = getSharedPreferences(PREFS, MODE_PRIVATE)
        host = p.getString("host", DEFAULT_HOST) ?: DEFAULT_HOST
        port = p.getInt("port", Proto.PORT)
        // 两个框各自回填：端口单独一格，用户随时能改（不再拼成一个字符串）
        etHost.setText(host)
        etPort.setText(port.toString())
        swMic.isChecked = p.getBoolean("mic", false)
        swSpeaker.isChecked = p.getBoolean("speaker", false)
        swRemote.isChecked = p.getBoolean("remote", false)
        swRemoteText.isChecked = p.getBoolean("remoteText", true)
        swRemoteKeys.isChecked = p.getBoolean("remoteKeys", true)
        micSourceId = p.getInt("micSource", MicSources.ALL[0].id)
        fillMicSources()
    }

    /** 麦克风 API 下拉：按 [MicSources.ALL] 的顺序（常用在上）填充，选中上次的选择。 */
    private fun fillMicSources() {
        val labels = MicSources.ALL.map { getString(it.labelRes) }
        spMicSource.setAdapter(
            ArrayAdapter(this, android.R.layout.simple_list_item_1, labels),
        )
        spMicSource.setText(MicSources.label(this, micSourceId), false)
    }

    private fun savePrefs() {
        // 存归一后的 host + 端口（别把 `http://…/` 整串存进去；端口不是默认值时单独存）
        if (etHost.text.toString().trim().isNotEmpty()) readTarget()
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString("host", host)
            .putInt("port", port)
            .putBoolean("mic", swMic.isChecked)
            .putBoolean("speaker", swSpeaker.isChecked)
            .putBoolean("remote", swRemote.isChecked)
            .putBoolean("remoteText", swRemoteText.isChecked)
            .putBoolean("remoteKeys", swRemoteKeys.isChecked)
            .putInt("micSource", micSourceId)
            .apply()
    }
}