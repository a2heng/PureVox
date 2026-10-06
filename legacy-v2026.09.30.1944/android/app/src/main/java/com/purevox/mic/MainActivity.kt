/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
# 
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
# 
# SPDX-License-Identifier: GPL-3.0-or-later
 */

package com.purevox.mic

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.media.MediaRecorder
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.ViewGroup
import android.widget.*
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.purevox.mic.audio.AudioCapture
import com.purevox.mic.audio.OpusEncoder
import com.purevox.mic.discovery.MdnsDiscovery
import com.purevox.mic.discovery.DiscoveredServer
import com.purevox.mic.network.WsClient
import com.purevox.mic.service.StreamService

class MainActivity : AppCompatActivity() {
    private var discovery: MdnsDiscovery? = null
    private var wsClient: WsClient? = null
    private var audioCapture: AudioCapture? = null
    private var opusEncoder: OpusEncoder? = null
    private var isStreaming = false
    private var connectedIp: String? = null
    private var connectedPort = 59123

    private val uiHandler = Handler(Looper.getMainLooper())
    private val discoverRunnable = object : Runnable {
        override fun run() {
            if (connectedIp == null) startDiscovery()
            uiHandler.postDelayed(this, 10000)
        }
    }

    // UI
    private lateinit var tvStatus: TextView
    private lateinit var tvServer: TextView
    private lateinit var tvLatency: TextView
    private lateinit var tvPackets: TextView
    private lateinit var tvStreamState: TextView
    private lateinit var tvLog: TextView
    private lateinit var btnMic: Button
    private lateinit var vuLevel: VuMeterView
    private lateinit var spinnerAudioSource: Spinner

    // Debug info TextViews
    private lateinit var dbgCtxSr: TextView
    private lateinit var dbgDevSr: TextView
    private lateinit var dbgEncSr: TextView
    private lateinit var dbgFrame: TextView
    private lateinit var dbgBitrate: TextView
    private lateinit var dbgRtt: TextView
    private lateinit var dbgTotalLat: TextView
    private lateinit var dbgServerSr: TextView
    private lateinit var dbgBacklog: TextView
    private lateinit var dbgEncoded: TextView

    private var audioSource = MediaRecorder.AudioSource.MIC
    private val sourceNames = arrayOf("麦克风 (MIC)", "VoIP 通话", "摄录", "语音识别", "原始音频")

    // VU
    private var vuDb = -60f
    private var vuPeakDb = -60f
    private var vuPeakTime = 0L
    private var lastFallTime = 0L
    private var isInForeground = false

