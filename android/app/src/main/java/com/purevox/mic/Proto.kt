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
 * 这里只做「拼 JSON / 解 JSON」，不含任何状态 —— 状态都在 `net/WsClient.kt` 与
 * `MainActivity.kt`。字段只增不改，与电脑端保持一致。
 *
 * 传输：明文 `ws://`（AndroidManifest 已开 `usesCleartextTraffic`），
 * 二进制帧 = 一个 Opus 包（无头），文本帧 = 本文件的消息。
 */
object Proto {
    /** 协议版本；必须等于电脑端的 `proto::PROTO`，否则电脑回 `err` 并断开。 */
    const val VERSION = 1

    /** 端口与路径（沿用旧实现）。 */
    const val PORT = 59123
    const val WS_PATH = "/ws"
    const val HEALTH_PATH = "/health"

    /** 音频参数（协议硬约定，改动要同步电脑端）。 */
    const val CODEC = "audio/opus"
    const val SAMPLE_RATE = 48000
    const val CHANNELS = 1

    /** 上行建议码率（bit/s）；电脑端出站也是 32 k。 */
    const val BITRATE = 32000

    fun wsUrl(host: String, port: Int = PORT): String = "ws://$host:$port$WS_PATH"

    fun healthUrl(host: String, port: Int = PORT): String = "http://$host:$port$HEALTH_PATH"

    /** `{"t":"hello","proto":1}` —— 连上必须先握手，电脑回 `ready` 才算可用。 */
    fun hello(): String = JSONObject().put("t", "hello").put("proto", VERSION).toString()

    /** `{"t":"sub"}` —— 订阅电脑音频（电脑当手机音箱）。 */
    fun sub(): String = JSONObject().put("t", "sub").toString()

    /** `{"t":"text","s":"..."}` —— 手机输入法**已组合**好的文本。 */
    fun text(s: String): String = JSONObject().put("t", "text").put("s", s).toString()

    /** `{"t":"key","code":29,"down":true}` —— `code` 就是 Android `KeyEvent.KEYCODE_*`。 */
    fun key(code: Int, down: Boolean): String =
        JSONObject().put("t", "key").put("code", code).put("down", down).toString()

    /** `{"t":"ping","id":42}` —— 电脑原样回 `{"t":"pong","id":42}`。 */
    fun ping(id: Long): String = JSONObject().put("t", "ping").put("id", id).toString()

    /** `{"t":"rtt","ms":7.5}` —— 时延由手机算（它才是发 ping 的一方），上报给电脑调试界面。 */
    fun rtt(ms: Double): String = JSONObject().put("t", "rtt").put("ms", ms).toString()
}

/**
 * 电脑 → 手机 的消息（对应 `proto::ServerMsg`）。
 *
 * - [Ready]：握手或 `sub` 后都会收到；**收到之前不播放、不推流**。
 * - [Pong]：`ping` 的回声，RTT 由 [net.WsClient] 计算。
 * - [Err]：电脑的拒绝/故障原文（例如「远程输入未开启（界面开关）」），**必须显示给用户**。
 */
sealed class ServerMsg {
    data class Ready(
        val proto: Int,
        val codec: String,
        val rate: Int,
        val channels: Int,
        val frameMs: Int,
        val clients: Int,
    ) : ServerMsg()

    data class Pong(val id: Long) : ServerMsg()

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
                    clients = o.optInt("clients", 0),
                )
                "pong" -> Pong(o.optLong("id", -1L))
                "err" -> Err(o.optString("msg"))
                else -> Other(o.optString("t"))
            }
        }
    }
}