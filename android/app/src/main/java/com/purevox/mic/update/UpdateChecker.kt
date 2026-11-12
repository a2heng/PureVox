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
import android.os.Build
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * 应用内更新：**唯一数据源 = 本仓库 GitHub Releases**。
 *
 * **安卓与桌面是两条独立的发布通道**（见 `RELEASE-CHANNELS.md`）：
 * - 桌面 tag：`v<yyyy.MM.dd.HHmm>`；
 * - 安卓 tag：`android-v<yyyy.MM.dd.HHmm>`。
 *
 * 这里只认 `android-v*`：用 `/releases/latest` 会拿到「全仓库最新」，桌面一发版，
 * 手机上就会出现一个装不了的「新版本」。所以改成拉一页 release 列表自己筛，
 * 既不看桌面 release，也不受它们的时间顺序影响。
 */
object UpdateChecker {

    /** 唯一数据源。与电脑端 `update::REPO` 一致。 */
    private const val REPO = "a2heng/PureVox"
    private const val API = "https://api.github.com/repos/$REPO/releases?per_page=30"

    /** 安卓发布通道的 tag 前缀。**桌面是 `v`，不要混用**。 */
    private const val TAG_PREFIX = "android-v"

    /**
     * 没有版本时间戳的构建（本机 `assembleRelease` 默认 versionCode=1）。
     *
     * 注意 **2026-10-17 及更早的 CI 包也叫 1**（当时的 `ACODE` 十位数溢出被静默丢成 1）：
     * 那些包会一直报「开发构建」，只能手动装下一版 —— 修好之后的包 versionCode 正常。
     */
    private const val DEV_VERSION_CODE = 1L

    /** versionCode 上限（Play 的硬限值，也确保 Int32 装得下；与 `version.py` 同值）。 */
    private const val MAX_VERSION_CODE = 2_100_000_000L

    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(20, TimeUnit.SECONDS)
        .build()

    /** 一个 release 里我们关心的字段。 */
    data class ReleaseInfo(
        val tag: String,
        val versionCode: Int,
        val displayVersion: String,
        val apkName: String,
        val apkUrl: String,
        val notes: String,
    )

    /** 检查结果。三种「成功」之间意义不同，别混成一团。 */
    sealed class UpdateResult {
        /** 有新版本，可下载。 */
        data class Available(val release: ReleaseInfo, val installedCode: Long) : UpdateResult()

        /** 已是最新。 */
        data class UpToDate(val installedCode: Long) : UpdateResult()

        /** 开发构建，没有时间戳可比。 */
        data class DevBuild(val installedCode: Long) : UpdateResult()

        /** 网络 / 解析失败（原文显示给用户，不静默）。 */
        data class Failed(val reason: String) : UpdateResult()
    }

