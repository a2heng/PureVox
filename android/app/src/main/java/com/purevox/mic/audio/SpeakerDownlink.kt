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

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Build
import android.util.Log
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

/**
 * 下行播放（电脑音频 → 手机扬声器）：**包队列 + 解码线程 + 样本环形缓冲 + 写入线程 + AudioTrack**。
 *
 * 为什么这么分层：
 * - WebSocket 的收包线程（OkHttp）**不能**做解码 —— 一解码就会挡住后面的包，
 *   抖动直接变成音频卡顿。所以 [enqueue] 只做一次入队，解码在独立线程（`pv-spk-dec`）。
 * - 解码输出**按样本累积**在环形缓冲里，而不是「一包一帧对齐播放」：
 *   发包帧长（2.5~60 ms）和解码输出帧长（常见 20 ms）都可能不同。
 * - `AudioTrack` 是唯一主时钟：写入线程**永远不停**（欠载补静音），
 *   这样设备的播放时钟不会因为数据断档而重启/漂移；水位超上限则丢最旧。
 *
 * 三个线程 + 一个缓冲的典型水位：预热 40 ms，稳态几十毫秒。状态栏直接读
 * [bufferedMs] / [underruns] / [dropped] / [lastError]，不做「看起来正常」的伪装。
 */
class SpeakerDownlink {

    companion object {
        private const val TAG = "PureVoxSpk"

        const val SAMPLE_RATE = OpusEncoder.SAMPLE_RATE
        private const val CHANNEL_MASK = AudioFormat.CHANNEL_OUT_MONO
        private const val PCM_ENCODING = AudioFormat.ENCODING_PCM_FLOAT

        /** 环形缓冲 = 2^16 样本（约 1.36 s），满了丢最旧。 */
        private const val RING_SAMPLES = 1 shl 16

        /** 一次写入的样本数：10 ms @48 kHz（与工程 10 ms hop 网格一致）。 */
        private const val WRITE_CHUNK = 480

        /** 开声前预热水位：40 ms。 */
        private const val PREBUFFER_SAMPLES = 1920

        /** 包队列深度：200 包（@10 ms ≈ 2 s），突发兜底。 */
        private const val PACKET_QUEUE = 200

        private const val PREBUFFER_WAIT_MS = 500L

        /** 解码线程的取包等待；超时只是让线程有机会看到 running=false。 */
        private const val POLL_WAIT_MS = 100L
    }

    private val ring = FloatArray(RING_SAMPLES)
    private val wIdx = AtomicLong(0)
    private val rIdx = AtomicLong(0)

    /** 待解码的 Opus 包（OkHttp 线程写，解码线程读）。 */
    private val packets = ArrayBlockingQueue<ByteArray>(PACKET_QUEUE)

    private var decoder: OpusDecoder? = null
    private var track: AudioTrack? = null
    private var decodeThread: Thread? = null
    private var writeThread: Thread? = null

    @Volatile
    private var running = false

    /** 实际生效的解码器名。 */
    var decoderName: String = ""
        private set

    /** 欠载（缓冲空、写了静音）次数。 */
    @Volatile
    var underruns: Long = 0L
        private set

    /** 丢掉的包 / 样本数。 */
    @Volatile
    var dropped: Long = 0L
        private set

    /** 最近一块音频的峰值（0..1，状态栏用）。 */
    @Volatile
    var peak: Float = 0f
        private set

    /** 最近一次故障原因（null = 正常）；界面上要显示，不能静默。 */
    @Volatile
    var lastError: String? = null
        private set

    // ---------------------------------------------------------------- 生命周期

    /** 成功返回 true；失败时 [lastError] 有原因。 */
    fun start(): Boolean {
        if (running) return true
        lastError = null
        val dec = OpusDecoder()
        val name = try {
            dec.start { pcm, n -> push(pcm, n) }
        } catch (e: Throwable) {
            lastError = e.message ?: "Opus 解码器不可用"
            Log.e(TAG, "解码器启动失败", e)
            return false
        }
        decoder = dec
        decoderName = name

        val at = openTrack()
        if (at == null) {
            dec.stop()
            decoder = null
            return false
        }
        track = at
        running = true
        packets.clear()
        underruns = 0L
        dropped = 0L
        peak = 0f

        decodeThread = Thread({ decodeLoop(dec) }, "pv-spk-dec").also { it.isDaemon = true; it.start() }
        writeThread = Thread({ writeLoop(at) }, "pv-spk-out").also { it.isDaemon = true; it.start() }
        Log.i(TAG, "下行播放启动：$name")
        return true
    }

    fun stop() {
        running = false
        // 先让阻塞的 write 出来，再 join，否则线程会卡在 WRITE_BLOCKING 上
        val at = track
        if (at != null) {
            try {
                at.pause()
                at.flush()
            } catch (e: Throwable) {
                Log.w(TAG, "pause/flush 异常", e)
            }
        }
        writeThread?.join(1000)
        decodeThread?.join(500)
        writeThread = null
        decodeThread = null
        decoder?.stop()
        decoder = null
        if (at != null) {
            try {
                at.stop()
            } catch (e: Throwable) {
                Log.w(TAG, "stop 异常", e)
            }
            at.release()
        }
        track = null
        packets.clear()
        wIdx.set(0)
        rIdx.set(0)
        underruns = 0L
        dropped = 0L
        peak = 0f
        Log.i(TAG, "下行播放停止")
    }

