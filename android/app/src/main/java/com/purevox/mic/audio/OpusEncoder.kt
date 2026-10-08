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
import android.media.MediaCodec
import android.media.MediaFormat
import android.util.Log
import com.purevox.mic.Proto
import java.nio.ByteBuffer

/**
 * 上行 Opus 编码器（手机麦克风 → 电脑）。**系统 `MediaCodec`，没有 JNI / 没有 libopus**。
 *
 * 设计要点：
 * - **输出驱动**：`encode()` 不按固定节拍发包，而是「有多少输出包发多少」。
 *   因此无论编码器真实帧长是 2.5 ms、10 ms 还是 60 ms，流的平均速率都是对的，
 *   不会失步；帧长与 [CHUNK_SAMPLES] 不一致只多一点编码器内部缓冲。
 * - **帧长不硬凑 10 ms**：优先用 `KEY_OPUS_FRAME_DURATION_US`（API 29+ 的公开 key）声明
 *   20 ms，厂商实现不认这个 key 时自动降级为「不带该 key」再试一次。
 * - **输入 PCM 格式实测**：[OpusSupport.encoderPcmPreference] 给出优先级，
 *   逐个 configure；全失败就抛异常让界面显示明确原因，绝不静默推 0 字节。
 * - 调用线程：只有麦克风采集线程会调 [encode]，本类内部无锁。
 *
 * 采样约定：喂进来的样本是 **归一化到 -1..1 的 f32 单声道**；编码成 16 bit 时在
 * [writeSamples] 里做一次转换（带削顶与半 LSB 舍入）。
 */
class OpusEncoder {

    companion object {
        private const val TAG = "PureVoxEnc"

        /** 采样率 / 声道 / 码率只有 [Proto] 一处定义（协议硬约定，改一处即改全部）。 */
        const val SAMPLE_RATE = Proto.SAMPLE_RATE
        const val CHANNELS = Proto.CHANNELS
        const val BITRATE = Proto.BITRATE

        /** 一次投喂的样本数：20 ms @48 kHz（Opus 规范帧之一，AOSP opus 编码器默认）。 */
        const val CHUNK_SAMPLES = 960

        /** 单包最大字节（Opus 上限 1275 × 帧数，留足余量）；[OpusDecoder] 按同一上限接收。 */
        const val MAX_PACKET = 4000

        private const val FIFO_SAMPLES = Proto.SAMPLE_RATE      // 1 s 抖动余量

        /** `MediaFormat.KEY_OPUS_FRAME_DURATION_US`，字面量避免 minSdk 24 的 lint 抱怨。 */
        private const val KEY_OPUS_FRAME_DURATION_US = "opus-frame-duration-us"
        private const val FRAME_DURATION_US = 20000L
    }

    /** 实际生效的编码器名（能力探测 + configure 结果决定）。 */
    var codecName: String = ""
        private set

    /** 实际生效的输入 PCM 格式。 */
    var inputEncoding: Int = AudioFormat.ENCODING_PCM_16BIT
        private set

    /** 已产出的 Opus 包数（状态栏用）。 */
    var packets: Long = 0L
        private set

    private var codec: MediaCodec? = null
    private val info = MediaCodec.BufferInfo()
    private val fifo = FloatArray(FIFO_SAMPLES)
    private var fifoLen = 0
    private var onPacket: ((ByteArray) -> Unit)? = null

    /** 下一个输入块的时间戳基准（微秒），单调递增。 */
    private var pts = 0L

    /**
     * 建好编码器并 `start()`。成功返回编码器名，失败抛 [IllegalStateException]
     * （消息已含原因，由界面直接显示）。
     */
    fun start(onPacket: (ByteArray) -> Unit): String {
        stop()
        this.onPacket = onPacket
        val probe = OpusSupport.probe()
        val name = probe.encoder
            ?: throw IllegalStateException("本机没有 Opus 编码器（audio/opus），无法上行推流。${probe.detail}")
        val pcmOrder = OpusSupport.encoderPcmPreference(name, SAMPLE_RATE, CHANNELS, BITRATE)
        var lastReason = "未知"
        for (pcm in pcmOrder) {
            for (withFrameDuration in booleanArrayOf(true, false)) {
                try {
                    codec = buildAndStart(name, pcm, withFrameDuration)
                    codecName = name
                    inputEncoding = pcm
                    Log.i(TAG, "编码器就绪：$name pcm=${OpusSupport.pcmName(pcm)} frameDurationUs=$withFrameDuration")
                    return name
                } catch (e: Throwable) {
                    lastReason = e.message ?: e.javaClass.simpleName
                    Log.w(TAG, "编码器 $name pcm=$pcm frameDurationUs=$withFrameDuration 不可用：$lastReason")
                }
            }
        }
        stop()
        throw IllegalStateException("Opus 编码器配置失败：$lastReason")
    }

