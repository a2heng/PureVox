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
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.Editable
import android.text.TextWatcher
import android.util.Log
import android.view.KeyEvent
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.widget.SwitchCompat
import androidx.core.content.ContextCompat
import com.purevox.mic.audio.MicUplink
import com.purevox.mic.audio.OpusSupport
import com.purevox.mic.audio.SpeakerDownlink
import com.purevox.mic.net.HealthProbe
import com.purevox.mic.net.WsClient
import com.purevox.mic.service.CaptureService

/**
 * 唯一的界面与编排者（单 Activity，Material 风格）。
 *
 * 它负责把三条独立能力接上电脑端的同一个 WebSocket：
 * 1. 手机麦克风 → 电脑（[MicUplink]）
 * 2. 电脑音频 → 手机扬声器（[SpeakerDownlink]）
 * 3. 远程输入：IME 已组合文本（`text`）+ 手机实体键（`key`）。电脑端**默认关闭**，
 *    没开开关时电脑会回 `err`，本界面把 `err` 原文显示出来，不静默丢弃。
 *
 * 纪律：
 * - **收到 `ready` 之前不推流、不播放**（[applyState] 里 `ready` 是必要条件）。
 * - 状态区是固定行数的读数（连接状态 / RTT / 收发包数 / 播放水位 / Opus 探测），
 *   2 Hz 刷新；失败项显示「不可用 + 原因」，绝不显示伪造的 0。
 * - 所有 OkHttp 回调都切回主线程再动 UI。
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "PureVoxUI"
        private const val PREFS = "purevox"
        private const val DEFAULT_HOST = "192.168.1.100"
        private const val MAX_LOG_LINES = 80
        private const val STATUS_INTERVAL_MS = 500L

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

    private var ws: WsClient? = null
    private var uplink: MicUplink? = null
    private var downlink: SpeakerDownlink? = null

    private var host = DEFAULT_HOST
    private var connState = "未连接"
    private var connecting = false
    private var lastErr = ""
    private var opusDetail = ""
    private var rttMs = Double.NaN
    private var healthText = "未自检"

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

    // 界面控件
    private lateinit var etHost: EditText
    private lateinit var btnConnect: Button
    private lateinit var btnHealth: Button
    private lateinit var tvStatus: TextView
    private lateinit var tvHealth: TextView
    private lateinit var tvErr: TextView
    private lateinit var swMic: SwitchCompat
    private lateinit var swSpeaker: SwitchCompat
    private lateinit var swRemote: SwitchCompat
    private lateinit var swRemoteText: SwitchCompat
    private lateinit var swRemoteKeys: SwitchCompat
    private lateinit var etText: EditText
    private lateinit var tvLog: TextView

    private val ticker = object : Runnable {
        override fun run() {
            updateStatus()
            ui.postDelayed(this, STATUS_INTERVAL_MS)
        }
    }

    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
            applyState()
            updateStatus()
        }

    // ---------------------------------------------------------------- 生命周期

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        bindViews()
        restorePrefs()

        opusDetail = OpusSupport.probe().detail
        if (opusDetail.isEmpty()) opusDetail = "未探测到 audio/opus 编解码器"

        btnConnect.setOnClickListener {
            if (ws == null && !connecting) connect() else disconnect()
        }
        btnHealth.setOnClickListener { runHealthCheck() }

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

        etText.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}

            override fun afterTextChanged(s: Editable?) {
                if (suppressTextSync) return
                val typed = s?.toString().orEmpty()
                if (typed.isEmpty()) return
                if (!remoteTextOn || !ready) return
                // 同步给电脑后清空：既让电脑拿到「已组合好」的文本，又不与本地输入打架
                suppressTextSync = true
                try {
                    sendText(typed)
                } finally {
                    etText.setText("")
                    suppressTextSync = false
                }
            }
        })

        requestPermissions()
        refreshRemoteSwitches()
        log("PureVox 客户端就绪 · $opusDetail")
    }

    override fun onStart() {
        super.onStart()
        ui.post(ticker)
    }

    override fun onStop() {
        ui.removeCallbacks(ticker)
        super.onStop()
    }

    override fun onPause() {
        super.onPause()
        savePrefs()
    }

    override fun onDestroy() {
        ui.removeCallbacks(ticker)
        teardown()
        super.onDestroy()
    }

    // ---------------------------------------------------------------- 连接

    private fun connect() {
        val target = etHost.text.toString().trim()
        if (target.isEmpty()) {
            showErr("请先填写电脑的 IP 或主机名")
            return
        }
        host = target
        savePrefs()
        connecting = true
        ready = false
        rttMs = Double.NaN
        lastErr = ""
        connState = "连接中…"
        val client = WsClient(host, Proto.PORT, wsListener)
        ws = client
        client.connect()
        updateButtons()
        updateStatus()
        log("连接 ${Proto.wsUrl(host)}")
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
        rttMs = Double.NaN
        connState = "已断开"
        applyState()
        updateButtons()
        updateStatus()
        log(reason)
    }

    private val wsListener = object : WsClient.Listener {
        override fun onOpen() {
            ui.post {
                connState = "已连接（等待 ready）"
                updateButtons()
                updateStatus()
                log("WebSocket 已连接，已发送 hello(proto=${Proto.VERSION})")
            }
        }

        override fun onMessage(msg: ServerMsg) {
            ui.post {
                when (msg) {
                    is ServerMsg.Ready -> onReady(msg)
                    is ServerMsg.Err -> showErr("电脑：${msg.msg}")
                    is ServerMsg.Pong -> Unit
                    is ServerMsg.Other -> log("未识别的消息 t=${msg.type}")
                }
                updateStatus()
            }
        }

        override fun onAudioPacket(packet: ByteArray) {
            val d = downlink
            if (d != null && speakerOn) d.enqueue(packet)
        }

        override fun onPong(id: Long, rtt: Double) {
            ui.post { rttMs = rtt }
        }

        override fun onClosed(reason: String) {
            ui.post { onSessionEnded("电脑关闭了连接：$reason") }
        }

        override fun onFailure(reason: String) {
            ui.post { onSessionEnded("连接失败：$reason") }
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
        log(
            "ready · proto=${msg.proto} codec=${msg.codec} rate=${msg.rate} " +
                "ch=${msg.channels} frame=${msg.frameMs}ms 电脑侧客户端=${msg.clients}",
        )
        // sub 只由 applyStateInner 发一次（电脑端每收一条 sub 就注册一个新的出站订阅，
        // 重复发会泄漏订阅），这里不重复发。
        applyState()
        updateButtons()
    }

    private fun runHealthCheck() {
        val target = etHost.text.toString().trim()
        if (target.isNotEmpty()) host = target
        healthText = "自检中…"
        updateStatus()
        HealthProbe.check(host) { ok, text ->
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
            CaptureService.stop(this)
            log("麦克风上行已停止")
            updateButtons()
        }

        speakerOn = swSpeaker.isChecked && ready
        if (speakerOn && downlink == null) {
            val d = SpeakerDownlink()
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
            // 协议没有退订消息，电脑会继续推包；这里只是不再解码/播放
            log("电脑音频播放已停止（电脑端仍在推流，协议无退订消息，重连才会停止）")
        }

        remoteTextOn = swRemote.isChecked && swRemoteText.isChecked && ready
        remoteKeysOn = swRemote.isChecked && swRemoteKeys.isChecked && ready
        updateButtons()
    }

    /** 起上行：先起前台服务（Android 9+ 后台麦克风的前提），再开 AudioRecord。 */
    private fun startUplink(): Boolean {
        CaptureService.start(this, getString(R.string.notif_starting))
        val u = MicUplink(
            context = this,
            onPacket = { pkt -> ws?.sendAudio(pkt, pkt.size) },
            onError = { reason -> ui.post { showErr(reason) } },
        )
        if (!u.start()) {
            CaptureService.stop(this)
            return false
        }
        uplink = u
        log(
            "麦克风上行已启动：采集 ${OpusSupport.pcmName(u.recordEncoding)} @${MicUplink.SAMPLE_RATE}Hz " +
                "→ 编码 ${u.encoderName}",
        )
        return true
    }

    private fun teardown() {
        uplink?.stop()
        uplink = null
        downlink?.stop()
        downlink = null
        CaptureService.stop(this)
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

    // ---------------------------------------------------------------- 状态显示

    private fun updateButtons() {
        val online = ready
        btnConnect.text = if (ws != null || connecting) getString(R.string.btn_disconnect) else getString(R.string.btn_connect)
        btnConnect.isEnabled = !connecting
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
    }

    /** 固定 7 行读数；行数固定，数值变长也不跳布局。 */
    private fun updateStatus() {
        val c = ws
        val enc = uplink
        val dec = downlink
        val line = StringBuilder()
        line.append(row(getString(R.string.status_conn), connState))
        line.append(row(getString(R.string.status_host), Proto.wsUrl(host)))
        line.append(
            row(
                getString(R.string.status_rtt),
                if (rttMs.isNaN()) "测量中…" else String.format("%.1f ms", rttMs),
            ),
        )
        line.append(
            row(
                getString(R.string.status_up),
                if (c == null) {
                    "未连接"
                } else {
                    "${c.txPackets.get()} 包" + if (enc != null) {
                        " · 编码 ${enc.encoderName} · 采集 ${OpusSupport.pcmName(enc.recordEncoding)}" +
                            " · 电平 ${String.format("%.2f", enc.level)}"
                    } else {
                        " · 未开启"
                    }
                },
            ),
        )
        line.append(
            row(
                getString(R.string.status_down),
                if (c == null) {
                    "未连接"
                } else {
                    "${c.rxPackets.get()} 包" + if (dec != null) {
                        " · 解码 ${dec.decoderName} · 水位 ${dec.bufferedMs().toInt()} ms"
                    } else {
                        " · 未订阅"
                    }
                },
            ),
        )
        line.append(
            row(
                getString(R.string.status_play),
                if (dec == null) {
                    "未开启"
                } else {
                    "欠载 ${dec.underruns} · 丢包 ${dec.dropped} · 峰值 ${String.format("%.2f", dec.peak)}" +
                        (dec.lastError?.let { " · $it" } ?: "")
                },
            ),
        )
        line.append(row(getString(R.string.status_opus), opusDetail))
        tvStatus.text = line.toString()

        tvHealth.text = getString(R.string.status_health) + healthText
        tvErr.text = lastErr
        tvErr.visibility = if (lastErr.isEmpty()) android.view.View.GONE else android.view.View.VISIBLE
    }

    private fun row(label: String, value: String): String =
        String.format("%-11s %s%n", label, value)

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
            tvLog.post { tvLog.scrollTo(0, tvLog.layoutHeight) }
        }
    }

    // ---------------------------------------------------------------- 偏好 & 绑定

    private fun bindViews() {
        etHost = findViewById(R.id.etHost)
        btnConnect = findViewById(R.id.btnConnect)
        btnHealth = findViewById(R.id.btnHealth)
        tvStatus = findViewById(R.id.tvStatus)
        tvHealth = findViewById(R.id.tvHealth)
        tvErr = findViewById(R.id.tvErr)
        swMic = findViewById(R.id.swMic)
        swSpeaker = findViewById(R.id.swSpeaker)
        swRemote = findViewById(R.id.swRemote)
        swRemoteText = findViewById(R.id.swRemoteText)
        swRemoteKeys = findViewById(R.id.swRemoteKeys)
        etText = findViewById(R.id.etText)
        tvLog = findViewById(R.id.tvLog)
    }

    private fun restorePrefs() {
        val p = getSharedPreferences(PREFS, MODE_PRIVATE)
        host = p.getString("host", DEFAULT_HOST) ?: DEFAULT_HOST
        etHost.setText(host)
        swMic.isChecked = p.getBoolean("mic", false)
        swSpeaker.isChecked = p.getBoolean("speaker", false)
        swRemote.isChecked = p.getBoolean("remote", false)
        swRemoteText.isChecked = p.getBoolean("remoteText", true)
        swRemoteKeys.isChecked = p.getBoolean("remoteKeys", true)
    }

    private fun savePrefs() {
        // 用户改了地址框但没点连接时也别丢掉
        val typed = etHost.text.toString().trim()
        if (typed.isNotEmpty()) host = typed
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString("host", host)
            .putBoolean("mic", swMic.isChecked)
            .putBoolean("speaker", swSpeaker.isChecked)
            .putBoolean("remote", swRemote.isChecked)
            .putBoolean("remoteText", swRemoteText.isChecked)
            .putBoolean("remoteKeys", swRemoteKeys.isChecked)
            .apply()
    }
}