    // RTT
    private val sendTimestamps = mutableMapOf<Int, Long>()
    private var rttAvg = 0.0
    private var rttSamples = 0
    private var packetCount = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        try {
            super.onCreate(savedInstanceState)
            setContentView(R.layout.activity_main)

            tvStatus = findViewById(R.id.tvStatus)
            tvServer = findViewById(R.id.tvServer)
            tvLatency = findViewById(R.id.tvLatency)
            tvPackets = findViewById(R.id.tvPackets)
            tvStreamState = findViewById(R.id.tvStreamState)
            tvLog = findViewById(R.id.tvLog)
            btnMic = findViewById(R.id.btnMic)
            vuLevel = findViewById(R.id.vuLevel)
            spinnerAudioSource = findViewById(R.id.spinnerAudioSource)

            dbgCtxSr = findViewById(R.id.dbg_ctx_sr)
            dbgDevSr = findViewById(R.id.dbg_dev_sr)
            dbgEncSr = findViewById(R.id.dbg_enc_sr)
            dbgFrame = findViewById(R.id.dbg_frame)
            dbgBitrate = findViewById(R.id.dbg_bitrate)
            dbgRtt = findViewById(R.id.dbg_rtt)
            dbgTotalLat = findViewById(R.id.dbg_total_lat)
            dbgServerSr = findViewById(R.id.dbg_server_sr)
            dbgBacklog = findViewById(R.id.dbg_backlog)
            dbgEncoded = findViewById(R.id.dbg_encoded)

            applyThemeColors()
            setupSourceSpinner()
            requestPermissions()
            uiHandler.post(discoverRunnable)
            btnMic.setOnClickListener { toggleStream() }
            log("UI 初始化完成")
            // 日志支持滑动
            tvLog.movementMethod = android.text.method.ScrollingMovementMethod()
        } catch (e: Throwable) {
            android.util.Log.e("PureVox", "onCreate崩溃", e)
            throw e
        }
    }

    private fun applyThemeColors() {
        val isDark = (resources.configuration.uiMode and
            android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        val border = if (isDark) Color.parseColor("#444444") else Color.parseColor("#d0c8b8")
        val logBg = if (isDark) Color.parseColor("#0a0a0a") else Color.parseColor("#f0ebe3")

        // Spinner 背景
        val bgInput = if (isDark) Color.parseColor("#1a1a1a") else Color.parseColor("#e8e2d8")
        spinnerAudioSource.background = GradientDrawable().apply {
            setColor(bgInput)
            setStroke(1, border)
            cornerRadius = 4f
        }

        // 按钮固定红色（停止态）
        setButtonRed()

        // 日志背景
        tvLog.setBackgroundDrawable(GradientDrawable().apply {
            setColor(logBg)
            setStroke(1, border)
            cornerRadius = 2f
        })
    }

    private fun setButtonRed() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            btnMic.backgroundTintList = android.content.res.ColorStateList.valueOf(
                Color.parseColor("#cc4433"))
        }
        btnMic.setTextColor(Color.WHITE)
    }

    private fun setButtonGreen() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            btnMic.backgroundTintList = android.content.res.ColorStateList.valueOf(
                Color.parseColor("#44bb66"))
        }
        btnMic.setTextColor(Color.WHITE)
    }

    override fun onResume() {
        super.onResume()
        isInForeground = true
    }

    override fun onPause() {
        super.onPause()
        isInForeground = false
    }

    override fun onDestroy() {
        super.onDestroy()
        uiHandler.removeCallbacks(discoverRunnable)
        stopDiscovery()
        audioCapture?.stop()
        opusEncoder?.destroy()
        wsClient?.disconnect()
    }

    private fun log(msg: String) {
        uiHandler.post {
            val t = java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault())
                .format(java.util.Date())
            tvLog.append("[$t] $msg\n")
            val limit = 100
            val lines = tvLog.text.split("\n")
            if (lines.size > limit) {
                tvLog.text = lines.drop(lines.size - limit).joinToString("\n")
            }
            // 滚动到底部
            val parent = tvLog.parent as? View
            if (parent is ScrollView) parent.fullScroll(View.FOCUS_DOWN)
        }
    }

    private fun setupSourceSpinner() {
        val adapter = object : ArrayAdapter<String>(this, android.R.layout.simple_spinner_item, sourceNames) {
            override fun getView(pos: Int, v: View?, parent: ViewGroup): View {
                val tv = super.getView(pos, v, parent) as TextView
                tv.textSize = 13f
                tv.ellipsize = android.text.TextUtils.TruncateAt.END
                tv.maxLines = 1
                return tv
            }
            override fun getDropDownView(pos: Int, v: View?, parent: ViewGroup): View {
                val tv = super.getDropDownView(pos, v, parent) as TextView
                tv.textSize = 13f
                tv.setPadding(24, 12, 24, 12)
                return tv
            }
        }
        spinnerAudioSource.adapter = adapter
        spinnerAudioSource.setSelection(0)
        spinnerAudioSource.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(p: AdapterView<*>, v: View?, pos: Int, id: Long) {
                audioSource = when (pos) {
                    0 -> MediaRecorder.AudioSource.MIC
                    1 -> MediaRecorder.AudioSource.VOICE_COMMUNICATION
                    2 -> if (Build.VERSION.SDK_INT >= 24) MediaRecorder.AudioSource.CAMCORDER else MediaRecorder.AudioSource.MIC
                    3 -> MediaRecorder.AudioSource.VOICE_RECOGNITION
                    4 -> if (Build.VERSION.SDK_INT >= 26) MediaRecorder.AudioSource.UNPROCESSED else MediaRecorder.AudioSource.MIC
                    else -> MediaRecorder.AudioSource.MIC
                }
                vuDb = -60f; vuPeakDb = -60f; vuPeakTime = 0L; lastFallTime = 0L
                vuLevel.updateLevel(-60f, -60f)
            }
            override fun onNothingSelected(p: AdapterView<*>) {}
        }
    }

    private fun requestPermissions() {
        val perms = mutableListOf(Manifest.permission.RECORD_AUDIO)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU)
            perms.add(Manifest.permission.POST_NOTIFICATIONS)
        val needed = perms.filter {
            ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
        }
        if (needed.isNotEmpty())
            ActivityCompat.requestPermissions(this, needed.toTypedArray(), 100)
    }

    private fun startDiscovery() {
        stopDiscovery()
        val mdns = MdnsDiscovery(this)
        mdns.onServerFound = { server ->
            uiHandler.post { onServerFound(server) }
        }
        mdns.start()
        discovery = mdns
    }

    private fun stopDiscovery() {
        discovery?.stop()
        discovery = null
    }

    private fun onServerFound(server: DiscoveredServer) {
        connectedIp = server.host
        connectedPort = server.port
        stopDiscovery()
        tvStatus.text = "已发现"
        tvServer.text = "https://${server.host}:${server.port}"
        tvServer.visibility = View.VISIBLE
        log("发现服务器: ${server.host}:${server.port}")
        connectToServer(server.host, server.port)
    }

    private fun connectToServer(ip: String, port: Int) {
        wsClient = WsClient(ip, port)
        wsClient?.onConnected = {
            uiHandler.post {
                tvStatus.text = "已连接"
                tvStatus.setTextColor(
                    if ((resources.configuration.uiMode and
                            android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
                        android.content.res.Configuration.UI_MODE_NIGHT_YES)
                        Color.parseColor("#55cc66") else Color.parseColor("#558855"))
                btnMic.isEnabled = true
                updateDebugInfo()
                log("WebSocket 已连接")
            }
        }
        wsClient?.onDisconnected = {
            uiHandler.post {
                tvStatus.text = "已断开"
                tvStatus.setTextColor(
                    if ((resources.configuration.uiMode and
                            android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
                        android.content.res.Configuration.UI_MODE_NIGHT_YES)
                        Color.parseColor("#cc5544") else Color.parseColor("#cc5544"))
                btnMic.isEnabled = false
                stopStreaming()
                wsClient = null
                connectedIp = null
                log("WebSocket 已断开")
                updateDebugInfo()
            }
        }
        wsClient?.onError = { msg ->
            uiHandler.post {
                tvStatus.text = "连接失败"
                log("连接失败: $msg")
                wsClient = null
                connectedIp = null
            }
        }
        wsClient?.onAck = { seq ->
            val sent = sendTimestamps.remove(seq)
            if (sent != null) {
                val rtt = System.currentTimeMillis() - sent
                rttAvg = (rttAvg * rttSamples + rtt) / (rttSamples + 1)
                rttSamples = minOf(rttSamples + 1, 100)
                uiHandler.post {
                    tvLatency.text = "${rttAvg.toInt()}ms"
                    updateDebugInfo()
                }
            }
        }
        wsClient?.connect()
    }

    private fun startStreaming() {
        if (isStreaming) return
        opusEncoder = OpusEncoder()
        audioCapture = AudioCapture(this)
        audioCapture?.onPcmData = { pcm ->
            val opus = opusEncoder?.encode(pcm)
            if (opus != null) {
                val seq = wsClient?.sendAudio(opus)
                if (seq != null) {
                    sendTimestamps[seq] = System.currentTimeMillis()
                    packetCount++
                    uiHandler.post {
                        tvPackets.text = packetCount.toString()
                        updateDebugInfo()
                    }
                }
            }
        }
        audioCapture?.onLevel = { level ->
            val db = 20f * kotlin.math.log10(level.coerceAtLeast(1e-6f))
            val now = System.currentTimeMillis()
            if (db > vuPeakDb) { vuPeakDb = db; vuPeakTime = now; lastFallTime = now }
            vuDb = db
            uiHandler.post {
                if (!isInForeground) return@post
                if (now - vuPeakTime > 3000) {
                    val dt = (now - lastFallTime).coerceAtMost(100L) / 1000f
                    vuPeakDb = (vuPeakDb - 20f * dt).coerceAtMost(-60f)
                    lastFallTime = now
                }
                vuLevel.updateLevel(vuDb, vuPeakDb)
            }
        }
        if (audioCapture?.start(audioSource) == true) {
            isStreaming = true
            btnMic.text = "停止推流"
            setButtonGreen()
            tvStreamState.text = "● 推流"
            tvStreamState.setTextColor(Color.parseColor("#55cc66"))
            spinnerAudioSource.isEnabled = false
            startForegroundService()
            log("推流已开始")
            updateDebugInfo()
        } else {
            tvStatus.text = "麦克风启动失败"
            log("错误: 麦克风启动失败")
        }
    }

    private fun stopStreaming() {
        if (!isStreaming) return

        // 发 flush 通知服务端清空缓冲（不断开 WSS）
        wsClient?.sendFlush()

        audioCapture?.stop()
        opusEncoder?.destroy()
        audioCapture = null
        opusEncoder = null
        isStreaming = false
        btnMic.text = "开始推流"
        setButtonRed()
        tvStreamState.text = "○ 停止"
        val isDark = (resources.configuration.uiMode and
            android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        tvStreamState.setTextColor(if (isDark) Color.parseColor("#888888") else Color.parseColor("#8a8580"))
        spinnerAudioSource.isEnabled = true
        stopForegroundService()
        log("推流已停止")
        updateDebugInfo()
    }

    private fun updateDebugInfo() {
        dbgEncSr.text = "enc SR 48000"
        dbgFrame.text = "frame 480"
        dbgBitrate.text = "bitrate 32k"
        dbgEncoded.text = "encoded $packetCount"

        dbgRtt.text = if (rttSamples > 0) "RTT ${rttAvg.toInt()}ms" else "RTT --"

        // 总延迟估算: RTT/2 + frame(10ms) + prefill(30ms) + TARGET_ACC(50ms)
        val estOneWay = if (rttSamples > 0) (rttAvg / 2).toInt() else 50
        val totalLat = estOneWay + 10 + 30 + 50
        val latColor = when {
            totalLat < 200 -> "#55cc66"
            totalLat < 500 -> "#cccc44"
            else -> "#cc5544"
        }
        dbgTotalLat.text = "总延迟 ${totalLat}ms"

        // ctx SR / dev SR — Android 固定 48000
        dbgCtxSr.text = "ctx SR 48000"
        dbgDevSr.text = "dev SR 48000"
        dbgServerSr.text = "srv SR --"
        dbgBacklog.text = "backlog --"
    }

    private fun startForegroundService() {
        val intent = Intent(this, StreamService::class.java).apply {
            action = StreamService.ACTION_START
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O)
            startForegroundService(intent)
        else startService(intent)
    }

    private fun stopForegroundService() {
        val intent = Intent(this, StreamService::class.java).apply {
            action = StreamService.ACTION_STOP
        }
        stopService(intent)
    }

    private fun toggleStream() {
        if (isStreaming) stopStreaming()
        else startStreaming()
    }
}
