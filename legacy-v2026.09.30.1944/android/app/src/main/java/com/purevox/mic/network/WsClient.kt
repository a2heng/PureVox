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

package com.purevox.mic.network

import android.util.Base64
import okhttp3.*
import org.json.JSONObject
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager

class WsClient(private val serverIp: String, private val serverPort: Int) {
    private var ws: WebSocket? = null
    private var client: OkHttpClient? = null
    private var seq = 0
    var onConnected: (() -> Unit)? = null
    var onDisconnected: (() -> Unit)? = null
    var onError: ((String) -> Unit)? = null
    var onAck: ((seq: Int) -> Unit)? = null

    fun connect() {
        val trustAllCerts = arrayOf<TrustManager>(object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {}
            override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {}
            override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
        })
        val sslContext = SSLContext.getInstance("TLS").apply {
            init(null, trustAllCerts, SecureRandom())
        }
        client = OkHttpClient.Builder()
            .sslSocketFactory(sslContext.socketFactory, trustAllCerts[0] as X509TrustManager)
            .hostnameVerifier { _, _ -> true }
            .readTimeout(0, TimeUnit.MILLISECONDS)
            .build()
        val request = Request.Builder()
            .url("wss://$serverIp:$serverPort/ws/audio")
            .build()
        ws = client?.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                onConnected?.invoke()
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                try {
                    val json = JSONObject(text)
                    if (json.optString("type") == "ack") {
                        onAck?.invoke(json.optInt("seq", 0))
                    }
                } catch (_: Exception) {}
            }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
                onDisconnected?.invoke()
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                onError?.invoke(t.message ?: "连接失败")
            }
        })
    }

    fun sendAudio(opusData: ByteArray): Int {
        val b64 = Base64.encodeToString(opusData, Base64.NO_WRAP)
        val s = seq++
        val msg = JSONObject().apply {
            put("type", "audio")
            put("data", b64)
            put("seq", s)
            put("timestamp", System.currentTimeMillis())
        }
        ws?.send(msg.toString())
        return s
    }

    fun sendStop() {
        val msg = JSONObject().apply { put("type", "stop") }
        ws?.send(msg.toString())
    }

    fun sendFlush() {
        val msg = JSONObject().apply {
            put("type", "flush")
            put("last_seq", seq - 1)  // 最后发出的 seq
        }
        ws?.send(msg.toString())
    }

    fun disconnect() {
        ws?.close(1000, "user disconnect")
        ws = null
    }
}
