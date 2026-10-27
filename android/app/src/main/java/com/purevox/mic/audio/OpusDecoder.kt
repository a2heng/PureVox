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

import android.media.AudioFormat
import android.util.Log

/**
 * 下行 Opus 解码器（电脑音频 → 手机），**libopus 1.5.2**（见 [OpusNative] 为什么不用系统 codec）。
 *
 * ## 一包 ≠ 固定帧长
 *
 * 电脑端按 10 ms 发包（480 样本），但别的客户端可能发 2.5~60 ms 任意帧长。所以**不假设
 * 输出长度**：每次 [decode] 按 libopus 实际解出的样本数回调给播放层，由播放层累积进环形缓冲。
 * JNI 的输出缓冲按 Opus 单帧上限（120 ms）给足，避免帧长比预期长时被截断。
 *
 * ## 丢包用 PLC，不要静音
 *
 * 包解不出来（真丢包）时传 null 走 libopus 的 **PLC**：它按上一帧的频谱合成补偿，
 * 比直接补静音自然得多（尤其低码率语音，静音的间隔非常刺耳）。
 *
 * 调用线程：只有播放层的解码线程会调 [decode]，本类内部无锁。
 */
class OpusDecoder {

    companion object {
        private const val TAG = "PureVoxDec"

        /** 单包上限，与 [OpusEncoder.MAX_PACKET] 对齐。 */
        const val MAX_PACKET = 4000

        /**
         * 解码输出缓冲的样本数：Opus 单帧最长 120 ms @48 kHz = 5760。
         * 用上限而不是 480 是因为**帧长由发包方决定**，按 480 开缓冲会把长帧截断（听起来是"咔"一声）。
         */
        private const val SCRATCH_SAMPLES = 5760
    }

    /** 实际生效的解码器名。 */
    var codecName: String = "libopus"
        private set

    /** 丢弃的包数（理论极少；统计出来是因为「看不见的状态等于不存在的状态」）。 */
    @Volatile
    var dropped: Long = 0L
        private set

    /** 走 PLC 补偿的次数：正常应为 0，持续非 0 说明网络在抖。界面线程会读，故 volatile。 */
    @Volatile
    var concealed: Long = 0L
        private set

    private var handle: Long = 0
    private val scratch = FloatArray(SCRATCH_SAMPLES)
    private var onPcm: ((FloatArray, Int, Boolean) -> Unit)? = null

    /** 建好解码器。成功返回 [codecName]，失败抛 [IllegalStateException]（消息已含原因）。 */
    fun start(onPcm: (FloatArray, Int, Boolean) -> Unit): String {
        stop()
        OpusNative.require()
        this.onPcm = onPcm
        val h = OpusNative.decCreate(OpusEncoder.SAMPLE_RATE, OpusEncoder.CHANNELS)
        if (h == 0L) {
            val why = "${OpusNative.lastError()}（code ${OpusNative.lastErrorCode()}）"
            throw IllegalStateException("Opus 解码器创建失败：$why")
        }
        handle = h
        codecName = "libopus ${OpusNative.version}"
        Log.i(TAG, "解码器就绪：$codecName  输出缓冲 ${SCRATCH_SAMPLES} 样本（120 ms）")
        return codecName
    }

    /** 投喂一个 Opus 包（[pkt] 前 [len] 字节有效）并把解出的样本交给回调。 */
    fun decode(pkt: ByteArray, len: Int) {
        val h = handle
        if (h == 0L) return
        val n = len.coerceIn(0, MAX_PACKET)
        if (n == 0) {
            plc()
            return
        }
        val r = OpusNative.decDecode(h, pkt, n, scratch, SCRATCH_SAMPLES)
        emit(r, plc = false)
    }

    /**
     * 丢包补偿：按上一帧频谱合成一帧（[SCRATCH_SAMPLES] 是 120 ms，按协议帧长只取前面一段）。
     *
     * 取多少样本取决于编码方用的帧长，而客户端不知道——所以固定用协议约定的 10 ms
     * （480 样本）；真发来别的帧长的包时上面 [decode] 会解出对应的样本数，播放层累积即可。
     */
    fun plc() {
        val h = handle
        if (h == 0L) return
        val r = OpusNative.decDecode(h, null, 0, scratch, SCRATCH_SAMPLES)
        concealed++
        emit(r, plc = true)
    }

    fun stop() {
        val h = handle
        handle = 0
        onPcm = null
        dropped = 0
        concealed = 0
        if (h != 0L) OpusNative.decDestroy(h)
    }

    // ---------------------------------------------------------------- 内部

    /**
     * 交出解出的样本。`plc = true` 表示这是**补出来的**一帧（不是网络上收到的包）：
     * 播放环据此**不把它算进「收到包 / 来晚 / 到达间隔」**，否则统计会被自己的补偿污染
     * （实测：PLC 帧落进空环 → 每补一帧就自报一次「来晚」，把 5 次/s 的欠载显示成 34 次/s）。
     */
    private fun emit(samples: Int, plc: Boolean) {
        if (samples < 0) {
            dropped++
            Log.w(TAG, "解码失败：${OpusNative.lastError()}（code $samples）")
            return
        }
        if (samples == 0) return
        val n = samples.coerceAtMost(scratch.size)
        onPcm?.invoke(scratch, n, plc)
    }
}