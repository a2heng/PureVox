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
import android.os.Process
import android.util.Log
import com.purevox.mic.Proto

/**
 * 下行播放（电脑音频 → 手机扬声器）。
 *
 * ## 结构（只有三层，别再加）
 *
 * ```text
 *   OkHttp 收包线程 —— 解析帧头 → Opus 解码 → 写进播放环
 *   播放环（[PlaybackRing]）—— 吸收网络抖动 + 用变速重采样吸收晶振差
 *   写线程 —— 按 AudioTrack 的主时钟取 10 ms → 写设备
 * ```
 *
 * **没有包队列、没有第二个环、没有播放定时器**：`AudioTrack` 的阻塞写就是主时钟。
 * 旧实现是「包队列 + 解码线程 + 样本环 + 20 ms 写入线程 + 50 ms 预热」，多出来的每一层
 * 都是白送的延迟，而且每层都有自己的水位策略（互相打架）。
 *
 * ## 欠载怎么办
 *
 * 环空 → 先让 libopus 走 **PLC** 合成一帧（保持时间轴连续，听感是「一个音节糊了一下」），
 * 还空 → 环里淡化重复上一格。两种都计数，界面与 `/debug` 都看得到 —— 不静默。
 *
 * ## 时钟偏差是**量出来**的
 *
 * [clockPpm] 用 `AudioTrack.getPlaybackHeadPosition()`（硬件实际播出的帧数）对
 * `System.nanoTime()` 做差分：这就是「手机音频时钟 vs 电脑时钟」的真实偏差，
 * 随 `st` 上报给电脑，`/debug` 的 `net.client.clock_ppm` 直接能读。
 * 它也是伺服要吃掉的那个量 —— 先量，再决定要不要调参（AGENTS §1.4）。
 */
