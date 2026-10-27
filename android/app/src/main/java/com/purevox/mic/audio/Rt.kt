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

package com.purevox.mic.audio

import com.purevox.mic.Proto

/**
 * 下行播放环（电脑 → 手机）：**整条链上唯一的缓冲**。
 *
 * 这是 `src-tauri/src/net/rt.rs` 的 `Playout` 的**逐行镜像**：阈值与策略必须与那边一致。
 *
 * ## 策略只有两条（极简）
 *
 * 1. 排队超过**实时性上限**（滑块，1 帧 = 10 ms）→ 直接丢最旧的，不做任何平滑；
 * 2. 取数时不够一格 → **输出静音**（残留整块丢掉），不做 priming、不做 PLC、不做淡出。
 *
 * 音频与别的数据不同：**晚到的样本没有价值**。所以这里没有目标水位、没有容差、
 * 没有控制回路，只有一个用户可以拧的上限。
 *
 * ## 为什么这么小
 *
 * 实测：设备晶振差 **±25 ppm**、局域网到达抖动 **0.4~0.9 ms**、电脑 10 ms 节拍误差 **0.0%**。
 * 所以默认深度 [RING_MS] = 20 ms，运行期只有两种动作：超了丢一格（10 ms）、
 * 空了补一格（PLC / 淡出）。**没有重采样、没有 PI 伺服、没有抖动驱动的目标水位** ——
 * 前两版都死在控制回路上（服务器侧实测摆出 ±1500 ppm 颤音、起播被误判成 25 次欠载）。
 *
 * **深度可手动调**（10~200 ms、10 ms 步进，[setTargetMs]）：那是用户拧的旋钮，
 * 不是自适应 —— 值不会因为抖动/欠载自己动，两侧界面与环同步同一个值。
 *
 * 代价：25 ppm 的漂移约 **13 分钟**修正一格。换回来的是音频**逐样本原样**（不变调、不颤）。
 */
class PlaybackRing {

    companion object {
        private const val SR = Proto.SAMPLE_RATE
        private const val HOP = Proto.FRAME_SAMPLES

        /** 默认实时性上限（ms）。**可手动调**（滑块，1 帧 = 10 ms）。 */
        const val RING_MS = 20.0

        const val MIN_MS = 10.0
        const val MAX_MS = 300.0
        const val STEP_MS = 10.0

        /** 容量按上限算（300 ms + 一格）。 */
        private const val CAP = (SR / 1000) * (MAX_MS.toInt() + 10)
    }

    private val buf = FloatArray(CAP)
    private var head = 0
    private var count = 0

    /**
     * 实时性上限（ms）：**唯一的控制**。
     *
     * 音频只看实时性 —— 数据不重要、晚到的样本没有价值。所以这里只有一个数：
     * 排队超过它就**直接丢最旧的**，不做任何平滑。
     */
    @Volatile
    var maxMs: Double = RING_MS
        private set

    // ---- 到达统计（只观测） ----
    private var lastArrivalNs = 0L
    var packets = 0L
        private set
    var hops = 0L
        private set
    var underruns = 0L
        private set
    var trims = 0L
        private set
    var dropped = 0L
        private set
    var late = 0L
        private set
    var maxGapMs = 0.0
        private set

    fun levelMs(): Double = count * 1000.0 / SR
    fun targetMs(): Double = maxMs
    fun gapOverMs(ms: Double): Long = if (maxGapMs > ms) 1 else 0

    /** 改实时性上限（手动）。变小时立刻按新上限丢最旧的。 */
    fun setMaxMs(ms: Double) {
        maxMs = (ms.coerceIn(MIN_MS, MAX_MS) / STEP_MS).toInt() * STEP_MS
        trimToCap()
    }

    private fun trimToCap() {
        val cap = (maxMs * SR / 1000.0).toInt()
        if (count > cap) {
            val drop = count - cap
            head = (head + drop) % CAP
            count -= drop
            trims++
            dropped += drop
        }
    }

    /**
     * 收到解码后的样本。**只看实时性**：收下、超过上限就丢最旧的。
     * `plc = true` 表示本机补的帧（现在不会有了，字段保留兼容）。
     */
    fun push(samples: FloatArray, n: Int, nowNs: Long, plc: Boolean = false) {
        if (n <= 0) return
        if (!plc) {
            packets++
            if (lastArrivalNs != 0L) {
                val dtMs = (nowNs - lastArrivalNs) / 1_000_000.0
                if (dtMs > maxGapMs) maxGapMs = dtMs
            }
            lastArrivalNs = nowNs
        }
        for (i in 0 until n) {
            buf[(head + count) % CAP] = samples[i]
            count++
        }
        trimToCap()
    }

    /**
     * 按设备节拍取一格：**有就播、不够就静音**。
     *
     * 不做 priming（不垫深度）、不做 PLC、不做淡出 —— 音频只看实时性，
     * 晚到的样本没有价值，补出来的东西也一样：宁可静音一下，也不要把时间轴往后拖。
     * 返回 `true` = 真实样本；`false` = 这一拍是静音。
     */
    fun pull(out: FloatArray, n: Int): Boolean {
        if (count >= n) {
            var idx = head
            for (i in 0 until n) {
                out[i] = buf[idx]
                idx++
                if (idx == CAP) idx = 0
            }
            head = idx
            count -= n
            hops++
            return true
        }
        // 断流：**整块丢掉**残留（它们已经过时了），输出静音
        underruns++
        dropped += count.toLong()
        head = 0
        count = 0
        out.fill(0f, 0, n)
        return false
    }

    fun notePlc() {} // 保留签名：PLC 已经取消，调用方不需要改

    fun reset() {
        head = 0
        count = 0
        lastArrivalNs = 0L
        maxGapMs = 0.0
        packets = 0
        hops = 0
        underruns = 0
        trims = 0
        dropped = 0
        late = 0
    }
}
