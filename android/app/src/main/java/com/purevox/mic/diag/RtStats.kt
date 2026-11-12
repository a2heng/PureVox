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
import com.purevox.mic.R
import kotlin.math.abs
import kotlin.math.log10

/**
 * 诊断读数的时间序列（纯计算，不碰 Android API、不碰界面）。
 *
 * 为什么单独一个类：这些量都要**相对上一次采样**才能算出来 —— 速率 = 差值 ÷ 间隔，
 * 抖动 = RTT 的前后差。界面每 500 ms 刷一次，混在 Activity 里就得在那儿再养一整套
 * 「上一次的值」；这里只吐一个 [Net] 快照，怎么显示是界面的事。
 *
 * 两个结论**分开报，不许互相冒充**（与桌面端 `/debug` 的 AEC 行同一个道理）：
 * [netVerdict] 只看真正属于网络的量（平均 RTT + 抖动）；
 * [audioVerdict] 只看音频有没有真的在流动（欠载 / 解码掩盖 / 丢包）。
 * 「声音断了」可能是网络、也可能是电脑端没喂上数据，混成一个数字就再也分不出来。
 *
 * 结论标签与依据是**用户可见文案**（诊断面板里的一行），所以走字符串资源，
 * 跟着界面语言变 —— 中文即默认资源，英文在 `values-en/`。
 */
class RtStats(private val context: Context) {

    companion object {
        /** RTT 统计窗口：1 Hz 探针 → 20 s。 */
        const val RTT_WINDOW = 20

        /** 速率采样间隔（界面 2 Hz 调用，内部按这个间隔才真算一次）。 */
        const val SAMPLE_INTERVAL_MS = 1000L

        /** 结论至少要有几个 RTT 样本 —— 少了就先报「采样中」，别把刚连上判成差网络。 */
        const val VERDICT_MIN_SAMPLES = 3

        /** 电平条的量程：-60 dBFS ~ 0 dBFS。 */
        const val LEVEL_FLOOR_DB = -60.0
    }

    /**
     * 一次快照。单位已归一：ms / 包每秒 / kbit 每秒；
     * **没有样本时是 NaN，不是 0** —— 界面据此显示「采样中…」而不是伪造的 0。
     */
    data class Net(
        val rttLast: Double = Double.NaN,
        val rttMin: Double = Double.NaN,
        val rttAvg: Double = Double.NaN,
        val rttMax: Double = Double.NaN,
        val jitter: Double = Double.NaN,
        val rttSamples: Int = 0,
        val upPps: Double = Double.NaN,
        val downPps: Double = Double.NaN,
        val upKbps: Double = Double.NaN,
        val downKbps: Double = Double.NaN,
        val underrunPps: Double = Double.NaN,
        val concealedPps: Double = Double.NaN,
        val droppedPps: Double = Double.NaN,
    ) {
        /** 是否已经算过一次速率（第一秒内是 false）。 */
        val hasRate: Boolean get() = !upPps.isNaN()
    }

    /** 结论：等级（0 最好）+ 短标签 + 依据。 */
    data class Verdict(val level: Int, val label: String, val detail: String)

    private val rttWin = DoubleArray(RTT_WINDOW)
    private var rttCount = 0
    private var rttPos = 0
    private var rttPrev = Double.NaN
    private var rttLast = Double.NaN
    private var jitter = Double.NaN

    private var lastSampleMs = 0L
    private var prevUp = 0L
    private var prevUpBytes = 0L
    private var prevDown = 0L
    private var prevDownBytes = 0L
    private var prevUnderrun = 0L
    private var prevDropped = 0L
    private var prevConcealed = 0L
    private var rates = Net()

    /** 收到一次 pong 的 RTT（1 Hz；由界面在 UI 线程喂进来，所以内部不加锁）。 */
    fun onRtt(ms: Double) {
        if (ms.isNaN() || ms < 0) return
        if (!rttPrev.isNaN()) {
            val d = abs(ms - rttPrev)
            // RFC 3550 的抖动：EWMA(|ΔRTT|)。对单次尖峰不敏感，持续波动才涨得起来
            jitter = if (jitter.isNaN()) d else jitter * 15.0 / 16.0 + d / 16.0
        }
        rttPrev = ms
        rttLast = ms
        rttWin[rttPos] = ms
        rttPos = (rttPos + 1) % RTT_WINDOW
        if (rttCount < RTT_WINDOW) rttCount++
    }

    /**
     * 采样一次累计计数（界面每 500 ms 调一次即可）。
     *
     * 只有跨过 [SAMPLE_INTERVAL_MS] 才真算速率 —— 而且**基线只在算速率时更新**，
     * 否则 1 s 的差值会被拆成「半个 500 ms」算，速率直接翻倍。
     */
    fun sample(
        nowMs: Long,
        upPackets: Long,
        upBytes: Long,
        downPackets: Long,
        downBytes: Long,
        underruns: Long,
        dropped: Long,
        concealed: Long,
    ): Net {
        if (lastSampleMs == 0L) {
            // 第一次只记基线：此时算出来的「速率」会把「连接之前攒的时间」也算进去
            lastSampleMs = nowMs
            rebaseline(upPackets, upBytes, downPackets, downBytes, underruns, dropped, concealed)
        } else if (nowMs - lastSampleMs >= SAMPLE_INTERVAL_MS) {
            val dt = (nowMs - lastSampleMs) / 1000.0
            rates = Net(
                upPps = delta(upPackets, prevUp) / dt,
                downPps = delta(downPackets, prevDown) / dt,
                upKbps = delta(upBytes, prevUpBytes) * 8.0 / 1000.0 / dt,
                downKbps = delta(downBytes, prevDownBytes) * 8.0 / 1000.0 / dt,
                underrunPps = delta(underruns, prevUnderrun) / dt,
                concealedPps = delta(concealed, prevConcealed) / dt,
                droppedPps = delta(dropped, prevDropped) / dt,
            )
            lastSampleMs = nowMs
            rebaseline(upPackets, upBytes, downPackets, downBytes, underruns, dropped, concealed)
        }
        return rates.copy(
            rttLast = rttLast,
            rttMin = rttMin(),
            rttAvg = rttAvg(),
            rttMax = rttMax(),
            jitter = jitter,
            rttSamples = rttCount,
        )
    }

