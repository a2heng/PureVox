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
import java.nio.ByteBuffer

/**
 * 下行 Opus 解码器（电脑音频 → 手机）。**系统 `MediaCodec`，没有 JNI / 没有 libopus**。
 *
 * 关键约定：**一包不等于固定帧长**。电脑端按 10 ms 发包（480 样本），但：
 * - 别的客户端可能发 2.5~60 ms 任意帧长；
 * - 本机的 Opus 解码器常输出 20 ms（960 样本），与发包帧长无关。
 *
 * 所以本类**不假设输出长度**：每次 [decode] 把所有就绪的输出缓冲都取走，按**实际样本数**
 * 回调给播放层，由播放层累积进环形缓冲。
 *
 * 输出 PCM 格式以 `MediaCodec.getOutputFormat()` 为准（权威），而不是猜；
 * 拿不到该字段时按 AOSP 默认的 16 bit 处理。
 *
 * 调用线程：只有播放层的解码线程会调 [decode]，本类内部无锁。
 */
class OpusDecoder {

    companion object {
        private const val TAG = "PureVoxDec"

        /** 单包上限，与 [OpusEncoder.MAX_PACKET] 对齐（Opus 单包最大 1275 字节 × 帧数富余）。 */
        const val MAX_PACKET = 4000

        /** 输出缓冲暂存区（一次 dequeue 的输出按样本拷贝到这里，回调方必须立刻消费）。 */
        private const val SCRATCH_SAMPLES = 16384
    }

    /** 实际生效的解码器名。 */
    var codecName: String = ""
        private set

    /** 实际生效的输出 PCM 格式（由 `getOutputFormat()` 确认）。 */
    var outputEncoding: Int = AudioFormat.ENCODING_PCM_16BIT
        private set

    /** 因输入槽忙而丢弃的包数（理论极少；统计出来是因为「看不见的状态等于不存在的状态」）。 */
    var dropped: Long = 0L
        private set

    private var codec: MediaCodec? = null
    private val info = MediaCodec.BufferInfo()
    private val scratch = FloatArray(SCRATCH_SAMPLES)
    private var onPcm: ((FloatArray, Int) -> Unit)? = null

    /** 下一个包的时间戳基准（微秒），单调递增。 */
    private var pts = 0L

    /**
     * 建好解码器并 `start()`。成功返回解码器名，失败抛 [IllegalStateException]。
     */
    fun start(onPcm: (FloatArray, Int) -> Unit): String {
        stop()
        this.onPcm = onPcm
        val probe = OpusSupport.probe()
        val name = probe.decoder
            ?: throw IllegalStateException("本机没有 Opus 解码器（audio/opus），无法收听电脑音频。${probe.detail}")
        var lastReason = "未知"
        for (pcm in OpusSupport.decoderPcmPreference()) {
            try {
                codec = buildAndStart(name, pcm)
                codecName = name
                Log.i(TAG, "解码器就绪：$name（请求输出 ${OpusSupport.pcmName(pcm)}）")
                return name
            } catch (e: Throwable) {
                lastReason = e.message ?: e.javaClass.simpleName
                Log.w(TAG, "解码器 $name pcm=$pcm 不可用：$lastReason")
            }
        }
        stop()
        throw IllegalStateException("Opus 解码器配置失败：$lastReason")
    }

    /** 投喂一个 Opus 包（[pkt] 前 [len] 字节有效）并把解出的样本交给回调。 */
    fun decode(pkt: ByteArray, len: Int) {
        val c = codec ?: return
        drainOutput(c)
        val idx = c.dequeueInputBuffer(0)
        if (idx < 0) {
            dropped++
            return
        }
        val buf = c.getInputBuffer(idx)
        if (buf == null) {
            c.queueInputBuffer(idx, 0, 0, 0)
            return
        }
        buf.clear()
        val n = len.coerceIn(0, MAX_PACKET).coerceAtMost(buf.capacity())
        buf.put(pkt, 0, n)
        // presentationTimeUs 用单调递增的采样序号换算：帧长不固定，用真实时间戳会
        // 引入漂移，也让解码器按 PTS 做抖动缓冲时行为不可预期。
        val ptsUs = n.toLong() * 1_000_000L / OpusEncoder.SAMPLE_RATE
        pts += ptsUs
        c.queueInputBuffer(idx, 0, n, 0, pts)
        drainOutput(c)
    }

    fun stop() {
        val c = codec
        codec = null
        onPcm = null
        dropped = 0L
        pts = 0L
        if (c == null) return
        try {
            c.stop()
        } catch (e: Throwable) {
            Log.w(TAG, "stop 异常", e)
        }
        c.release()
    }

    // ---------------------------------------------------------------- 内部

    private fun buildAndStart(name: String, pcmEncoding: Int): MediaCodec {
        val format = MediaFormat().apply {
            setString(MediaFormat.KEY_MIME, OpusSupport.MIME)
            setInteger(MediaFormat.KEY_SAMPLE_RATE, OpusEncoder.SAMPLE_RATE)
            setInteger(MediaFormat.KEY_CHANNEL_COUNT, OpusEncoder.CHANNELS)
            setInteger(MediaFormat.KEY_PCM_ENCODING, pcmEncoding)
        }
        val c = MediaCodec.createByCodecName(name)
        try {
            c.configure(format, null, null, 0)
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
        while (guard++ < 64) {
            val idx = c.dequeueOutputBuffer(info, 0)
            if (idx == MediaCodec.INFO_TRY_AGAIN_LATER) return
            if (idx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                readOutputFormat(c.outputFormat)
                continue
            }
            if (idx < 0) return
            var samples = 0
            val buf = c.getOutputBuffer(idx)
            if (buf != null && info.size > 0) {
                buf.position(info.offset)
                buf.limit(info.offset + info.size)
                samples = readSamples(buf)
            }
            c.releaseOutputBuffer(idx, false)
            if (samples > 0) onPcm?.invoke(scratch, samples)
        }
    }

    private fun readSamples(buf: ByteBuffer): Int {
        return if (outputEncoding == AudioFormat.ENCODING_PCM_FLOAT) {
            val fb = buf.asFloatBuffer()
            val n = fb.remaining().coerceAtMost(scratch.size)
            fb.get(scratch, 0, n)
            n
        } else {
            val sb = buf.asShortBuffer()
            val n = sb.remaining().coerceAtMost(scratch.size)
            for (i in 0 until n) scratch[i] = sb.get(i) / 32768f
            n
        }
    }

    private fun readOutputFormat(format: MediaFormat) {
        try {
            if (format.containsKey(MediaFormat.KEY_PCM_ENCODING)) {
                val enc = format.getInteger(MediaFormat.KEY_PCM_ENCODING)
                if (enc != outputEncoding) {
                    outputEncoding = enc
                    Log.i(TAG, "解码输出 PCM 改为 ${OpusSupport.pcmName(enc)}")
                }
            }
            Log.i(TAG, "解码输出格式：$format")
        } catch (e: Throwable) {
            Log.w(TAG, "读输出格式失败", e)
        }
    }
}