    // ---------------------------------------------------------------- 数据面

    /** 收包线程调用：入队一个 Opus 包。队列满则丢最旧（保最新，保可懂度）。 */
    fun enqueue(packet: ByteArray) {
        if (!running) return
        if (packets.offer(packet)) return
        packets.poll()
        dropped++
        if (!packets.offer(packet)) dropped++
    }

    /** 解码线程调用：把解出的样本写进环形缓冲。 */
    private fun push(src: FloatArray, n: Int) {
        if (n <= 0) return
        var w = (wIdx.get() and (RING_SAMPLES - 1).toLong()).toInt()
        for (i in 0 until n) {
            ring[w] = src[i]
            w++
            if (w == RING_SAMPLES) w = 0
        }
        wIdx.addAndGet(n.toLong())
        val over = wIdx.get() - rIdx.get() - RING_SAMPLES
        if (over > 0) {
            // 解码跑飞（长时间取不走）：丢最旧，把水位压回上限
            rIdx.set(wIdx.get() - RING_SAMPLES)
            dropped++
        }
    }

    // ---------------------------------------------------------------- 状态

    private fun bufferedSamples(): Int = (wIdx.get() - rIdx.get()).toInt().coerceAtLeast(0)

    fun bufferedMs(): Float = bufferedSamples() * 1000f / SAMPLE_RATE

    // ---------------------------------------------------------------- 内部

    private fun openTrack(): AudioTrack? {
        return try {
            val minBytes = AudioTrack.getMinBufferSize(SAMPLE_RATE, CHANNEL_MASK, PCM_ENCODING)
            // 至少 80 ms：给调度抖动留余量（minBytes 常常只有几十毫秒）
            val bytes = maxOf(if (minBytes > 0) minBytes else 0, WRITE_CHUNK * 4 * 8)
            val attrs = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                .build()
            val format = AudioFormat.Builder()
                .setEncoding(PCM_ENCODING)
                .setSampleRate(SAMPLE_RATE)
                .setChannelMask(CHANNEL_MASK)
                .build()
            val builder = AudioTrack.Builder()
                .setAudioAttributes(attrs)
                .setAudioFormat(format)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(bytes)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            }
            val at = builder.build()
            if (at.state != AudioTrack.STATE_INITIALIZED) {
                lastError = "AudioTrack 初始化失败（48 kHz 单声道 float 输出不被支持）"
                at.release()
                null
            } else {
                at
            }
        } catch (e: Throwable) {
            lastError = "AudioTrack 创建失败：${e.message ?: e.javaClass.simpleName}"
            Log.e(TAG, "AudioTrack 创建失败", e)
            null
        }
    }

    private fun decodeLoop(dec: OpusDecoder) {
        while (running) {
            val pkt = try {
                packets.poll(POLL_WAIT_MS, TimeUnit.MILLISECONDS)
            } catch (e: InterruptedException) {
                null
            } ?: continue
            if (!running) return
            try {
                dec.decode(pkt, pkt.size)
            } catch (e: Throwable) {
                lastError = "解码异常：${e.message ?: e.javaClass.simpleName}"
                Log.e(TAG, "解码异常", e)
                return
            }
        }
    }

    private fun writeLoop(at: AudioTrack) {
        val chunk = FloatArray(WRITE_CHUNK)
        val silence = FloatArray(WRITE_CHUNK)
        try {
            // 预热：先攒够水位再开声，避免起播一段断音
            val deadline = System.currentTimeMillis() + PREBUFFER_WAIT_MS
            while (running && bufferedSamples() < PREBUFFER_SAMPLES && System.currentTimeMillis() < deadline) {
                try {
                    Thread.sleep(5)
                } catch (e: InterruptedException) {
                    return
                }
            }
            at.play()
            while (running) {
                if (bufferedSamples() >= WRITE_CHUNK) {
                    readInto(chunk)
                    writeFully(at, chunk)
                } else {
                    // 欠载：补静音，绝不停写 —— 播放时钟必须不断
                    underruns++
                    writeFully(at, silence)
                }
            }
        } catch (e: Throwable) {
            lastError = "播放线程退出：${e.message ?: e.javaClass.simpleName}"
            Log.e(TAG, "播放线程异常", e)
        }
    }

    private fun writeFully(at: AudioTrack, buf: FloatArray) {
        var off = 0
        while (off < buf.size && running) {
            val w = try {
                at.write(buf, off, buf.size - off, AudioTrack.WRITE_BLOCKING)
            } catch (e: Throwable) {
                lastError = "AudioTrack.write 异常：${e.message ?: e.javaClass.simpleName}"
                Log.e(TAG, "write 异常", e)
                return
            }
            if (w <= 0) {
                if (w < 0) lastError = "AudioTrack.write 返回 $w（输出设备不可用）"
                try {
                    Thread.sleep(1)
                } catch (e: InterruptedException) {
                    return
                }
                return
            }
            off += w
        }
    }

    private fun readInto(dst: FloatArray) {
        var idx = (rIdx.get() and (RING_SAMPLES - 1).toLong()).toInt()
        var pk = 0f
        for (i in dst.indices) {
            val v = ring[idx]
            dst[i] = v
            val a = if (v < 0f) -v else v
            if (a > pk) pk = a
            idx++
            if (idx == RING_SAMPLES) idx = 0
        }
        rIdx.addAndGet(dst.size.toLong())
        peak = pk
    }
}