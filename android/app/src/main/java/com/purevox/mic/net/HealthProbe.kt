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

import android.os.Handler
import android.os.Looper
import android.util.Log
import com.purevox.mic.Proto
import okhttp3.Call
import okhttp3.Callback
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import java.io.IOException
import java.util.concurrent.TimeUnit

/**
 * `GET /health` 自检（明文 `http://host:59123/health`）。
 *
 * 电脑端提供这个端点就是给人 / 手机看用的，**不需要**先建立 WebSocket
 * （见 `src-tauri/src/net/server.rs` 的 `health` 路由）。所以它独立于
 * [WsClient]：连不上 ws 时照样能自检，用来区分「地址写错了」和「服务没开」。
 */
object HealthProbe {
    private const val TAG = "PureVoxHealth"

    private val client: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(4, TimeUnit.SECONDS)
            .readTimeout(4, TimeUnit.SECONDS)
            .build()
    }

    /**
     * 回调 `(成功?, 说明)`，在 OkHttp 线程上调用（调用方自己切 UI 线程）。
     */
    fun check(host: String, port: Int = Proto.PORT, cb: (Boolean, String) -> Unit) {
        val request = Request.Builder().url(Proto.healthUrl(host, port)).get().build()
        try {
            client.newCall(request).enqueue(object : Callback {
                override fun onFailure(call: Call, e: IOException) {
                    cb(false, "GET /health 失败：${e.message ?: e.javaClass.simpleName}")
                }

                override fun onResponse(call: Call, response: Response) {
                    val body = try {
                        response.body?.string().orEmpty()
                    } catch (e: IOException) {
                        Log.w(TAG, "读取响应体失败", e)
                        ""
                    }
                    cb(response.isSuccessful, "HTTP ${response.code} · $body")
                }
            })
        } catch (e: Throwable) {
            Log.w(TAG, "发起自检异常", e)
            Handler(Looper.getMainLooper()).post { cb(false, "发起自检异常：${e.message}") }
        }
    }
}