class SpeakerDownlink(
    /**
     * 电脑算出的时钟偏移（手机 − 电脑，µs）；还没测出来时返回 null。
     * 用来把下行帧头的时间戳换成**单向延迟**。
     */
    private val clockOffsetUs: () -> Long?,
) {

    companion object {
        private const val TAG = "PureVoxSpk"

        private const val SR = Proto.SAMPLE_RATE
        private const val HOP = Proto.FRAME_SAMPLES

        /** 立体声：很多机型的低延迟快通路只对 stereo 生效（mono 会被丢到深缓冲）。 */
        private const val CHANNELS = 2
        private const val CHANNEL_MASK = AudioFormat.CHANNEL_OUT_STEREO
        private const val ENCODING = AudioFormat.ENCODING_PCM_FLOAT

        /** AudioTrack 底层缓冲：**20 ms**（2 格）。它只吸收写线程调度，不是 jitter buffer。 */
        private const val TRACK_MS = 20
        /** 起播前的最低水位（ms）：到这就开声，不等「目标水位」，起播更快。 */
        private const val START_MS = 15.0
        private const val START_WAIT_MS = 500L

        /** 接缝阈值：1 kHz @ -20 dBFS 的正常样本差 ≈ 0.013，取 0.05（约 4 倍）当「跳变」。 */
        private const val SPLICE_DELTA = 0.05f
    }

    private var track: AudioTrack? = null
    private var decoder: OpusDecoder? = null
    private var writeThread: Thread? = null

    /** 播放环：**唯一的缓冲**（见 [PlaybackRing]）。 */
    private val ring = PlaybackRing()

    /** 解码器的保护锁：`decode()` 在 OkHttp 线程、`plc()` 在写线程。 */
    private val decLock = Any()

    @Volatile
    private var running = false

    /** 解码器名（诊断）。 */
    @Volatile
    var decoderName: String = ""
        private set

    /** 最近一次故障原因（null = 正常）。 */
    @Volatile
    var lastError: String? = null
        private set

    /** 最近一块音频的峰值（0..1）。 */
    @Volatile
    var peak: Float = 0f
        private set

    /** 最近一包的单向延迟（电脑发出 → 手机收到，ms）；没同步过就是 NaN。 */
    @Volatile
    var oneWayMs: Double = Double.NaN
        private set

    /**
     * 手机音频时钟实测偏差（ppm，正 = 手机比电脑快）；样本不足时是 NaN。
     *
     * 这不是估算：用 **硬件播放头**（`getPlaybackHeadPosition`）的增量除以 `nanoTime`
     * 增量。它回答的是「我们这边 hop 已经发出去了，接收方的 ring 到底会不会漂」。
     */
    @Volatile
    var clockPpm: Double = Double.NaN
        private set

    /**
     * 改**实时性上限**（ms，手动、两侧同步、1 帧 = 10 ms）。**不重建 AudioTrack**：
     * 只是阈值变了，下一拍就按新上限丢最旧的（拖滑块时听不出打断）。
     */
    fun setTargetMs(ms: Double) {
        ring.setMaxMs(ms)
        Log.i(TAG, "实时性上限 → ${ring.targetMs().toInt()} ms")
    }

    val levelMs: Double get() = ring.levelMs()
    val targetMs: Double get() = ring.targetMs()

    /** 抖动 / 最坏抖动：极简策略不再统计（保留字段，界面与诊断结构不变）。 */
    val jitterMs: Double get() = Double.NaN
    val worstJitterMs: Double get() = Double.NaN
    val maxGapMs: Double get() = ring.maxGapMs

    /** 到达间隔超过实时性上限的次数（这些拍一定听得出）。 */
    val gapOverRing: Long get() = if (ring.maxGapMs > ring.maxMs) 1L else 0L
    val underruns: Long get() = ring.underruns
    val plc: Long get() = 0L
    val trims: Long get() = ring.trims
    val late: Long get() = ring.late
    val dropped: Long get() = ring.dropped

    /** 收到的包数（解码线程计数，和 [hops] 一起就能算出「包是否都对上了格」）。 */
    val packets: Long get() = ring.packets

    /** 从环里取出的 hop 数（正常 = 100/s）。 */
    val hops: Long get() = ring.hops

    /** 掩盖次数：极简策略不再补帧（断流直接静音），恒为 0。 */
    val concealed: Long get() = 0L

    /** 「接缝」计数：极简策略下**恒为 0**（不做任何拼接/补帧，字段保留给界面结构）。 */
    val splicedHops: Long get() = 0L

    /** 观测到的最大相邻样本跳变（同上，极简策略下不再统计）。 */
    val maxSampleDelta: Float get() = 0f

    /** AudioTrack 自己的欠载次数（与环的欠载是两件事）。 */
    val deviceUnderruns: Int
        get() = try {
            track?.underrunCount ?: 0
        } catch (e: Throwable) {
            0
        }

    /** AudioTrack 实际申请的底层缓冲（ms）。 */
    val trackBufferMs: Int
        get() = try {
            (track?.bufferSizeInFrames ?: 0) * 1000 / SR
        } catch (e: Throwable) {
            0
        }

    /** AudioTrack 实际使用的采样率（设备可能不是 48 kHz，框架会重采样；诊断要看得到）。 */
    val trackSampleRate: Int
        get() = try {
            track?.sampleRate ?: 0
        } catch (e: Throwable) {
            0
        }

    /** 我们申请的底层缓冲（ms）。实际值见 [trackBufferMs] —— 两个都要能看到。 */
    @Volatile
    var bufferRequestedMs: Int = 0
        private set

    /** 是否拿到了低延迟快通路（`PERFORMANCE_MODE_LOW_LATENCY` 被系统接受）。 */
    @Volatile
    var lowLatencyPath: Boolean = false
        private set

    // ---------------------------------------------------------------- 生命周期

    /** 成功返回 true；失败时 [lastError] 有原因。 */
    fun start(): Boolean {
        if (running) return true
        lastError = null
        val dec = OpusDecoder()
        val name = try {
            dec.start { pcm, n, plc -> ring.push(pcm, n, System.nanoTime(), plc) }
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
        ring.reset()
        running = true
        oneWayMs = Double.NaN
        clockPpm = Double.NaN
        writeThread = Thread({ writeLoop(at) }, "pv-spk-out").also {
            it.isDaemon = true
            it.start()
        }
        Log.i(
            TAG,
            "下行播放启动：$name（环目标 ${targetMs.toInt()} ms，底层 $trackBufferMs ms，" +
                "共 ${trackBufferMs + targetMs.toInt()} ms 缓冲）",
        )
        return true
    }

    fun stop() {
        running = false
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
        writeThread = null
        synchronized(decLock) {
            decoder?.stop()
            decoder = null
        }
        if (at != null) {
            try {
                at.stop()
            } catch (e: Throwable) {
                Log.w(TAG, "stop 异常", e)
            }
            at.release()
        }
        track = null
        ring.reset()
        peak = 0f
        oneWayMs = Double.NaN
        Log.i(TAG, "下行播放停止")
    }

    // ---------------------------------------------------------------- 数据面

    /**
     * 收到一个音频包（**OkHttp 线程**）。`payload` 已经去掉了 8 字节帧头。
     *
     * 解码就在这个线程做（一个 10 ms 帧的解码是零点几毫秒，不会挡住后面的包）：
     * 多一个「解码线程 + 包队列」只会多一层延迟和一层水位。
     */
    fun onPacket(seq: Int, tsUs: Int, payload: ByteArray) {
        if (!running || payload.isEmpty()) return
        val dec = decoder ?: return
        // 单向延迟（需要时钟偏移）：只做展示与统计，不影响播放
        clockOffsetUs()?.let { off ->
            val us = Proto.usDiff(Proto.nowUs(), tsUs) + off
            if (us in 0..5_000_000) oneWayMs = us / 1000.0
        }
        synchronized(decLock) {
            try {
                dec.decode(payload, payload.size)
            } catch (e: Throwable) {
                lastError = "解码异常：${e.message ?: e.javaClass.simpleName}"
                Log.w(TAG, "解码异常（seq=$seq）", e)
            }
        }
    }

    // ---------------------------------------------------------------- 内部

    private fun openTrack(): AudioTrack? {
        var lastReason = "未知"
        for (lowLatency in booleanArrayOf(true, false)) {
            try {
                val minBytes = AudioTrack.getMinBufferSize(SR, CHANNEL_MASK, ENCODING)
                if (minBytes <= 0) {
                    lastReason = "getMinBufferSize = $minBytes"
                    continue
                }
                val bytes = maxOf(minBytes, HOP * 4 * CHANNELS * (TRACK_MS / 10))
                val attrs = AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build()
                val format = AudioFormat.Builder()
                    .setEncoding(ENCODING)
                    .setSampleRate(SR)
                    .setChannelMask(CHANNEL_MASK)
                    .build()
                val builder = AudioTrack.Builder()
                    .setAudioAttributes(attrs)
                    .setAudioFormat(format)
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .setBufferSizeInBytes(bytes)
                if (lowLatency && Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
                }
                val at = builder.build()
                if (at.state == AudioTrack.STATE_INITIALIZED) {
                    // 申请值 ≠ 实际值：设备侧的最小缓冲常常远大于我们要的 20 ms
                    // （本机实测 128 ms —— 那等于把抖动缓冲变成 128 ms，延迟白涨、环被抽干）。
                    // `setBufferSizeInFrames` 可以在**已分配**的缓冲里再收紧一次，收到硬件允许
                    // 的最小值为止；收不下去就把实际值报出去（诊断里一眼看见，不用猜）。
                    val allocated = at.bufferSizeInFrames
                    val want = HOP * 2 // 20 ms
                    val effective = try {
                        at.setBufferSizeInFrames(want).coerceAtLeast(want)
                    } catch (e: Throwable) {
                        Log.w(TAG, "setBufferSizeInFrames 失败", e)
                        allocated
                    }
                    bufferRequestedMs = TRACK_MS
                    lowLatencyPath = lowLatency
                    Log.i(
                        TAG,
                        "${if (lowLatency) "低延迟" else "普通"} · 分配 ${allocated * 1000 / SR} ms" +
                            " → 实际 ${effective * 1000 / SR} ms（申请 $TRACK_MS ms）",
                    )
                    return at
                }
                lastReason = "${if (lowLatency) "低延迟" else "普通"}：state=${at.state}"
                at.release()
            } catch (e: Throwable) {
                lastReason = "${if (lowLatency) "低延迟" else "普通"}：${e.message ?: e.javaClass.simpleName}"
                Log.w(TAG, "AudioTrack 创建失败", e)
            }
        }
        lastError = "AudioTrack 创建失败：$lastReason"
        return null
    }

    private fun writeLoop(at: AudioTrack) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val mono = FloatArray(HOP)
        val out = FloatArray(HOP * CHANNELS)
        // 起播：攒到最低水位再开声（0.5 s 内攒不到也开，别让用户以为坏了）
        val deadline = System.currentTimeMillis() + START_WAIT_MS
        while (running && ring.levelMs() < START_MS && System.currentTimeMillis() < deadline) {
            try {
                Thread.sleep(2)
            } catch (e: InterruptedException) {
                return
            }
        }
        try {
            at.play()
        } catch (e: Throwable) {
            lastError = "AudioTrack.play 失败：${e.message}"
            return
        }

        // 先把 AudioTrack 自己的底层缓冲**灌满静音**，再进 10 ms 取数循环。
        //
        // 不灌会怎样（2026-10-27 实测，就是「1 kHz 和系统声音都吞吞吐吐」的根因）：
        // `play()` 之后底层缓冲是空的，头两格写入把环里的 20 ms **一次性抽进底层缓冲**，
        // 环立刻稳定在 0 —— 之后每个包都落在空环上（`late` 每包 +1）、每一拍都补 PLC
        // （`underruns` 与 `concealed` 同步涨），听感就是规律的断续；而 `/debug` 里
        // `level_ms` 稳定显示 0，看着像「环没工作」。
        // 灌满之后：底层缓冲（设备侧垫层）+ 环（网络抖动垫层）各就各位，环停在 20 ms，
        // 与 DESIGN §4.1.5 的预算（环 20 + AudioTrack 20）一致。
        val prefillFrames = try {
            at.bufferSizeInFrames
        } catch (e: Throwable) {
            0
        }
        if (prefillFrames > 0) {
            if (!writeFully(at, FloatArray(prefillFrames * CHANNELS))) return
        }

        var lastHead = -1L
        var lastHeadNs = 0L
        var lastPeak = 0L
        // 接缝检测用：上一格最后一个样本
        var prevSample = 0f
        var started = false
        while (running) {
            val real = ring.pull(mono, HOP)
            var pk = 0f
            var maxDelta = 0f
            for (i in 0 until HOP) {
                val v = mono[i]
                out[i * 2] = v
                out[i * 2 + 1] = v
                val a = if (v < 0f) -v else v
                if (a > pk) pk = a
                // 相邻样本跳变（含**跨格接缝**：上一格末样本 → 本格首样本）
                val ref = if (i == 0) prevSample else mono[i - 1]
                val d = if (v > ref) v - ref else ref - v
                if (d > maxDelta) maxDelta = d
            }
            prevSample = mono[HOP - 1]
            peak = pk
            // 极简策略：不做补帧/拼接，所以没有「接缝」可统计（`real` 只用于电平判断）
            if (!real && pk > 0.5f) peak = 0f
            if (!writeFully(at, out)) return

            // 时钟偏差：硬件播放头的增量 vs nanoTime 增量（每 ~1 s 更新一次）
            val now = System.nanoTime()
            // 2 s 窗口：`getPlaybackHeadPosition` 是**分块**推进的，500 ms 窗口里
            // 那点量化误差会算出 ±700 ppm 这种不可能的晶振差（实测），拉长到 2 s 才稳。
            if (now - lastPeak > 2_000_000_000L) {
                lastPeak = now
                val head = try {
                    at.playbackHeadPosition.toLong() and 0xFFFF_FFFFL
                } catch (e: Throwable) {
                    -1L
                }
                if (head >= 0) {
                    if (lastHead >= 0) {
                        val dFrames = (head - lastHead + (1L shl 32)) % (1L shl 32)
                        val dNs = now - lastHeadNs
                        if (dNs > 400_000_000L && dFrames > 0) {
                            val rate = dFrames * 1e9 / dNs
                            clockPpm = (rate / SR - 1.0) * 1e6
                        }
                    }
                    lastHead = head
                    lastHeadNs = now
                }
            }
        }
    }

    private fun writeFully(at: AudioTrack, buf: FloatArray): Boolean {
        var off = 0
        while (off < buf.size && running) {
            val w = try {
                at.write(buf, off, buf.size - off, AudioTrack.WRITE_BLOCKING)
            } catch (e: Throwable) {
                lastError = "AudioTrack.write 异常：${e.message ?: e.javaClass.simpleName}"
                Log.w(TAG, "write 异常", e)
                return false
            }
            if (w <= 0) {
                if (w < 0) lastError = "AudioTrack.write 返回 $w（输出设备不可用）"
                try {
                    Thread.sleep(1)
                } catch (e: InterruptedException) {
                    return false
                }
                return true
            }
            off += w
        }
        return running
    }
}
