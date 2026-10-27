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
import java.net.InetAddress
import java.net.Socket
import javax.net.SocketFactory
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

/**
 * 与电脑端的 WebSocket 连接（`wss://host:59123/ws` —— 电脑端只监听 HTTPS，
 * 自签证书的放行规则见 [LocalTls]）。
 *
 * ## 这一层只做四件事
 *
 * 1. **连接 / 收发**：文本消息往上抛，音频包拆掉 8 字节帧头后往上抛；
 * 2. **音频帧头**：上行加 `seq` + 采集时刻，下行解析出 `seq`/`ts`（见 [Proto]）；
 * 3. **时钟同步**：回 `syncreply`、收 `offset`（NTP 四时戳，电脑算偏移）；
 * 4. **`TCP_NODELAY`**：见 [LowLatencySocketFactory]。
 *
 * 编解码在 `audio/`，播放环在 `audio/PlaybackRing.kt`。本类不认识 Opus，也不做缓冲。
 *
 * ## 为什么必须 `TCP_NODELAY`
 *
 * 我们的包是 10 ms 一个、每个只有几十到一百多字节。Nagle 算法会把「还有未确认数据」
 * 时的新包攒起来，等 ACK 回来再发 —— 在 Wi-Fi 省电让 ACK 变慢时，这会把平滑的 10 ms
 * 序列变成「憋一下再来一串」，听感是周期性断音 / 延迟抖动。OkHttp 没有开关，但
 * `OkHttpClient.socketFactory` 允许我们交出已经设好 `tcpNoDelay` 的 socket，
 * TLS 再包一层也不影响这个设置（同一根 TCP 连接）。
 */