    /** 本机安装包的 `versionCode`（长整型，避开历史 32 位截断）。 */
    fun installedCode(ctx: Context): Long {
        val pi = ctx.packageManager.getPackageInfo(ctx.packageName, 0)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            pi.longVersionCode
        } else {
            @Suppress("DEPRECATION")
            pi.versionCode.toLong()
        }
    }

    /** 本机显示版本名（`26.10.8`）。 */
    fun installedName(ctx: Context): String {
        val pi = ctx.packageManager.getPackageInfo(ctx.packageName, 0)
        return pi.versionName ?: "?"
    }

    /**
     * `v2026.10.08.1430` → `261008143`（= `ACODE`）。格式不对 / 越界返回 `null`。
     *
     * 与 `version.py` **逐字同构**：各字段**定长**（月/日/时分补零）后按十进权位拼
     * `yyMMddHHmm`，再整除 10。定长靠**乘 10 的幂**实现，**不用 `String.format`** ——
     * 后者按**默认 locale** 出数字，阿拉伯语等 locale 给的是非 ASCII 数字，`.toLong()`
     * 直接抛异常（旧代码同样踩这个坑，只是表现为返回 null 的静默失败）。
     * 去前导零拼（`26` + `10` + `8` + `1430`）同样错：位数不定，与安装包的值不可比。
     *
     * **÷10 不能省**：十位的 `2610172359`（2.6e9）超过 versionCode 的 Int32 上限
     * 2147483647 —— 溢出成 `null` 的表现是**整条链静默失效**：Gradle 侧退回 `versionCode=1`，
     * 应用侧认不出 release（报「没有可用的 APK 资产」），2026-10-17 实测踩到。÷10 后是
     * 10 分钟粒度、单调、上限 991231235，与 `version.py` 一致。
     */
    fun stampToCode(tag: String): Int? {
        val t = tag.trim().removePrefix(TAG_PREFIX).removePrefix("android-").removePrefix("v")
        val parts = t.split(".")
        if (parts.size != 4) return null
        val y = parts[0].toIntOrNull() ?: return null
        val mo = parts[1].toIntOrNull() ?: return null
        val d = parts[2].toIntOrNull() ?: return null
        val hm = parts[3].toIntOrNull() ?: return null
        if (mo !in 1..12 || d !in 1..31 || hm !in 0..2359) return null
        if (hm % 100 > 59) return null // 2359 合法，2360 不合法
        val yy = y % 100
        // yy(2) · MM(2) · dd(2) · HHmm(4) → 去掉最后一位
        val code = yy * 10_000_000L + mo * 100_000L + d * 1_000L + hm / 10
        if (code !in 1..MAX_VERSION_CODE) return null
        return code.toInt()
    }

    /** `android-v2026.10.08.1430` → `26.10.8`（显示版本，去前导零，与 `version.py` 的 VERSION 同构）。 */
    fun stampToVersion(tag: String): String? {
        val t = tag.trim().removePrefix(TAG_PREFIX).removePrefix("android-").removePrefix("v")
        val parts = t.split(".")
        if (parts.size != 4) return null
        val y = parts[0].toIntOrNull() ?: return null
        val mo = parts[1].toIntOrNull() ?: return null
        val d = parts[2].toIntOrNull() ?: return null
        return "${y % 100}.$mo.$d"
    }

    /**
     * 检查更新。**回调在 OkHttp 线程**，调用方负责切回主线程再动 UI。
     * 手动触发（界面按钮），不自动检查 —— 避免每次启动都打一次 GitHub API。
     */
    fun check(ctx: Context, onDone: (UpdateResult) -> Unit) {
        val installed = installedCode(ctx)
        if (installed <= DEV_VERSION_CODE) {
            onDone(UpdateResult.DevBuild(installed))
            return
        }
        val req = Request.Builder()
            .url(API)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "PureVox-Android")
            .get()
            .build()
        client.newCall(req).enqueue(object : okhttp3.Callback {
            override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                onDone(UpdateResult.Failed(e.message ?: e.javaClass.simpleName))
            }

            override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                response.use { r ->
                    if (!r.isSuccessful) {
                        onDone(UpdateResult.Failed("HTTP ${r.code}"))
                        return
                    }
                    val body = r.body?.string()
                    if (body.isNullOrBlank()) {
                        onDone(UpdateResult.Failed(ctx.getString(com.purevox.mic.R.string.err_update_empty_response)))
                        return
                    }
                    val info = try {
                        parseReleases(body)
                    } catch (e: Exception) {
                        onDone(
                            UpdateResult.Failed(
                                ctx.getString(
                                    com.purevox.mic.R.string.err_update_parse,
                                    e.message ?: e.javaClass.simpleName,
                                ),
                            ),
                        )
                        return
                    }
                    if (info == null) {
                        onDone(UpdateResult.Failed(ctx.getString(com.purevox.mic.R.string.err_update_no_release)))
                        return
                    }
                    onDone(
                        if (info.versionCode.toLong() > installed) {
                            UpdateResult.Available(info, installed)
                        } else {
                            UpdateResult.UpToDate(installed)
                        },
                    )
                }
            }
        })
    }

    /**
     * 从 `/releases?per_page=30` 的 JSON 数组里，只挑 **`android-v*`** 的 release，
     * 取 versionCode 最大的那条，再取其中第一个 `.apk` 资产；没有返回 `null`。
     *
     * 不能直接用数组第一条：桌面 release 也会出现在同一个列表里，而且可能更新。
     * 也不能混用桌面 tag —— 桌面包的手机装不上，装了也连不上同一个更新通道。
     */
    private fun parseReleases(body: String): ReleaseInfo? {
        val arr = JSONArray(body)
        var best: ReleaseInfo? = null
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i) ?: continue
            val tag = o.optString("tag_name")
            if (!tag.startsWith(TAG_PREFIX)) continue
            val code = stampToCode(tag) ?: continue
            val ver = stampToVersion(tag) ?: continue
            val assets = o.optJSONArray("assets") ?: continue
            var apkName = ""
            var apkUrl = ""
            for (j in 0 until assets.length()) {
                val a = assets.optJSONObject(j) ?: continue
                val name = a.optString("name")
                if (name.endsWith(".apk", ignoreCase = true)) {
                    apkName = name
                    apkUrl = a.optString("browser_download_url")
                    break
                }
            }
            if (apkName.isEmpty() || apkUrl.isEmpty()) continue
            if (best == null || code > best.versionCode) {
                best = ReleaseInfo(
                    tag = tag,
                    versionCode = code,
                    displayVersion = ver,
                    apkName = apkName,
                    apkUrl = apkUrl,
                    notes = o.optString("body"),
                )
            }
        }
        return best
    }
}
