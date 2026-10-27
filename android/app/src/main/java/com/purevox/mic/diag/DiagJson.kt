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

package com.purevox.mic.diag

import android.content.Context
import android.util.Log
import java.io.File

/**
 * 结构化诊断的落盘 + 日志出口（AGENTS §1.4：**可复现命令 + 机器可读 JSON**）。
 *
 * 界面上的「诊断」页是给人看的，`复制诊断` 贴到聊天窗口也是给人看的；这两样机器都读不了
 * （截图更不能当测量依据）。所以这里再给一份**同一份数据**的 JSON：
 *
 *  - 落盘 `filesDir/diag.json`（每秒覆盖写）：`adb shell run-as <包名> cat files/diag.json`
 *    一条命令就能取，不需要 root、不需要装 apk 解包工具；
 *  - 同时打一行 `Log.i("PureVoxDiag", json)`：`adb logcat -s PureVoxDiag:I` 就能连续观察，
 *    连 run-as 都不用（`adb logcat` 任何机器上都能用）。
 *
 * 两条出口写的是**同一份字符串**，字段名与 `/debug` 的 `net` 段一致（`level_ms` /
 * `target_ms` / `late` / `underruns` / `clock_ppm` …），这样电脑侧、手机侧、脚本读的是
 * 同一套名字，不会出现「同一个量三个叫法」。
 */
object DiagJson {

    /** 字段只增不改名；改结构必须 +1。 */
    const val SCHEMA_VERSION = 1

    const val TAG = "PureVoxDiag"

    private const val FILE_NAME = "diag.json"

    /** 上一次写盘的时刻（限流用：ticker 是 2 Hz，落盘与日志各 1 Hz 就够）。 */
    @Volatile
    private var lastWriteMs = 0L

    /**
     * 写入一次（带限流：`minIntervalMs` 内只写一次）。
     *
     * 失败**不抛**（诊断不能反过来把界面搞崩），但会把原因记进返回串，调用方可以 `log`。
     */
    fun publish(ctx: Context, json: String, nowMs: Long, minIntervalMs: Long = 1000L): String? {
        if (nowMs - lastWriteMs < minIntervalMs) return null
        lastWriteMs = nowMs
        Log.i(TAG, json)
        return try {
            val f = File(ctx.filesDir, FILE_NAME)
            val tmp = File(ctx.filesDir, "$FILE_NAME.tmp")
            tmp.writeText(json)
            // 覆盖写用 rename：读的一方永远看到完整 JSON（不会读到写了一半的半截）
            if (!tmp.renameTo(f)) {
                f.writeText(json)
                tmp.delete()
            }
            null
        } catch (e: Throwable) {
            "诊断落盘失败：${e.message ?: e.javaClass.simpleName}"
        }
    }

    /** 诊断文件的绝对路径（界面/日志里显示用）。 */
    fun path(ctx: Context): String = File(ctx.filesDir, FILE_NAME).absolutePath
}
