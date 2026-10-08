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

package com.purevox.mic.net

import android.util.Log
import com.purevox.mic.Proto
import com.purevox.mic.ServerMsg
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

/**
 * 与电脑端的 WebSocket 连接（明文 `ws://host:59123/ws`）。
 *
 * 职责边界：
 * - 这里**只管连接与收发**：文本消息往上抛（[Listener.onMessage]）、二进制包往上抛
 *   （[Listener.onAudioPacket]）、握手 / RTT ping 也在这里发。
 * - 音频的解码与播放在 `audio/SpeakerDownlink.kt`；上行编码在 `audio/MicUplink.kt`。
 *   本类不认识 Opus。
 * - RTT 由本类测量（客户端自测再 `rtt` 上报，电脑的调试界面因此能直接看到）。
 *
 * 线程：所有回调在 OkHttp 的调度线程上；[sendAudio] / [sendText] 线程安全
 * （OkHttp 内部排队），所以音频线程可以直接调 [sendAudio]。
 * 注意 [Listener.onAudioPacket] 也在 OkHttp 线程上，**不要在那里做解码**。
 */
class WsClient(
    private val host: String,
    private val port: Int,
    private val listener: Listener,
) {

    interface Listener {
        /** WebSocket 握手完成（此时已自动发出 `hello`，`ready` 随后到达）。 */
        fun onOpen()

        /** 收到非 `pong` 的文本消息。 */
        fun onMessage(msg: ServerMsg)

        /** 收到一个 Opus 包（二进制帧）。数组是本类新分配的，可安全跨线程持有。 */
        fun onAudioPacket(packet: ByteArray)

        /** `pong` 回来，算好的往返时延（ms）。 */
        fun onPong(id: Long, rttMs: Double)

        /** 正常或被对端关闭。 */
        fun onClosed(reason: String)

        /** 链路层失败（连不上、被拒、读写异常）。 */
        fun onFailure(reason: String)
    }

    companion object {
        private const val TAG = "PureVoxWs"
        private const val PING_INTERVAL_MS = 1000L
        /** 电脑端空闲 60 s 断开，所以手机必须持续发消息；ping 就是为此。 */
        private const val PING_TABLE_LIMIT = 64
    }

    private var client: OkHttpClient? = null
    private var ws: WebSocket? = null
    private var pingThread: Thread? = null

    @Volatile
    private var opened = false

    private val pendingPings = ConcurrentHashMap<Long, Long>()
    private var pingSeq = 0L

    /** 上行（手机 → 电脑）已成功入队的 Opus 包数。 */
    val txPackets = AtomicLong(0)

    /** 下行（电脑 → 手机）收到的 Opus 包数。 */
    val rxPackets = AtomicLong(0)

    // ---------------------------------------------------------------- 连接

    fun connect() {
        disconnect()
        val c = OkHttpClient.Builder()
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.MILLISECONDS)     // WebSocket 是长连接，不设读超时
            .writeTimeout(10, TimeUnit.SECONDS)
            .pingInterval(20, TimeUnit.SECONDS)        // 协议层保活，与应用层 ping 互不替代
            .retryOnConnectionFailure(true)
            .build()
        client = c
        val request = Request.Builder()
            .url(Proto.wsUrl(host, port))
            .get()
            .build()
        ws = c.newWebSocket(request, SocketListener())
    }

    fun disconnect() {
        opened = false
        stopPing()
        val s = ws
        ws = null
        try {
            s?.close(1000, "client quit")
        } catch (e: Throwable) {
            Log.w(TAG, "close 异常", e)
        }
        pendingPings.clear()
        val c = client
        client = null
        // 关掉连接池里的空闲连接，避免反复连断开线程堆积
        c?.dispatcher?.executorService?.shutdown()
        c?.connectionPool?.evictAll()
    }

    // ---------------------------------------------------------------- 发送

    /** 发一条控制文本帧（内部消息一律由 `Proto` 拼好）。 */
    fun sendText(json: String): Boolean {
        val s = ws ?: return false
        return try {
            s.send(json)
        } catch (e: Throwable) {
            Log.w(TAG, "sendText 异常", e)
            false
        }
    }

    /** 发一个 Opus 包（二进制帧 = 一个包，无头）。[len] ≤ `data.size`。 */
    fun sendAudio(data: ByteArray, len: Int): Boolean {
        val s = ws ?: return false
        return try {
            val payload = if (len == data.size) data else data.copyOf(len)
            if (!s.send(payload.toByteString())) return false
            txPackets.incrementAndGet()
            true
        } catch (e: Throwable) {
            Log.w(TAG, "sendAudio 异常", e)
            false
        }
    }

    // ---------------------------------------------------------------- ping / pong

    private fun startPing() {
        stopPing()
        val t = Thread({
            while (opened) {
                try {
                    Thread.sleep(PING_INTERVAL_MS)
                } catch (e: InterruptedException) {
                    return@Thread
                }
                if (!opened) return@Thread
                val id = ++pingSeq
                pendingPings[id] = System.nanoTime()
                // 表不该无限长：只保留最近的若干条，其余的 pong 直接丢掉
                while (pendingPings.size > PING_TABLE_LIMIT) {
                    val oldest = pendingPings.keys.minOrNull() ?: break
                    pendingPings.remove(oldest)
                }
                sendText(Proto.ping(id))
            }
        }, "pv-ping")
        t.isDaemon = true
        pingThread = t
        t.start()
    }

    private fun stopPing() {
        val t = pingThread ?: return
        pingThread = null
        t.interrupt()
    }

    private fun onPong(id: Long) {
        val sent = pendingPings.remove(id) ?: return
        val ms = (System.nanoTime() - sent) / 1_000_000.0
        listener.onPong(id, ms)
        // 协议规定：RTT 由客户端算好后上报，电脑调试界面直接读
        sendText(Proto.rtt(ms))
    }

    // ---------------------------------------------------------------- OkHttp 回调

    private inner class SocketListener : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            opened = true
            sendText(Proto.hello())
            startPing()
            listener.onOpen()
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            when (val msg = ServerMsg.parse(text)) {
                is ServerMsg.Pong -> onPong(msg.id)
                else -> listener.onMessage(msg)
            }
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            // ByteString 的底层数组不能直接交给解码线程（可能复用），复制一份
            val packet = bytes.toByteArray()
            if (packet.isEmpty()) return
            rxPackets.incrementAndGet()
            listener.onAudioPacket(packet)
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            try {
                webSocket.close(1000, null)
            } catch (e: Throwable) {
                Log.w(TAG, "close 异常", e)
            }
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            opened = false
            stopPing()
            val why = if (reason.isNullOrEmpty()) "电脑关闭了连接（code=$code）" else reason
            listener.onClosed(why)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            opened = false
            stopPing()
            Log.w(TAG, "WebSocket 失败", t)
            listener.onFailure(describe(t))
        }
    }

    /** 把异常翻译成人能看懂的中文（直连失败是最常见的场景）。 */
    private fun describe(t: Throwable): String = when (t) {
        is IOException -> {
            val m = t.message.orEmpty()
            if (m.contains("refused", true) || m.contains("Failed to connect", true) || t is java.net.ConnectException) {
                "连不上 $host:$port —— 请确认电脑已启动网络服务（端口 ${Proto.PORT}）、" +
                    "手机与电脑在同一局域网、电脑防火墙放行了该端口"
            } else if (m.isEmpty()) {
                "网络错误（${t.javaClass.simpleName}）"
            } else {
                "网络错误：$m"
            }
        }
        else -> t.message ?: t.javaClass.simpleName
    }
}