class WsClient(
    private val host: String,
    private val port: Int,
    private val listener: Listener,
) {

    interface Listener {
        /**
         * 所有回调都带 `client`：**界面上可能已经换了一条新连接**，而旧连接关闭 / 失败
         * 的回调总是晚一步到达。调用方必须用 `client !== 当前连接` 把旧事件丢掉，
         * 否则「重连 / 扫码切电脑」会被旧回调当成新会话结束（用户 2026-10-27 实测：
         * 第二次重连 / 第二次扫码直接断开）。
         */
        fun onOpen(client: WsClient)

        /** 收到非 `pong` / `sync` / `offset` 的文本消息。 */
        fun onMessage(client: WsClient, msg: ServerMsg)

        /** 收到一个音频包（**已去掉 8 字节帧头**）。数组是本类新分配的，可安全跨线程持有。 */
        fun onAudioPacket(client: WsClient, seq: Int, tsUs: Int, payload: ByteArray)

        /** `pong` 回来，算好的往返时延（ms）。 */
        fun onPong(client: WsClient, id: Long, rttMs: Double)

        /** 正常或被对端关闭。 */
        fun onClosed(client: WsClient, reason: String)

        /** 链路层失败（连不上、被拒、读写异常）。 */
        fun onFailure(client: WsClient, reason: String)
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

    /** 上行包序号（每个连接从 0 开始）。 */
    private var txSeq = 0

    /** 电脑算出的时钟偏移（手机 − 电脑，µs）；`Long.MIN_VALUE` = 还没测出来。 */
    private val offsetUs = AtomicLong(Long.MIN_VALUE)

    /** 上行（手机 → 电脑）已成功入队的 Opus 包数。 */
    val txPackets = AtomicLong(0)

    /** 下行（电脑 → 手机）收到的 Opus 包数。 */
    val rxPackets = AtomicLong(0)

    /** 上/下行**字节数**（含控制帧），诊断里按秒差分算真实码率。 */
    val txBytes = AtomicLong(0)
    val rxBytes = AtomicLong(0)

    /** 时钟偏移（µs）；没同步过返回 null（界面显示「测量中」，不编 0）。 */
    val clockOffsetUs: Long?
        get() {
            val v = offsetUs.get()
            return if (v == Long.MIN_VALUE) null else v
        }

    // ---------------------------------------------------------------- 连接

    fun connect() {
        disconnect()
        // 证书校验在 OkHttp 线程上跑，`X509TrustManager` 看不到主机名，只能靠这里记下目标
        LocalTls.setTarget(host)
        val c = OkHttpClient.Builder()
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.MILLISECONDS) // WebSocket 是长连接，不设读超时
            .writeTimeout(10, TimeUnit.SECONDS)
            .pingInterval(20, TimeUnit.SECONDS) // 协议层保活，与应用层 ping 互不替代
            .retryOnConnectionFailure(true)
            // 音频包只有几十字节、10 ms 一个：关掉 Nagle（见类注释）
            .socketFactory(LowLatencySocketFactory)
            // 电脑端是 HTTPS 且证书自签（每台机器自己签）。只对**私有网段**放行
            .sslSocketFactory(LocalTls.socketFactory(), LocalTls.trustManager())
            .hostnameVerifier(LocalTls.hostnameVerifier())
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
        offsetUs.set(Long.MIN_VALUE)
        val c = client
        client = null
        c?.dispatcher?.executorService?.shutdown()
        c?.connectionPool?.evictAll()
    }

    // ---------------------------------------------------------------- 发送

    /** 发一条控制文本帧（消息一律由 `Proto` 拼好）。 */
    fun sendText(json: String): Boolean {
        val s = ws ?: return false
        return try {
            if (!s.send(json)) return false
            txBytes.addAndGet(json.toByteArray(Charsets.UTF_8).size.toLong())
            true
        } catch (e: Throwable) {
            Log.w(TAG, "sendText 异常", e)
            false
        }
    }

    /**
     * 发一个 Opus 包：`8 字节帧头（seq + 采集时刻 µs）+ 载荷`，拼成**一条** WS 二进制帧。
     *
     * 为什么时间戳由采集侧给：它要表示「这段音频是什么时候从麦克风进来的」，
     * 晚到再补一个当前时间就完全量不出延迟了。
     */
    fun sendAudio(payload: ByteArray, len: Int, tsUs: Int): Boolean {
        val s = ws ?: return false
        return try {
            val n = len.coerceIn(0, payload.size)
            val msg = ByteArray(Proto.HDR_BYTES + n)
            System.arraycopy(Proto.header(txSeq, tsUs), 0, msg, 0, Proto.HDR_BYTES)
            System.arraycopy(payload, 0, msg, Proto.HDR_BYTES, n)
            txSeq++
            if (!s.send(msg.toByteString())) return false
            txPackets.incrementAndGet()
            txBytes.addAndGet(msg.size.toLong())
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
        listener.onPong(this@WsClient, id, ms)
        // 协议规定：RTT 由客户端算好后上报，电脑调试界面直接读
        sendText(Proto.rtt(ms))
    }

    // ---------------------------------------------------------------- OkHttp 回调

    private inner class SocketListener : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            opened = true
            txSeq = 0
            offsetUs.set(Long.MIN_VALUE)
            sendText(Proto.hello())
            startPing()
            listener.onOpen(this@WsClient)
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            rxBytes.addAndGet(text.toByteArray(Charsets.UTF_8).size.toLong())
            when (val msg = ServerMsg.parse(text)) {
                is ServerMsg.Pong -> onPong(msg.id)
                is ServerMsg.Sync -> {
                    // NTP 四时戳：t1 = 收到、t2 = 回（都用手机的单调时钟）
                    val t1 = Proto.nowUs()
                    sendText(Proto.syncReply(msg.id, msg.t0, t1, Proto.nowUs()))
                }
                is ServerMsg.Offset -> offsetUs.set(msg.us)
                else -> listener.onMessage(this@WsClient, msg)
            }
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            val raw = bytes.toByteArray()
            if (raw.size <= Proto.HDR_BYTES) return
            val seq = Proto.readU32(raw, 0)
            val ts = Proto.readU32(raw, 4)
            val payload = raw.copyOfRange(Proto.HDR_BYTES, raw.size)
            rxPackets.incrementAndGet()
            rxBytes.addAndGet(raw.size.toLong())
            listener.onAudioPacket(this@WsClient, seq, ts, payload)
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
            listener.onClosed(this@WsClient, why)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            opened = false
            stopPing()
            Log.w(TAG, "WebSocket 失败", t)
            listener.onFailure(this@WsClient, describe(t))
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

/**
 * 交出 `tcpNoDelay = true` 的 socket（见 [WsClient] 的类注释）。
 *
 * 只改这一个选项：socket 的其他行为（连接超时、TLS 包装）仍走 OkHttp 与系统默认。
 */
private object LowLatencySocketFactory : SocketFactory() {
    private fun tune(s: Socket): Socket = s.apply {
        try {
            tcpNoDelay = true
        } catch (e: Throwable) {
            Log.w("PureVoxWs", "设置 TCP_NODELAY 失败", e)
        }
    }

    override fun createSocket(): Socket = tune(Socket())

    override fun createSocket(host: String?, port: Int): Socket =
        tune(Socket()).apply { connect(java.net.InetSocketAddress(host, port)) }

    override fun createSocket(host: String?, port: Int, localHost: InetAddress?, localPort: Int): Socket =
        tune(Socket(host, port, localHost, localPort))

    override fun createSocket(host: InetAddress?, port: Int): Socket =
        tune(Socket(host, port))

    override fun createSocket(address: InetAddress?, port: Int, localAddress: InetAddress?, localPort: Int): Socket =
        tune(Socket(address, port, localAddress, localPort))
}