    /**
     * 网络质量：平均 RTT 与抖动的较差者决定等级，依据里写清是哪一项拖的后腿。
     *
     * [connected] = false 时报「未连接」而不是拿空样本硬算出「优」——没连上就没有网络质量可言。
     */
    fun netVerdict(net: Net, connected: Boolean): Verdict {
        if (!connected) {
            return Verdict(
                0,
                context.getString(R.string.verdict_not_connected),
                context.getString(R.string.verdict_not_connected_detail),
            )
        }
        if (net.rttSamples < VERDICT_MIN_SAMPLES) {
            return Verdict(
                0,
                context.getString(R.string.verdict_sampling),
                context.getString(R.string.verdict_rtt_samples, net.rttSamples, VERDICT_MIN_SAMPLES),
            )
        }
        val reasons = ArrayList<String>(2)
        var level = when {
            net.rttAvg < 60 -> 0
            net.rttAvg < 150 -> 1
            net.rttAvg < 300 -> 2
            else -> 3
        }
        if (level > 0) reasons += context.getString(R.string.verdict_latency, fmt(net.rttAvg))

        val jl = when {
            net.jitter < 15 -> 0
            net.jitter < 40 -> 1
            net.jitter < 100 -> 2
            else -> 3
        }
        if (jl > 0) {
            reasons += context.getString(R.string.verdict_jitter, fmt(net.jitter))
            if (jl > level) level = jl
        }
        val label = context.getString(
            when (level) {
                0 -> R.string.verdict_good
                1 -> R.string.verdict_fair
                2 -> R.string.verdict_weak
                else -> R.string.verdict_poor
            },
        )
        return Verdict(
            level,
            label,
            if (reasons.isEmpty()) {
                context.getString(R.string.verdict_net_ok)
            } else {
                reasons.joinToString(context.getString(R.string.list_sep))
            },
        )
    }

    /**
     * 音频是否真的在流动：欠载（补静音）+ 解码掩盖（丢包补帧）+ 丢包，按每秒计。
     *
     * [subscribed] = false 时报「未开启」——这三个计数全都属于**下行**，
     * 没订阅电脑音频时它们恒为 0，直接算出来就会谎报「正常」。
     */
    fun audioVerdict(net: Net, subscribed: Boolean): Verdict {
        if (!subscribed) {
            return Verdict(
                0,
                context.getString(R.string.diag_off),
                context.getString(R.string.verdict_audio_off_detail),
            )
        }
        if (!net.hasRate) {
            return Verdict(
                0,
                context.getString(R.string.verdict_sampling),
                context.getString(R.string.verdict_no_rate_sample),
            )
        }
        val bad = net.underrunPps + net.concealedPps + net.droppedPps
        val detail = buildString {
            append(context.getString(R.string.verdict_underrun, fmt(net.underrunPps)))
            append(" · ")
            append(context.getString(R.string.verdict_concealed, fmt(net.concealedPps)))
            append(" · ")
            append(context.getString(R.string.verdict_dropped, fmt(net.droppedPps)))
        }
        return when {
            bad <= 0.0 -> Verdict(
                0,
                context.getString(R.string.verdict_normal),
                context.getString(R.string.verdict_normal_detail),
            )

            bad < 1.0 -> Verdict(1, context.getString(R.string.verdict_occasional), detail)
            else -> Verdict(2, context.getString(R.string.verdict_broken), detail)
        }
    }

    /** 峰值电平（0..1）→ 电平条百分比（-60 dBFS 以下算 0；浮点电平本身是线性峰值）。 */
    fun levelPercent(level: Float): Int {
        if (level <= 0f) return 0
        val db = 20.0 * log10(level.toDouble())
        val pct = (db - LEVEL_FLOOR_DB) / (0.0 - LEVEL_FLOOR_DB) * 100.0
        return pct.coerceIn(0.0, 100.0).toInt()
    }

    // ---------------------------------------------------------------- 内部

    private fun rebaseline(
        upPackets: Long,
        upBytes: Long,
        downPackets: Long,
        downBytes: Long,
        underruns: Long,
        dropped: Long,
        concealed: Long,
    ) {
        prevUp = upPackets
        prevUpBytes = upBytes
        prevDown = downPackets
        prevDownBytes = downBytes
        prevUnderrun = underruns
        prevDropped = dropped
        prevConcealed = concealed
    }

    /** 计数只会涨；变小说明那条链路重启过（上行停/开），这一拍按 0 算，不报负数。 */
    private fun delta(now: Long, prev: Long): Long = (now - prev).coerceAtLeast(0L)

    private fun rttMin(): Double = if (rttCount == 0) Double.NaN else (0 until rttCount).minOf { rttWin[it] }

    private fun rttMax(): Double = if (rttCount == 0) Double.NaN else (0 until rttCount).maxOf { rttWin[it] }

    private fun rttAvg(): Double = if (rttCount == 0) Double.NaN else (0 until rttCount).sumOf { rttWin[it] } / rttCount

    private fun fmt(v: Double): String = if (v.isNaN()) "—" else String.format("%.1f", v)
}
