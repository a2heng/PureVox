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
import com.purevox.mic.Proto

/**
 * 上行 Opus 编码器（手机麦克风 → 电脑），**libopus 1.5.2**（见 [OpusNative] 为什么不用系统 codec）。
 *
 * ## 帧长是确定的 10 ms
 *
 * 系统 MediaCodec 版本的帧长由厂商决定（20 ms、40 ms 都见过），所以当时只能「攒够多少发多少」。
 * 换成 libopus 后**帧长由我们自己定，且必须是协议约定的 10 ms**（480 样本 @48 kHz）——
 * 这正好和引擎的 10 ms hop 网格对齐（ARCHITECTURE.md §3），一包一 hop，不再有「两包对不齐」的情况。
 *
 * 于是结构简单到可以一眼看懂：FIFO 里够 480 个样本就编码一包，不够就等下一批。
 *
 * 采样约定：喂进来的样本是**归一化到 -1..1 的 f32 单声道**（与 `AudioRecord` 的 float 输出、
 * 引擎内部格式一致）；JNI 内部转 int16 喂给 libopus，那一层对 Java 侧零感知。
 *
 * 调用线程：只有麦克风采集线程会调 [encode]，本类内部无锁。
 */
class OpusEncoder {

    companion object {
        private const val TAG = "PureVoxEnc"

        /** 采样率 / 声道 / 码率只有 [Proto] 一处定义（协议硬约定，改一处即改全部）。 */
        const val SAMPLE_RATE = Proto.SAMPLE_RATE
        const val CHANNELS = Proto.CHANNELS
        const val BITRATE = Proto.BITRATE

        /**
         * 一次投喂的样本数：**10 ms @48 kHz = 480**。
         *
         * 这是 Opus 的标准帧长之一（也是引擎的 hop 网格），所以不用 `OPUS_SET_EXPERT_FRAME_SIZE`
         * 那种「任意帧长」模式——那会关掉编码器的一些优化。
         */
        const val CHUNK_SAMPLES = OpusSupport.FRAME_SAMPLES

        /** 单包上限：Opus 单帧最大 1275 字节，留 3 倍余量给将来换帧长。 */
        const val MAX_PACKET = 4000

        private const val FIFO_SAMPLES = SAMPLE_RATE // 1 s 抖动余量
    }

    /** 实际生效的编码器名（固定为 libopus，这里留着是为了状态栏与日志统一）。 */
    var codecName: String = "libopus"
        private set

    /** 已产出的 Opus 包数（状态栏用）。 */
    var packets: Long = 0L
        private set

    private var handle: Long = 0
    private val info = 0
    private val fifo = FloatArray(FIFO_SAMPLES)
    private var fifoLen = 0
    private var onPacket: ((ByteArray) -> Unit)? = null

    private val outBuf = ByteArray(MAX_PACKET)

    /** 攒到但还没发出去的包（满帧前不发），以及溢出丢弃的样本数。 */
    var dropped: Long = 0L
        private set

    /**
     * 建好编码器。成功返回 [codecName]，失败抛 [IllegalStateException]（消息已含原因）。
     */
    fun start(onPacket: (ByteArray) -> Unit): String {
        stop()
        OpusNative.require()
        this.onPacket = onPacket
        val h = OpusNative.encCreate(SAMPLE_RATE, CHANNELS, BITRATE, OpusNative.APP_VOIP)
        if (h == 0L) {
            val why = "${OpusNative.lastError()}（code ${OpusNative.lastErrorCode()}）"
            throw IllegalStateException("Opus 编码器创建失败：$why")
        }
        handle = h
        codecName = "libopus ${OpusNative.version}"
        Log.i(TAG, "编码器就绪：$codecName  ${SAMPLE_RATE}Hz/${CHANNELS}ch/${BITRATE}bps  帧 ${CHUNK_SAMPLES} 样本")
        return codecName
    }

    /**
     * 投喂 [count] 个归一化样本（-1..1 单声道），并把编码好的包交给回调。
     * 非阻塞：不够一整帧就把样本留在 FIFO 里，下次调用再试。
     */
    fun encode(src: FloatArray, count: Int) {
        if (count <= 0 || handle == 0L) return
        appendFifo(src, count)
        // FIFO 可能攒了多帧（麦克风一批读到的样本 > 一帧），一次全部编完
        while (fifoLen >= CHUNK_SAMPLES) {
            val n = OpusNative.encEncode(handle, fifo, CHUNK_SAMPLES, outBuf)
            if (n < 0) {
                Log.w(TAG, "编码失败：${OpusNative.lastError()}（code $n）")
                // 编不出来这一帧就丢掉它，别卡住整条链（否则 FIFO 永远满、之后全是丢弃）
                consumeFifo(CHUNK_SAMPLES)
                continue
            }
            onPacket?.invoke(outBuf.copyOf(n))
            packets++
            consumeFifo(CHUNK_SAMPLES)
        }
    }

    fun stop() {
        val h = handle
        handle = 0
        fifoLen = 0
        packets = 0
        dropped = 0
        onPacket = null
        if (h != 0L) OpusNative.encDestroy(h)
    }

    // ---------------------------------------------------------------- 内部

    private fun appendFifo(src: FloatArray, count: Int) {
        if (fifoLen + count > fifo.size) {
            val drop = fifoLen + count - fifo.size
            System.arraycopy(fifo, drop, fifo, 0, fifoLen - drop)
            fifoLen -= drop
            dropped += drop
        }
        System.arraycopy(src, 0, fifo, fifoLen, count)
        fifoLen += count
    }

    private fun consumeFifo(n: Int) {
        val remain = fifoLen - n
        if (remain > 0) System.arraycopy(fifo, n, fifo, 0, remain)
        fifoLen = remain
    }
}