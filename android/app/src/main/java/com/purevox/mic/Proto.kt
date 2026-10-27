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

import org.json.JSONObject

/**
 * 电脑端 WebSocket 协议的唯一镜像（对应 `src-tauri/src/net/proto.rs`）。
 *
 * 这里只做「拼 JSON / 解 JSON / 拼 8 字节帧头」，不含任何状态 —— 状态在
 * `net/WsClient.kt`、`audio/SpeakerDownlink.kt`、`audio/MicUplink.kt`。
 *
 * ## 协议 3 与「帧头 + 时间戳」
 *
 * 音频包 = **8 字节帧头 + 一个 Opus 包**（大端）：
 *
 * ```text
 *   0        4        8
 *   | seq u32| ts u32 | Opus 载荷…
 * ```
 *
 * - `seq`：每个方向独立的包序号，用来判丢包 / 重复；
 * - `ts`：发送方的单调时钟（**微秒**，u32，71.6 分钟回绕）。收端靠它 + 时钟偏移
 *   （`sync`/`syncreply` 估出来）算**单向延迟** —— 没有它，「延迟到底多少毫秒」
 *   只能靠猜。回绕比较一律用 [usDiff]。
 *
 * 缓冲**只在接收方**：电脑 → 手机这条路上，电脑每出一个 10 ms hop 就发一包，
 * 手机上唯一的播放环（`audio/Rt.kt`）负责吸收网络抖动与两端晶振差。
 */
object Proto {
    /** 协议版本；必须等于电脑端的 `proto::PROTO`，否则电脑回 `err` 并断开。 */
    const val VERSION = 3

    /** 端口与路径（沿用旧实现）。 */
    const val PORT = 59123
    const val WS_PATH = "/ws"
    const val HEALTH_PATH = "/health"

    /** 音频参数（协议硬约定，改动要同步电脑端）。 */
    /** 协议里的 codec 字段（电脑端 `ServerMsg::Ready` 发的是 `"opus"`，不是 MIME）。 */
    const val CODEC = "opus"
    const val SAMPLE_RATE = 48000
    const val CHANNELS = 1

    /** 上行建议码率（bit/s）；电脑端出站也是 32 k。 */
    const val BITRATE = 32000

    /** 帧头字节数（电脑端 `ready.header_bytes` 会再报一遍，两边都要对）。 */
    const val HDR_BYTES = 8

    /** 帧 = 10 ms @48 kHz。 */
    const val FRAME_SAMPLES = 480

    /**
     * WebSocket 地址。**必须是 `wss://`**：电脑端的服务是 HTTPS（NETWORK.md §4.1.2）。
     *
     * 为什么必须 HTTPS：`getUserMedia` 之类要「安全上下文」，而 `http://<局域网IP>` 不是；
     * 更根本的是**编解码器放在浏览器里**，非安全上下文下浏览器直接禁用。同样地，
     * 电脑端自签证书也逼着这里走 TLS —— 混协议的代价是「一半能用一半不能用」。
     */
    fun wsUrl(host: String, port: Int = PORT): String = "wss://$host:$port$WS_PATH"

    /** 自检地址，同样走 HTTPS。 */
    fun healthUrl(host: String, port: Int = PORT): String = "https://$host:$port$HEALTH_PATH"

    /** `{"t":"hello","proto":3}` —— 连上必须先握手，电脑回 `ready` 才算可用。 */
    fun hello(): String = JSONObject().put("t", "hello").put("proto", VERSION).toString()

    /** `{"t":"sub"}` —— 订阅电脑音频（电脑当手机音箱）。 */
    fun sub(): String = JSONObject().put("t", "sub").toString()

    /** `{"t":"unsub"}` —— 退订电脑音频（关扬声器时发）。 */
    fun unsub(): String = JSONObject().put("t", "unsub").toString()

    /** `{"t":"text","s":"..."}` —— 手机输入法**已组合**好的文本。 */
    fun text(s: String): String = JSONObject().put("t", "text").put("s", s).toString()

