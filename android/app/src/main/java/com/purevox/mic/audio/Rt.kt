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
 * 这是 `src-tauri/src/net/rt.rs` 的 `Playout` 的**逐行镜像**：深度、阈值、起播规则、
 * 欠载策略必须与那边逐字一致（那边有同样的注释与单元测试）。
 *
 * ## 为什么这么小
 *
 * 实测：设备晶振差 **±25 ppm**、局域网到达抖动 **0.4~0.9 ms**、电脑 10 ms 节拍误差 **0.0%**。
 * 所以深度是**常量** [RING_MS] = 20 ms，运行期只有两种动作：超了丢一格（10 ms）、
 * 空了补一格（PLC / 淡出）。**没有重采样、没有 PI 伺服、没有抖动驱动的目标水位** ——
 * 前两版都死在控制回路上（服务器侧实测摆出 ±1500 ppm 颤音、起播被误判成 25 次欠载）。
 *
 * 代价：25 ppm 的漂移约 **13 分钟**修正一格。换回来的是音频**逐样本原样**（不变调、不颤）。
 */
class PlaybackRing {

    companion object {
        private const val SR = Proto.SAMPLE_RATE
        private const val HOP = Proto.FRAME_SAMPLES

        /** 环深度（ms）——与 `rt.rs` 的 `RING_MS` 同值，**改一处必须改三处**（含网页 worklet）。 */
        const val RING_MS = 20.0

        /** 溢出阈值：超过「深度 + 一格」就裁掉最旧一格。 */
        private const val TRIM_MS = RING_MS + 10.0

        /** 容量：深度 + 一格 + 余量。 */
        private const val CAP = (SR / 1000) * (RING_MS.toInt() + 30)
    }

    private val buf = FloatArray(CAP)
    private var head = 0
    private var count = 0

    /** 上一格输出（欠载时淡出重复）。 */
    private val lastHop = FloatArray(HOP)

    private var primed = false
    private var underrunStreak = 0

    // ---- 到达统计（**只观测，不参与控制**）----
    private var lastArrivalNs = 0L
    private var jitterEmaMs = 0.0
    private var worstJitterMs = 0.0
    private var maxGapMs = 0.0

    var packets = 0L
        private set
    var hops = 0L
        private set
    var underruns = 0L
        private set
    var plc = 0L
        private set
    var trims = 0L
        private set
    var dropped = 0L
        private set
    var late = 0L
        private set

    fun levelMs(): Double = count * 1000.0 / SR
    fun targetMs(): Double = RING_MS
    fun jitterMs(): Double = jitterEmaMs
    fun worstJitterMs(): Double = worstJitterMs
    fun maxGapMs(): Double = maxGapMs

    /**
     * 收到解码后的样本。`nowNs` = `System.nanoTime()`。
     *
     * 环当时是空的就说明这一包**来晚了**（播放已经在补音），计入 `late`。
     */
    fun push(samples: FloatArray, n: Int, nowNs: Long) {
        if (n <= 0) return
        packets++
        // 「来晚」= 已经起播却仍然空环；起播填充期不算
        if (primed && count == 0) late++
        if (lastArrivalNs != 0L) {
            val dtMs = (nowNs - lastArrivalNs) / 1_000_000.0
            maxGapMs = Math.max(maxGapMs, dtMs)
            if (dtMs in 0.0..500.0) {
                val dev = Math.abs(dtMs - HOP * 1000.0 / SR)
                jitterEmaMs += 0.02 * (dev - jitterEmaMs)
                if (dev > worstJitterMs) worstJitterMs = dev
            }
        }
        lastArrivalNs = nowNs

        for (i in 0 until n) {
            buf[(head + count) % CAP] = samples[i]
            count++
        }
        // 溢出：只保留「深度 + 一格」以内，超了裁到深度
        val cap = (TRIM_MS * SR / 1000.0).toInt()
        if (count > cap) {
            val keep = (RING_MS * SR / 1000.0).toInt()
            val drop = count - keep
            head = (head + drop) % CAP
            count -= drop
            trims++
            dropped += drop
        }
    }

    /**
     * 按播放节拍取 `n` 个样本（**总是填满**：不够就淡出重复上一格）。
     *
     * 返回 `true` = 真实样本；`false` = 欠载（调用方可以再补一帧 Opus PLC）。
     */
    fun pull(out: FloatArray, n: Int): Boolean {
        if (!primed) {
            // 起播 = 攒满**深度**（20 ms）：只在起播这一刻做一次，让稳态水位落在深度上
            if (count >= (RING_MS * SR / 1000.0).toInt()) {
                primed = true
            } else {
                out.fill(0f, 0, n)
                return false
            }
        }
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
            underrunStreak = 0
            if (n == HOP) System.arraycopy(out, 0, lastHop, 0, HOP)
            return true
        }
        // 欠载：淡出重复上一格
        underruns++
        underrunStreak++
        val total = Math.max(1, n).toFloat()
        for (i in 0 until n) {
            out[i] = lastHop[i % HOP] * 0.5f * (1f - i / total)
        }
        if (underrunStreak > 100) {
            // 断流超过 1 s：退回未起播，等真正有数据再说
            primed = false
            underrunStreak = 0
            lastArrivalNs = 0
        }
        return false
    }

    /** 记一次「补了一帧 Opus PLC」（计数用）。 */
    fun notePlc() {
        plc++
    }

    fun reset() {
        head = 0
        count = 0
        primed = false
        underrunStreak = 0
        lastArrivalNs = 0L
        jitterEmaMs = 0.0
        worstJitterMs = 0.0
        maxGapMs = 0.0
        packets = 0
        hops = 0
        underruns = 0
        plc = 0
        trims = 0
        dropped = 0
        late = 0
        lastHop.fill(0f)
    }
}
