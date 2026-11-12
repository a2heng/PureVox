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

package com.purevox.mic.update

import android.content.Context
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.TimeUnit

/**
 * 下载 release 里的 APK 到**应用私有外部目录**（`Android/data/<pkg>/files/update/`），
 * 再用 [UpdateInstaller] 交给系统安装器。
 *
 * 落私有目录（而不是公共 Download）有两个好处：不需要存储权限，且随应用卸载一起清掉。
 * 每次下载前**清掉同目录的旧文件** —— 那是上一版残留，留着只占空间。
 *
 * 回调在 OkHttp 线程，调用方切主线程再动 UI。
 */
object UpdateDownloader {

    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(30, TimeUnit.SECONDS)
        .build()

    /**
     * @param onProgress 已下载百分比（0..100；拿不到 Content-Length 时不回调）
     * @param onDone     (文件, 错误原因)；成功时错误为 `null`
     */
    fun download(
        ctx: Context,
        release: UpdateChecker.ReleaseInfo,
        onProgress: (Int) -> Unit,
        onDone: (File?, String?) -> Unit,
    ) {
        val dir = File(ctx.getExternalFilesDir(null), "update")
        if (!dir.exists() && !dir.mkdirs()) {
            onDone(null, ctx.getString(com.purevox.mic.R.string.err_download_dir, dir.absolutePath))
            return
        }
        dir.listFiles()?.forEach { it.delete() }

        val target = File(dir, release.apkName)
        val req = Request.Builder()
            .url(release.apkUrl)
            .header("User-Agent", "PureVox-Android")
            .get()
            .build()

        client.newCall(req).enqueue(object : okhttp3.Callback {
            override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                onDone(null, e.message ?: e.javaClass.simpleName)
            }

            override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                response.use { r ->
                    if (!r.isSuccessful) {
                        onDone(null, "HTTP ${r.code}")
                        return
                    }
                    // OkHttp 5 的 body 不再可空；空响应由 contentLength = -1 / 0 字节体现
                    val body = r.body
                    val total = body.contentLength()
                    try {
                        body.byteStream().use { input ->
                            FileOutputStream(target).use { output ->
                                val buf = ByteArray(64 * 1024)
                                var sum = 0L
                                var read = input.read(buf)
                                while (read > 0) {
                                    output.write(buf, 0, read)
                                    sum += read
                                    if (total > 0) onProgress(((sum * 100) / total).toInt())
                                    read = input.read(buf)
                                }
                            }
                        }
                        onDone(target, null)
                    } catch (e: Exception) {
                        target.delete()
                        onDone(null, e.message ?: e.javaClass.simpleName)
                    }
                }
            }
        })
    }
}