    /** `{"t":"key","code":29,"down":true}` —— `code` 就是 Android `KeyEvent.KEYCODE_*`。 */
    fun key(code: Int, down: Boolean): String =
        JSONObject().put("t", "key").put("code", code).put("down", down).toString()

    /** `{"t":"ping","id":42}` —— 电脑原样回 `{"t":"pong","id":42}`。 */
    fun ping(id: Long): String = JSONObject().put("t", "ping").put("id", id).toString()

    /** `{"t":"rtt","ms":7.5}` —— 时延由手机算（它才是发 ping 的一方），上报给电脑调试界面。 */
    fun rtt(ms: Double): String = JSONObject().put("t", "rtt").put("ms", ms).toString()

    /**
     * `{"t":"settarget","ms":40}` —— 改接收方播放环深度（**手动**，10~200 ms、按 10 步进）。
     *
     * 电脑收到后落盘并广播 `target` 给所有客户端（含本机），所以两边界面与环永远同一个值。
     */
    fun setTarget(ms: Int): String =
        JSONObject().put("t", "settarget").put("ms", ms).toString()

    /**
     * `{"t":"st",…}` —— 手机侧播放 / 采集状态，2 Hz 上报。
     *
     * 为什么手机要报：**缓冲在接收方**（NETWORK.md §4.1），电脑那一侧看不到手机环的水位、
     * 欠载、以及手机音频时钟实测偏差。没有这份上报，「为什么还是慢」只能靠猜。
     */
    fun stats(
        levelMs: Double,
        targetMs: Double,
        underruns: Long,
        concealed: Long,
        late: Long,
        oneWayMs: Double?,
        clockPpm: Double?,
    ): String {
        val o = JSONObject()
            .put("t", "st")
            .put("level_ms", round3(levelMs))
            .put("target_ms", round3(targetMs))
            .put("underruns", underruns)
            .put("concealed", concealed)
            .put("late", late)
        if (oneWayMs != null) o.put("one_way_ms", round3(oneWayMs))
        if (clockPpm != null) o.put("clock_ppm", round3(clockPpm))
        return o.toString()
    }

    /** `{"t":"syncreply",…}` —— 对电脑 `sync` 的 NTP 四时戳应答。三个时间戳都是手机时钟（µs）。 */
    /**
     * `{"t":"syncreply",…}` —— 对电脑 `sync` 的 NTP 四时戳应答。三个时间戳都是手机时钟（µs）。
     *
     * **必须按无符号 32 位发**：µs 计数器回绕后落在 2^31..2^32-1，Android 的 `Int` 是
     * 有符号的，直接 `put` 会写成负数，而电脑侧按 `u32` 解析 → 整条消息解析失败 →
     * 手机上弹红字「电脑：控制消息格式不对」（实测 2026-10-27）。所以统一转成 `Long`
     * 再按 32 位掩码，让 JSON 里出现的永远是非负数。
     */
    fun syncReply(id: Long, t0: Int, t1: Int, t2: Int): String = JSONObject()
        .put("t", "syncreply")
        .put("id", id)
        .put("t0", t0.toLong() and 0xFFFF_FFFFL)
        .put("t1", t1.toLong() and 0xFFFF_FFFFL)
        .put("t2", t2.toLong() and 0xFFFF_FFFFL)
        .toString()

    /** 拼 8 字节帧头（大端）。 */
    fun header(seq: Int, tsUs: Int): ByteArray {
        val b = ByteArray(HDR_BYTES)
        b[0] = (seq ushr 24).toByte()
        b[1] = (seq ushr 16).toByte()
        b[2] = (seq ushr 8).toByte()
        b[3] = seq.toByte()
        b[4] = (tsUs ushr 24).toByte()
        b[5] = (tsUs ushr 16).toByte()
        b[6] = (tsUs ushr 8).toByte()
        b[7] = tsUs.toByte()
        return b
    }