    /**
     * 投喂 [count] 个归一化样本（-1..1 单声道），并把编码器已就绪的输出包交给回调。
     * 非阻塞：没有可用输入槽就把样本留在内部 FIFO 里，下次调用再试。
     */
    fun encode(src: FloatArray, count: Int) {
        if (count <= 0) return
        val c = codec ?: return
        appendFifo(src, count)
        drainOutput(c)
        queueInput(c)
    }

    fun stop() {
        val c = codec
        codec = null
        fifoLen = 0
        pts = 0L
        packets = 0L
        onPacket = null
        if (c == null) return
        try {
            c.stop()
        } catch (e: Throwable) {
            Log.w(TAG, "stop 异常", e)
        }
        c.release()
    }

    // ---------------------------------------------------------------- 内部

    private fun buildAndStart(name: String, pcmEncoding: Int, withFrameDuration: Boolean): MediaCodec {
        val format = MediaFormat().apply {
            setString(MediaFormat.KEY_MIME, OpusSupport.MIME)
            setInteger(MediaFormat.KEY_SAMPLE_RATE, SAMPLE_RATE)
            setInteger(MediaFormat.KEY_CHANNEL_COUNT, CHANNELS)
            setInteger(MediaFormat.KEY_BIT_RATE, BITRATE)
            setInteger(MediaFormat.KEY_PCM_ENCODING, pcmEncoding)
            if (withFrameDuration) setLong(KEY_OPUS_FRAME_DURATION_US, FRAME_DURATION_US)
        }
        val c = MediaCodec.createByCodecName(name)
        try {
            c.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            c.start()
        } catch (e: Throwable) {
            try {
                c.release()
            } catch (e2: Throwable) {
                Log.w(TAG, "release 异常", e2)
            }
            throw e
        }
        return c
    }

    private fun drainOutput(c: MediaCodec) {
        var guard = 0
        while (guard++ < 256) {
            val idx = c.dequeueOutputBuffer(info, 0)
            if (idx == MediaCodec.INFO_TRY_AGAIN_LATER) return
            if (idx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                Log.i(TAG, "编码输出格式：${c.outputFormat}")
                continue
            }
            if (idx < 0) return
            val buf = c.getOutputBuffer(idx)
            if (buf != null && info.size > 0) {
                val n = info.size.coerceAtMost(MAX_PACKET)
                val packet = ByteArray(n)
                buf.position(info.offset)
                buf.get(packet, 0, n)
                onPacket?.invoke(packet)
                packets++
            }
            c.releaseOutputBuffer(idx, false)
        }
    }

    private fun queueInput(c: MediaCodec) {
        if (fifoLen < CHUNK_SAMPLES) return
        val idx = c.dequeueInputBuffer(0)
        if (idx < 0) return
        val n = fifoLen.coerceAtMost(CHUNK_SAMPLES)
        val buf = c.getInputBuffer(idx)
        if (buf == null) {
            c.queueInputBuffer(idx, 0, 0, 0L, 0)
            return
        }
        val bytes = writeSamples(buf, n)
        // 同样用单调递增的 PTS（按样本数换算），不用墙钟：帧长不固定，
        // 墙钟 PTS 会漂移，也让对端任何按 PTS 的处理变得不可预期。
        val ptsUs = n.toLong() * 1_000_000L / SAMPLE_RATE
        pts += ptsUs
        c.queueInputBuffer(idx, 0, bytes, pts, 0)
        consumeFifo(n)
    }

    /** 把 FIFO 前 [n] 个样本写成编码器要的 PCM 形态，返回写入字节数。 */
    private fun writeSamples(buf: ByteBuffer, n: Int): Int {
        buf.clear()
        return if (inputEncoding == AudioFormat.ENCODING_PCM_FLOAT) {
            buf.asFloatBuffer().put(fifo, 0, n)
            val bytes = n * 4
            buf.position(0)
            buf.limit(bytes)
            bytes
        } else {
            val shorts = buf.asShortBuffer()
            for (i in 0 until n) {
                val clamped = when {
                    fifo[i] > 1f -> 1f
                    fifo[i] < -1f -> -1f
                    else -> fifo[i]
                }
                val q = clamped * 32767f
                shorts.put((if (q >= 0f) q + 0.5f else q - 0.5f).toInt().toShort())
            }
            val bytes = n * 2
            buf.position(0)
            buf.limit(bytes)
            bytes
        }
    }

    private fun appendFifo(src: FloatArray, count: Int) {
        if (fifoLen + count > fifo.size) {
            val drop = fifoLen + count - fifo.size
            System.arraycopy(fifo, drop, fifo, 0, fifoLen - drop)
            fifoLen -= drop
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