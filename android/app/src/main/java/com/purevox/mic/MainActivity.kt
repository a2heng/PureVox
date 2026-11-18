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
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import android.widget.Toast
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatDelegate
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.core.os.LocaleListCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.purevox.mic.audio.MicSources
import com.purevox.mic.audio.MicUplink
import com.purevox.mic.audio.OpusSupport
import com.purevox.mic.audio.SpeakerDownlink
import com.purevox.mic.control.Control
import com.purevox.mic.diag.DiagJson
import com.purevox.mic.diag.RtStats
import com.purevox.mic.net.Discovery
import com.purevox.mic.net.HealthProbe
import com.purevox.mic.net.WsClient
import com.purevox.mic.service.CaptureService
import com.purevox.mic.ui.PureVoxApp
import com.purevox.mic.update.UpdateChecker
import com.purevox.mic.update.UpdateDownloader
import com.purevox.mic.update.UpdateInstaller
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

/**
 * 唯一的界面与编排者（单 Activity；界面是 Compose + COUI-Kit）。
 *
 * 它负责把三条独立能力接上电脑端的同一个 WebSocket：
 * 1. 手机麦克风 → 电脑（[MicUplink]）
 * 2. 电脑音频 → 手机扬声器（[SpeakerDownlink]）
 * 3. 远程输入：IME 已组合文本（`text`）+ 手机实体键（`key`）。电脑端**默认关闭**，
 *    没开开关时电脑会回 `err`，本界面把 `err` 原文显示出来，不静默丢弃。
 *
 * 界面全部在 `ui/MainScreen.kt`，本类只留编排与状态：一页式分组（头图 / 连接 / 功能 /
 * 诊断），设置收进头图右上角按钮的居中浮层 —— 保活 / 更新 / 关于不常看，不占一整页。
 *
 * **界面状态是 Compose state**（`micOn` / `spkOn` / `ringMs` / `hostInput` …），
 * 不再持有任何 View 引用：业务逻辑与视图层只通过这里的字段 + 方法通信。
 *
 * 纪律：
 * - **收到 `ready` 之前不推流、不播放**（[applyState] 里 `ready` 是必要条件）。
 * - 状态区（在设置浮层里，常显）分两块：**网络**（RTT 窗口 / 抖动 / 真实速率 / 结论）
 *   与**音频**（上/下行电平条、缓冲水位条、欠载 / 丢包 / 解码掩盖），再加几行累计读数；
 *   2 Hz 刷新。失败项显示「不可用 + 原因」，实时量没有样本时写「采样中…」——
 *   **绝不显示伪造的 0**。整块诊断可一键复制到剪贴板（[copyDiag]）。
 * - 所有 OkHttp 回调都切回主线程再动 UI。
 *
 * 会话保活：只要「已连接」或「正在推流」就由 [CaptureService] 常驻一条前台通知
 * （[syncSessionService]），通知 / 磁贴 / 小组件的按钮经 [handleControl] 回到这里处理
 * —— 服务不认识协议，编排只此一处。
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "PureVoxUI"
        private const val PREFS = "purevox"
        /** 没有缓存地址时的空值（**不再预填一个假 IP**，那会让人以为已经配对过）。 */
        private const val DEFAULT_HOST = ""
        private const val MAX_LOG_LINES = 80
        private const val STATUS_INTERVAL_MS = 500L

        /**
         * 当前界面实例（通知 / 磁贴 / 小组件按钮的落点）。
         *
         * 控制面发的是广播（[ControlReceiver]），不是直接持有界面：界面不在时接收端会
         * 明确提示「先打开应用」，而不是静默丢掉一次点击。
         */
        @Volatile
        var current: MainActivity? = null
            private set

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

    }

    private val ui = Handler(Looper.getMainLooper())

    @Volatile
    private var ws: WsClient? = null
    private var uplink: MicUplink? = null
    private var downlink: SpeakerDownlink? = null

    private var host = DEFAULT_HOST
    /** 当前**已建立/正在建立**的这次连接的真实目标；成功后才写进偏好。 */
    private var sessionHost = ""
    private var sessionPort = Proto.PORT

    /**
     * 电脑端的端口。**独立于 host 存**，因为它是用户可改的（NETWORK.md §4.1.1），
     * 而且改的是**电脑端**那个设置 —— 手机这边只能从地址里学（mDNS / 二维码），
     * 学到的必须记住，否则下次启动退回 59123 就「莫名连不上」。
     */
    private var port = Proto.PORT
    /** 选中的麦克风采集 API（`MediaRecorder.AudioSource`，见 [MicSources]）。 */
    internal var micSourceId by mutableIntStateOf(MicSources.ALL[0].id)
    private var opusDetail = ""

    // 界面状态（Compose）：只保留「要显示 / 要交互」的值，不再有 View 引用
    internal var hostInput by mutableStateOf(DEFAULT_HOST)
    internal var portInput by mutableStateOf(Proto.PORT.toString())
    /** 连接开关的位置 = 连没连上（与 `ws != null || connecting` 同步，见 [updateButtons]）。 */
    internal var connectOn by mutableStateOf(false)
    internal var micOn by mutableStateOf(false)
    internal var spkOn by mutableStateOf(false)
    /** 抖动缓冲深度（ms）；Compose 的 Slider 直接读写它。 */
    internal var ringMs by mutableIntStateOf(50)
    internal var settingsOpen by mutableStateOf(false)
    /** 2 Hz 状态刷新计数：Compose 根读它，普通字段（日志等）也能跟着重组。 */
    internal var uiTick by mutableIntStateOf(0)
    /** 日志追加计数（每次 [log] +1）；日志行本身是普通集合，不逐条生成 state。 */
    internal var logVersion by mutableIntStateOf(0)

    internal var connState by mutableStateOf("")
    internal var heroTargetText by mutableStateOf("")
    internal var ready by mutableStateOf(false)
    internal var connecting by mutableStateOf(false)
    internal var lastErr by mutableStateOf("")
    internal var healthText by mutableStateOf("")

    // 设置层里的读数：算成字符串/百分比存下来，Compose 直接渲染（设置层关着也照样更新）
    internal var keepaliveText by mutableStateOf("")
    internal var notifPermText by mutableStateOf("")
    internal var batteryText by mutableStateOf("")
    internal var updateText by mutableStateOf("")
    internal var updateBusy by mutableStateOf(false)
    internal var updateRelease by mutableStateOf<UpdateChecker.ReleaseInfo?>(null)
    internal var updateApk by mutableStateOf<File?>(null)
    internal var diagNetText by mutableStateOf("")
    internal var diagAudioText by mutableStateOf("")
    internal var diagStatusText by mutableStateOf("")
    internal var upPercent by mutableIntStateOf(0)
    internal var downPercent by mutableIntStateOf(0)
    internal var bufferPercent by mutableIntStateOf(0)
    internal var upValueText by mutableStateOf("")
    internal var downValueText by mutableStateOf("")
    internal var bufferValueText by mutableStateOf("")

    /**
     * 诊断读数的时间序列（RTT 窗口 / 速率 / 结论）。**连接重建时换一个新的**，
     * 这样上一次会话的 RTT 与速率不会渗进新会话的读数里（见 [connect]）。
     * 结论文字要按当前语言取，所以它拿着 Activity 的 resources。
     */
    private var rt = RtStats(this)

    @Volatile
    private var speakerOn = false

    @Volatile
    private var applyingState = false

    /** 前台会话服务当前是否在跑 / 是否含麦克风类型（避免每次对账都重发通知）。 */
    private var svcRunning = false
    private var svcMic = false
    /** 通知服务当前是否真的有音频流（决定 WakeLock）。 */
    private var svcAudio = false
    /** 通知里扬声器 / 连接状态图标的最近一次下发值（变了才重发通知）。 */
    private var svcSpk = false
    private var svcConn = Control.CONN_OFFLINE

    /**
     * 界面是否在前台（onStart→onStop）。
     *
     * 用途只有一个：**界面开着就把通知挂着** —— 前台服务的第一次起必须在界面在前台时完成，
     * 后台里被拒的话整个会话都在裸奔（系统随时掐进程）。
     */
    private var visible = false

    /** 上次「补挂通知」的尝试时刻（`elapsedRealtime`），避免每 500 ms 重发一次通知。 */
    private var lastNotifTry = 0L

    /** 更新状态的提示行（版本行 + 检查/下载进度），与 [updateText] 拼接后上屏。 */
    private var updateMsg = ""

    /** 局域网自动发现。发现到就自动填地址并连上；找不到才让用户手动填。 */
    private var discovery: Discovery? = null
    /** 已通过 mDNS 发现到电脑（此时地址框只是「看一眼」，不是主路径）。 */
    private var found = false
    /** 发现到的协议版本；与本客户端不一致时明确告诉用户「客户端太旧」。 */
    private var foundProto: Int? = null
    /** 用户扫码 / 手填 / 手动点连接选定的目标：mDNS 不得再覆盖它。 */
    private var manualTarget = false
    /** 等本地网络权限回来后是否要自动继续连接。 */
    private var pendingLocalNetConnect = false

    /** 日志行（最近 [MAX_LOG_LINES] 条）。视图不在时也要能复制 / 落盘。 */
    private val logLines = ArrayDeque<String>()

    /**
     * 控制面（通知 / 快捷设置磁贴 / 桌面小组件）的动作入口。
     *
     * 三个出口都是**切换 / 刷新**，与界面上那个开关完全等价（同一个入口）：
     * - [Control.ACTION_MIC] / [Control.ACTION_SPK] 切换开关；
     * - [Control.ACTION_RECONNECT] 先拆掉已打开的流再重连。
     */
    internal fun handleControl(action: String) {
        when (action) {
            Control.ACTION_MIC -> {
                log("控制面：麦克风 → ${if (micOn) "关" else "开"}")
                setMic(!micOn)
            }
            Control.ACTION_SPK -> {
                log("控制面：扬声器 → ${if (spkOn) "关" else "开"}")
                setSpeaker(!spkOn)
            }
            Control.ACTION_RECONNECT -> {
                log("控制面：重新连接（先拆掉已打开的流再重连）")
                reconnect()
            }
        }
    }

    // ---------------------------------------------------------------- 界面动作
    //
    // Compose 只调这些方法，自己不碰网络 / 音频 / 服务；所有状态变化在 MainActivity
    // 这一层收口，通知栏 / 磁贴 / 小组件走的也是同一批入口。

    /** 连接开关：打开 = 用户明确选定目标（mDNS 不再覆盖），关闭 = 主动断开。 */
    internal fun setConnect(checked: Boolean) {
        if (checked) {
            if (ws == null && !connecting) {
                manualTarget = true
                connect()
            }
        } else if (ws != null || connecting) {
            disconnect()
        }
    }

    internal fun setMic(on: Boolean) {
        if (micOn == on) return
        micOn = on
        applyState()
        updateStatus()
    }

    internal fun setSpeaker(on: Boolean) {
        if (spkOn == on) return
        spkOn = on
        applyState()
        updateStatus()
    }

    /**
     * 抖动缓冲：**拖一下就生效**（不重建会话、不重连），并同步给电脑。
     *
     * 只在整数毫秒真的变了才发：Compose 的 Slider 在拖动中每帧回调，
     * 不去重的话每个像素都会往电脑发一条 `settarget`。
     */
    internal fun setRingTarget(ms: Int) {
        val v = ms.coerceIn(10, 200)
        if (v == ringMs) return
        ringMs = v
        downlink?.setTargetMs(v.toDouble())
        ws?.sendText(Proto.setTarget(v))
        log("抖动缓冲 → $v ms")
    }

    /** 换麦克风采集 API：立即记住；**正在推流时立即换源**，关闭状态下次开启生效。 */
    internal fun selectMicSource(id: Int) {
        if (id == micSourceId) return
        micSourceId = id
        savePrefs()
        log("麦克风 API：${MicSources.label(this, id)}")
        if (micOn && uplink != null) {
            uplink?.stop()
            uplink = null
            applyState()
        }
    }

    /** 打开设置浮层：先刷新一遍读数，打开即是最新（之后 2 Hz 的 ticker 继续刷）。 */
    internal fun openSettings() {
        updateKeepalive()
        refreshBattery()
        renderUpdate()
        updateStatus()
        settingsOpen = true
    }

    internal fun closeSettings() {
        settingsOpen = false
    }

    /** 当前 per-app 语言（`""` = 跟随系统）。 */
    internal fun currentLanguageTag(): String =
        AppCompatDelegate.getApplicationLocales().toLanguageTags().let {
            when {
                it.startsWith("zh") -> "zh"
                it.startsWith("en") -> "en"
                else -> ""
            }
        }

    /**
     * 切换界面语言。AppCompat 会重建 Activity，所以先把浮层关掉，
     * 免得旧 Activity 的窗口留到重建之后。
     */
    internal fun setLanguage(tag: String) {
        settingsOpen = false
        AppCompatDelegate.setApplicationLocales(
            if (tag.isEmpty()) {
                LocaleListCompat.getEmptyLocaleList()
            } else {
                LocaleListCompat.forLanguageTags(tag)
            },
        )
    }

    /** 日志快照（设置层每次重组读一次）。 */
    internal fun logSnapshot(): String = logLines.joinToString("\n")

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
            if (pendingLocalNetConnect) {
                pendingLocalNetConnect = false
                if (hasLocalNetworkPermission()) connect()
                else log(getString(R.string.err_local_net_permission))
            }
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
        current = this
        applyEdgeToEdge()

        // 连接状态 / 自检结论的初始值按当前语言取（切换语言会重建 Activity，走这里重新取）
        connState = getString(R.string.msg_not_connected)
        healthText = getString(R.string.health_none)

        // 一行说明「现在在用什么 + 为什么不碰系统 codec」，不是探测结果而是编解码器的身份
        opusDetail = OpusSupport.describe(this)

        // 偏好要在第一次组合之前读好：Compose 的首帧就直接显示上次的目标与开关
        restorePrefs()
        setContent { PureVoxApp(this) }

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
        // 有上次地址就直接连，**不先启动发现**：mDNS 缓存里可能还有旧端口记录，
        // 在连接过程中把 59662 改回 59123。发现只在「没有缓存地址」或「缓存连接失败」时兜底。
        if (host.isNotEmpty()) {
            log(getString(R.string.msg_reuse_cached, host, port))
            connect()
        } else {
            startDiscovery()
        }

        requestPermissions()
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
        if (current === this) current = null
        ui.removeCallbacks(ticker)
        // 发现监听必须停：`NsdManager` 的监听会一直占着（耗电），且下次启动重复注册会抛
        // `IllegalArgumentException`
        discovery?.stop()
        discovery = null
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
        val raw = hostInput.trim()
        if (raw.isEmpty()) return ""
        val norm = normalizeTarget(raw)
        val explicitRaw = explicitPortRaw(norm)
        val explicit = explicitRaw?.toIntOrNull()?.takeIf { it in 1..65535 }
        if (explicitRaw != null && explicit == null) {
            showErr(getString(R.string.msg_port_invalid, explicitRaw))
        }
        val typed = portInput.trim().toIntOrNull()?.takeIf { it in 1..65535 }
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
        hostInput = h
        portInput = p.toString()
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
    internal fun startScan() {
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
        manualTarget = true
        // 扫码是用户明确选定目标：停掉自动发现，别让缓存里的旧端口记录再改输入框。
        discovery?.stop()
        discovery = null
        hostInput = h
        portInput = (explicit ?: p).toString()
        // 扫码是用户明确选择：立即保存，不等连接成功（连接失败也保留这次结果）。
        saveTarget(h, explicit ?: p)
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
    internal fun rediscover() {
        manualTarget = false
        // 「重新发现」也是复位：当前会话要么已经断了，要么换网了，旧目标没有继续占着的理由。
        resetSession()
        startDiscovery()
        connState = getString(R.string.msg_discovering)
        updateStatus()
        log(getString(R.string.msg_discovering))
        applyState()
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
                    // **没有端口就忽略**：NSD 偶尔会先给一条 port=0 的未完成记录。
                    // 不能拿它回退到 59123 —— 那正是「发现到的端口和电脑实际端口不一致」的来源。
                    if (p <= 0) {
                        log("mDNS 服务 $name 还没给出端口，忽略")
                        return@runOnUiThread
                    }
                    // 扫码 / 手填 / 手动连接选定的目标优先：mDNS 里可能还有旧端口记录，
                    // 不能在用户已经选好目标时把端口改回 59123。点「重新发现」会清掉这个标记。
                    if (manualTarget) {
                        log("已选定 ${host}:${port}，忽略 mDNS 发现 $h:$p")
                        return@runOnUiThread
                    }
                    // **已连上就不要再让后来的 mDNS 记录改输入框**：同一台电脑可能还有
                    // 旧服务记录 / 多张网卡，等一会儿才到的 59123 会把已经连上的 59662
                    // 覆盖成默认端口，用户看到的就是「连的是 59662，重启后框里却是 59123」。
                    if (ws != null && ready) {
                        if (h != host || p != port) {
                            log("发现另一台电脑 $h:$p，保持当前连接 $host:$port")
                        }
                        return@runOnUiThread
                    }
                    val sameTarget = h == host && p == port
                    if (connecting && sameTarget) return@runOnUiThread
                    // 缓存地址正在连但发现到的端口/地址不一样：取消旧尝试，立刻连 mDNS 给的
                    // 真实端口。否则旧尝试会先超时 8 s，用户看到的就是「明明发现了却连不上」。
                    if (connecting) disconnect()
                    // 端口必须一起带上：电脑端端口可编辑，mDNS 报的是真实端口。
                    hostInput = h
                    portInput = (if (p > 0) p else Proto.PORT).toString()
                    // 发现只是「帮你填好」，不锁输入框 / 扫描按钮。
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

    /**
     * 重新连接（界面按钮 / 通知栏刷新按钮）。
     *
     * **语义是复位，不是「再发一次 connect」**：先把 WebSocket 和已经打开的上下行全部拆掉，
     * 再按当前目标重连。旧实现只在 `ws` 上 `disconnect()`、连 `ready/connecting` 都不清，
     * 而且 `found == false` 时直接转去 `rediscover()` —— 那时界面已经显示「查找中」，旧会话
     * 却还挂着；mDNS 回调看到 `ws != null && ready` 会拒绝接管，状态就**永远卡在「查找」**，
     * 用户再点一次（此时 `found` 已置 true）才动。这就是「偶尔死掉、再点就好」的根因。
     *
     * 没有目标（从没配对过、界面上也没地址）才退回自动发现 —— 那是唯一需要 mDNS 的情况。
     */
    internal fun reconnect() {
        val target = readTarget()
        resetSession()
        if (target.isEmpty()) {
            rediscover()
            return
        }
        connect()
    }

    /**
     * 拆掉当前会话：WebSocket + 麦克风上行 + 扬声器下行，并清掉 `ready` / `connecting`。
     *
     * 不动前台通知服务：下一步通常立刻 `connect()`，`connecting = true` 会让保活继续挂着；
     * 先停服务再在后台重启会被 Android 12+ 拒绝（整个会话就裸奔了）。
     */
    private fun resetSession() {
        val c = ws
        ws = null
        connecting = false
        ready = false
        c?.disconnect()
        uplink?.stop()
        uplink = null
        downlink?.stop()
        downlink = null
    }

    // ---------------------------------------------------------------- 连接

    private fun connect() {
        if (!ensureLocalNetworkPermission()) return
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
        sessionHost = h
        sessionPort = p
        connecting = true
        ready = false
        rt = RtStats(this)
        lastErr = ""
        connState = getString(R.string.state_connecting)
        val client = WsClient(h, p, wsListener, this)
        ws = client
        client.connect()
        // 立刻对账一次：停掉旧流（如果还有），并把通知切到「连接中」。
        applyState()
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
        rt = RtStats(this)
        connState = getString(R.string.state_disconnected)
        applyState()
        updateButtons()
        updateStatus()
        log(reason)
    }

    private val wsListener = object : WsClient.Listener {
        override fun onOpen(client: WsClient) {
            ui.post {
                if (client !== ws) return@post // 旧连接的迟到回调，丢掉
                connState = getString(R.string.state_connected_wait_ready)
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
                    is ServerMsg.Err -> showErr(getString(R.string.msg_server, msg.msg))
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

        override fun onPong(client: WsClient, id: Long, rttMs: Double) {
            // RTT 统计与界面刷新都在主线程做：RtStats 内部不加锁（调用方保证单线程）
            ui.post {
                if (client !== ws) return@post
                rt.onRtt(rttMs)
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
        // 连接成功才把目标写进偏好：手填/发现/扫码三条路都走这里，失败不落盘。
        commitTarget(sessionHost, sessionPort)
        connState = getString(R.string.state_ready)
        if (msg.proto != Proto.VERSION) {
            showErr(getString(R.string.err_proto, msg.proto, Proto.VERSION))
        }
        if (msg.codec.isNotEmpty() && msg.codec != Proto.CODEC) {
            showErr(getString(R.string.err_codec, msg.codec, Proto.CODEC))
        }
        if (msg.rate > 0 && msg.rate != Proto.SAMPLE_RATE) {
            showErr(getString(R.string.err_rate, msg.rate, Proto.SAMPLE_RATE))
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

    internal fun runHealthCheck() {
        val target = readTarget()
        if (target.isNotEmpty()) host = target
        healthText = getString(R.string.health_checking)
        updateStatus()
        HealthProbe.check(this, host, port) { ok, text ->
            ui.post {
                healthText = if (ok) text else getString(R.string.health_unavailable, text)
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
        val wantMic = micOn && ready && hasMicPermission()
        if (wantMic && uplink == null) {
            if (!startUplink()) {
                micOn = false
                updateButtons()
            }
        } else if (!wantMic && uplink != null) {
            uplink?.stop()
            uplink = null
            log("麦克风上行已停止")
        }

        speakerOn = spkOn && ready
        if (speakerOn && downlink == null) {
            // 单向延迟要用电脑算出的时钟偏移（`sync`/`offset`，还没测出来时返回 null）
            val d = SpeakerDownlink(this, clockOffsetUs = { ws?.clockOffsetUs })
            if (d.start()) {
                downlink = d
                ws?.sendText(Proto.sub())
                log("电脑音频播放已开启（解码 ${d.decoderName}）")
            } else {
                speakerOn = false
                spkOn = false
                showErr(d.lastError ?: getString(R.string.err_downlink_start))
            }
        } else if (!speakerOn && downlink != null) {
            downlink?.stop()
            downlink = null
            // 有 unsub 就退订，别让电脑继续往没人取的队列里推（水位顶格、白占 CPU）
            ws?.sendText(Proto.unsub())
            log("电脑音频播放已停止（已退订）")
        }

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
        val spk = speakerOn
        val conn = notificationConn()
        // 同一份状态发给磁贴 / 小组件（与通知图标同源）；只在变化时刷新。
        Control.publish(this, conn, mic, spk)
        val keep = ready || mic || connecting || visible
        if (!keep) {
            if (svcRunning) {
                CaptureService.stop(this)
                svcRunning = false
                svcMic = false
                svcAudio = false
                svcSpk = false
                svcConn = Control.CONN_OFFLINE
            }
            return
        }
        // 图标状态（连接 / 麦克风 / 扬声器）变一个就要重发：只比 mic/audio 会让
        // 「连上了」「开了扬声器」停在旧图标上（用户看到的通知与实际状态不一致）。
        if (!force && svcRunning && svcMic == mic && svcAudio == audio &&
            svcSpk == spk && svcConn == conn
        ) {
            return
        }
        svcRunning = true
        svcMic = mic
        svcAudio = audio
        svcSpk = spk
        svcConn = conn
        CaptureService.update(this, mic, audio, spk = spk, conn = conn)
        updateKeepalive()
    }

    /** 当前连接状态 → 通知 / 磁贴 / 小组件共用的图标状态。 */
    private fun notificationConn(): Int = when {
        ready -> Control.CONN_ONLINE
        connecting -> Control.CONN_CONNECTING
        else -> Control.CONN_OFFLINE
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
        // Android 13+ 允许用户划掉前台服务通知（NO_CLEAR 挡不住），而本应用的保活
        // 恰恰依赖「通知在」。所以这里不只看 CaptureService.foreground（那只表示
        // startForeground 成功过），还要看通知**此刻是否真的还在通知栏**：
        // 被划掉 / 被系统撤掉 → 5 秒内补挂。权限被关时不反复重试（设置卡片会说原因）。
        val nm = getSystemService(NotificationManager::class.java)
        val shown = CaptureService.foreground &&
            (!nm.areNotificationsEnabled() ||
                nm.activeNotifications.any { it.id == CaptureService.NOTIFICATION_ID })
        if (want && !shown) {
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
        keepaliveText = s
        notifPermText = getString(notifPermissionText())
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
    internal fun openNotifSettings() {
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
            mic = true,
            audio = true,
            spk = speakerOn,
            conn = notificationConn(),
        )
        svcRunning = true
        svcMic = true
        svcAudio = true
        svcSpk = speakerOn
        svcConn = notificationConn()
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
            "麦克风上行已启动：${u.sourceName} · 采集 ${OpusSupport.pcmName(this, u.recordEncoding)} " +
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

    /**
     * edge-to-edge：头图铺到状态栏后面（品牌渐变就是状态栏底色），
     * 页面底部让出导航栏；输入法弹出时底部让位。
     *
     * inset 由 Compose 的 `WindowInsets` 消费（`ui/MainScreen.kt`）—— 这里只负责
     * 让内容画进系统栏区域，并把两条系统栏的图标颜色定成与背景对比够的样子：
     * 状态栏固定浅色（头图是深蓝），导航栏跟随日夜。
     */
    private fun applyEdgeToEdge() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            isAppearanceLightStatusBars = false
            isAppearanceLightNavigationBars = !isNightMode()
        }
        window.isNavigationBarContrastEnforced = false
    }

    private fun isNightMode(): Boolean =
        (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES

    // ---------------------------------------------------------------- 权限

    private fun hasMicPermission(): Boolean =
        ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    /** Android 17（API 37）起，访问局域网电脑需要这个运行时权限；之前的系统恒为 true。 */
    private fun hasLocalNetworkPermission(): Boolean =
        Build.VERSION.SDK_INT < 37 ||
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.ACCESS_LOCAL_NETWORK,
            ) == PackageManager.PERMISSION_GRANTED

    /**
     * 连接前确认本地网络权限。没有就弹一次系统权限框，授权后自动继续刚才的连接。
     * 没有这个权限时 TCP 会一直超时（看起来像防火墙/网线问题），mDNS 却可能仍然能发现电脑。
     */
    private fun ensureLocalNetworkPermission(): Boolean {
        if (hasLocalNetworkPermission()) return true
        pendingLocalNetConnect = true
        connState = getString(R.string.state_waiting_local_net)
        updateStatus()
        log(getString(R.string.state_waiting_local_net))
        permissionLauncher.launch(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK))
        return false
    }

    private fun requestPermissions() {
        val want = ArrayList<String>()
        if (!hasMicPermission()) want.add(Manifest.permission.RECORD_AUDIO)
        if (!hasLocalNetworkPermission()) {
            want.add(Manifest.permission.ACCESS_LOCAL_NETWORK)
        }
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

    /**
     * 开关位置的对账：连着（或正在连）就在「开」的位置，其余按下发给 Compose 的
     * `enabled` 规则（连接中锁连接开关、未 ready 锁两个音频开关）。
     */
    private fun updateButtons() {
        connectOn = ws != null || connecting
    }

    /**
     * 诊断刷新：网络统计 + 音频电平/缓冲条 + 固定读数行（行数固定，数值变长也不跳）。
     *
     * 读数**先算成字符串 / 百分比**再上屏：设置浮层关着时 Compose 不组合诊断区，
     * 但复制诊断与落盘 JSON 仍要拿到这一拍的读数。
     */
    private fun updateStatus() {
        val c = ws
        val enc = uplink
        val dec = downlink
        heroTargetText = when {
            host.isEmpty() -> getString(R.string.hero_target_auto)
            port == Proto.PORT -> host
            else -> "$host:$port"
        }
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
        diagNetText = buildString {
            if (net.rttSamples == 0) {
                append(row(getString(R.string.status_rtt), getString(R.string.diag_no_pong)))
                append(row(getString(R.string.diag_jitter), "—"))
            } else {
                append(
                    row(
                        getString(R.string.status_rtt),
                        getString(
                            R.string.diag_rtt_row,
                            ms(net.rttLast),
                            ms(net.rttAvg),
                            ms(net.rttMax),
                            net.rttSamples,
                        ),
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
        upPercent = rt.levelPercent(upLevel)
        downPercent = rt.levelPercent(downPeak)

        val bufferedMs = dec?.levelMs?.toInt() ?: 0
        bufferPercent = (bufferedMs * 100 / BUFFER_FULL_MS).coerceIn(0, 100)

        val a = rt.audioVerdict(net, dec != null)
        diagAudioText = buildString {
            append(
                row(
                    getString(R.string.diag_chain),
                    if (enc == null && dec == null) {
                        getString(R.string.diag_both_off)
                    } else {
                        (
                            enc?.let {
                                getString(
                                    R.string.diag_enc_capture,
                                    it.encoderName,
                                    OpusSupport.pcmName(this@MainActivity, it.recordEncoding),
                                )
                            } ?: getString(R.string.diag_no_uplink)
                            ) + " → " + (
                            dec?.let { getString(R.string.diag_decode, it.decoderName) }
                                ?: getString(R.string.diag_no_sub)
                            )
                    },
                ),
            )
            if (enc != null) {
                append(
                    row(
                        getString(R.string.status_up),
                        getString(
                            R.string.diag_up_counts,
                            enc.encoderPackets,
                            enc.encoderDroppedSamples,
                        ),
                    ),
                )
            }
            append(
                row(
                    getString(R.string.status_play),
                    if (dec == null) {
                        getString(R.string.diag_off)
                    } else {
                        getString(
                            R.string.diag_play_row,
                            dec.underruns,
                            fmt1(net.underrunPps),
                            dec.dropped,
                            fmt1(net.droppedPps),
                            dec.concealed,
                            fmt1(net.concealedPps),
                            dec.deviceUnderruns,
                        ) + (dec.lastError?.let { " · $it" } ?: "")
                    },
                ),
            )
            if (dec != null) {
                append(
                    row(
                        getString(R.string.diag_latency),
                        getString(
                            R.string.diag_latency_row,
                            bufferedMs,
                            dec.targetMs.toInt(),
                            if (dec.oneWayMs.isNaN()) {
                                getString(R.string.diag_measuring)
                            } else {
                                fmt1(dec.oneWayMs) + " ms"
                            },
                            dec.trackBufferMs,
                            if (dec.clockPpm.isNaN()) {
                                getString(R.string.diag_sampling)
                            } else {
                                "%+.0f ppm".format(dec.clockPpm)
                            },
                        ),
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
        diagStatusText = line.toString()

        // ---- 上屏值（Compose 直接渲染；设置浮层关着也照样更新）
        upValueText = if (enc == null) getString(R.string.diag_off) else dbfs(upLevel)
        downValueText = if (dec == null) getString(R.string.diag_off) else dbfs(downPeak)
        bufferValueText = if (dec == null) getString(R.string.diag_off) else "$bufferedMs ms"

        // ---- 结构化诊断（DEBUG.md §1.4）：同一份读数同时落盘 + 打 logcat，
        // 脚本/AI 不必截图、不必点界面（`adb logcat -s PureVoxDiag:I` / `run-as ... cat files/diag.json`）
        val diagIssue = DiagJson.publish(this, diagJson(net, q, a), System.currentTimeMillis())
        if (diagIssue != null) log(diagIssue)

        // Compose 根读它：普通字段（日志快照等）也跟着这一拍刷新
        uiTick++
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
                    put("capture_format", OpusSupport.pcmName(this@MainActivity, enc.recordEncoding))
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
                for (line in logLines.takeLast(40)) arr.put(line)
            },
        )
        return root.toString()
    }

    /**
     * 应用环深度（来自 `ready` / `target`）：环、滑块、读数一起改。
     *
     * 关键是**不走 [setRingTarget]**：电脑广播回来的目标只改本地状态，不能再发回电脑，
     * 否则「电脑改 → 手机回声 settarget → 电脑再广播」会自激。
     */
    private fun applyRingTarget(ms: Int) {
        if (ms <= 0) return
        downlink?.setTargetMs(ms.toDouble())
        ringMs = ms.coerceIn(10, 200)
    }

    /**
     * 上/下行速率。没连上就说「未连接」、还没到第一次采样就说「采样中」——
     * **不拿 0 冒充**（0 包/s 与「根本没连上」是两件事）。
     */
    private fun rateText(pps: Double, kbps: Double, total: Long?): String = when {
        total == null -> getString(R.string.rate_not_connected)
        pps.isNaN() -> getString(R.string.rate_sampling)
        else -> getString(R.string.rate_row, fmt1(pps), fmt1(kbps), total)
    }

    /** 电平（线性峰值 0..1）→ dBFS 读数；太小直接报「静音」，免得出现 -inf。 */
    private fun dbfs(level: Float): String {
        if (level <= 1e-5f) return getString(R.string.level_silent)
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
            logLines.addLast("[$t] $msg")
            while (logLines.size > MAX_LOG_LINES) logLines.removeFirst()
            // 日志先记进内存，诊断区打开时再铺上。
            // **不要**在这里滚动日志：日志区在一屏可滚的内容里，每来一行就把正文顶一下
            // 看起来就是「整块在闪」。保留最近 MAX_LOG_LINES 行、不动视口即可。
            logVersion++
        }
    }

    // ---------------------------------------------------------------- 诊断区 / 保活 / 更新

    /**
     * 把整块诊断复制到剪贴板（网络 + 音频 + 读数 + 日志）。
     *
     * 存在的意义：手机上报问题最省事的方式就是复制一段文本，不必让人对着屏幕截图，
     * 也免得「日志只留在设备上、退一下就没了」。诊断在设置层里，层关着也能复制
     * （读数与日志都有内存副本，见 [diagNetText] / [logLines]）。
     */
    internal fun copyDiag() {
        val text = buildString {
            append(getString(R.string.diag_net)).append('\n')
            append(diagNetText)
            append('\n').append(getString(R.string.diag_audio)).append('\n')
            append(diagAudioText)
            append('\n')
            append(diagStatusText)
            append(getString(R.string.status_health)).append(healthText).append('\n')
            append(keepaliveText).append('\n')
            append(notifPermText).append('\n')
            append(batteryText).append('\n')
            if (lastErr.isNotEmpty()) append(lastErr).append('\n')
            append('\n').append(logLines.joinToString("\n")).append('\n')
        }
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText(getString(R.string.clip_diag_label), text))
        Toast.makeText(this, R.string.diag_copied, Toast.LENGTH_SHORT).show()
        log("诊断已复制到剪贴板（${text.length} 字）")
    }

    private fun isIgnoringBattery(): Boolean {
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        return pm.isIgnoringBatteryOptimizations(packageName)
    }

    private fun refreshBattery() {
        batteryText = getString(
            if (isIgnoringBattery()) R.string.battery_ignored else R.string.battery_not_ignored,
        )
    }

    /**
     * 请求「忽略电池优化」。用系统弹窗（`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`）
     * 而不是自己写白名单页 —— 是否放行由系统决定，我们只是发起请求。
     * 需要 `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` 权限（manifest 已声明）。
     */
    @SuppressLint("BatteryLife")
    internal fun requestIgnoreBattery() {
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
                showErr(
                    getString(
                        R.string.err_battery_settings,
                        e2.message ?: e2.javaClass.simpleName,
                    ),
                )
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
        updateText = if (updateMsg.isEmpty()) base else "$base\n$updateMsg"
    }

    internal fun checkUpdate() {
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

    internal fun downloadUpdate() {
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
                        updateMsg = getString(R.string.update_failed, err ?: getString(R.string.err_unknown))
                    }
                    log("更新下载：${file?.name ?: err}")
                    renderUpdate()
                    updateButtons()
                }
            },
        )
    }

    internal fun installUpdate() {
        val apk = updateApk ?: return
        if (!apk.exists()) {
            updateMsg = getString(R.string.update_failed, getString(R.string.err_apk_missing))
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

    // ---------------------------------------------------------------- 偏好

    /** 首帧之前读偏好：Compose 直接拿 `hostInput` / `portInput` / `micOn` / `spkOn` 渲染。 */
    private fun restorePrefs() {
        val p = getSharedPreferences(PREFS, MODE_PRIVATE)
        host = p.getString("host", DEFAULT_HOST) ?: DEFAULT_HOST
        port = p.getInt("port", Proto.PORT)
        // 两个框各自回填：端口单独一格，用户随时能改（不再拼成一个字符串）
        hostInput = host
        portInput = port.toString()
        micOn = p.getBoolean("mic", false)
        spkOn = p.getBoolean("speaker", false)
        micSourceId = p.getInt("micSource", MicSources.ALL[0].id)
        heroTargetText = when {
            host.isEmpty() -> getString(R.string.hero_target_auto)
            port == Proto.PORT -> host
            else -> "$host:$port"
        }
    }

    /** 只有「连接成功」才把目标写回偏好；失败 / 只是改了输入框都不落盘。 */
    private fun commitTarget(h: String, p: Int) {
        if (h.isEmpty() || p !in 1..65535) return
        host = h
        port = p
        hostInput = h
        portInput = p.toString()
        saveTarget(h, p)
    }

    /** 扫码结果立即保存（用户明确选了目标），与是否连上无关。 */
    private fun saveTarget(h: String, p: Int) {
        if (h.isEmpty() || p !in 1..65535) return
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString("host", h)
            .putInt("port", p)
            .apply()
    }

    private fun savePrefs() {
        // 目标地址只在连接成功 / 扫码时写；这里只存与连接目标无关的开关。
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putBoolean("mic", micOn)
            .putBoolean("speaker", spkOn)
            .putInt("micSource", micSourceId)
            .apply()
    }
}