    fun readU32(b: ByteArray, at: Int): Int =
        ((b[at].toInt() and 0xFF) shl 24) or
            ((b[at + 1].toInt() and 0xFF) shl 16) or
            ((b[at + 2].toInt() and 0xFF) shl 8) or
            (b[at + 3].toInt() and 0xFF)

    /**
     * 带符号的 u32 微秒差（跨回绕有效）。
     *
     * u32 微秒 71.6 分钟回绕一次；直接相减会在回绕点变成 ±40 亿。转成 Int 相减即得
     * 「按 2^32 回绕」的带符号差（±35 分钟以内都精确），与 Rust 的 `rt::us_diff` 等价。
     */
    fun usDiff(now: Int, then: Int): Int = now - then

    /** 单调时钟的微秒（帧头用）：`System.nanoTime()` 是单调的，与电脑的 offset 无关。 */
    fun nowUs(): Int = ((System.nanoTime() / 1_000L) and 0xFFFF_FFFFL).toInt()

    private fun round3(v: Double): Double = Math.round(v * 1000.0) / 1000.0
}

/**
 * 电脑 → 手机 的消息（对应 `proto::ServerMsg`）。
 *
 * - [Ready]：握手或 `sub` 后都会收到；**收到之前不播放、不推流**。
 * - [Pong]：`ping` 的回声，RTT 由 [net.WsClient] 计算。
 * - [Sync]：电脑的时钟探测（`t0` = 电脑时钟 µs），必须回 [Proto.syncReply]。
 * - [Offset]：电脑算出的时钟偏移（手机 − 电脑，µs），用来把下行帧头换成单向延迟。
 * - [Err]：电脑的拒绝/故障原文，**必须显示给用户**。
 */
sealed class ServerMsg {
    data class Ready(
        val proto: Int,
        val codec: String,
        val rate: Int,
        val channels: Int,
        val frameMs: Int,
        /** 音频帧头字节数（v3 = 8）。客户端按它切 Opus 载荷。 */
        val headerBytes: Int,
        /** 电脑建议的**手机播放环目标水位**（ms）—— 缓冲只在接收方，目标值跟着算法走。 */
        val targetMs: Int,
        val clients: Int,
    ) : ServerMsg()

    data class Pong(val id: Long) : ServerMsg()

    data class Sync(val id: Long, val t0: Int) : ServerMsg()

    data class Offset(val us: Long) : ServerMsg()

    /** 环深度变了（手动调整，两侧同步）：收到就把自己的环与滑块一起改到这个值。 */
    data class Target(val ms: Int) : ServerMsg()

    data class Err(val msg: String) : ServerMsg()

    /** 非本协议认识的 `t`：不断线，只记日志（协议只增不改，老客户端要能活）。 */
    data class Other(val type: String) : ServerMsg()

    companion object {
        /** 解析失败返回 `Other("!")`，协议错误返回 `Other(t)`；两种都不抛异常。 */
        fun parse(text: String): ServerMsg {
            val raw = text.trim()
            if (!raw.startsWith("{")) return Other("!")
            val o = try {
                JSONObject(raw)
            } catch (e: Exception) {
                return Other("!")
            }
            return when (o.optString("t")) {
                "ready" -> Ready(
                    proto = o.optInt("proto", -1),
                    codec = o.optString("codec"),
                    rate = o.optInt("rate", 0),
                    channels = o.optInt("channels", 0),
                    frameMs = o.optInt("frame_ms", 0),
                    headerBytes = o.optInt("header_bytes", Proto.HDR_BYTES),
                    targetMs = o.optInt("target_ms", 20),
                    clients = o.optInt("clients", 0),
                )
                "pong" -> Pong(o.optLong("id", -1L))
                "sync" -> Sync(o.optLong("id", -1L), o.optInt("t0", 0))
                "offset" -> Offset(o.optLong("us", 0L))
                "target" -> Target(o.optInt("ms", 0))
                "err" -> Err(o.optString("msg"))
                else -> Other(o.optString("t"))
            }
        }
    }
}
