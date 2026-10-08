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

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import androidx.core.content.ContextCompat
import com.purevox.mic.Proto

/**
 * 上行采集（手机麦克风 → 电脑）：`AudioRecord` 采集 + [OpusEncoder] 编码，一条线程搞定。
 *
 * - **采集格式** 48 kHz / 单声道 / `ENCODING_PCM_FLOAT`；设备不支持时（`getMinBufferSize`
 *   报错）自动退回 `ENCODING_PCM_16BIT` 并在读取后归一化到 -1..1，两条路径对上层完全一样。
 * - **后台受限**：Android 9+ 后台拿不到麦克风，所以推流必须由
 *   `service/CaptureService`（microphone 类型前台服务）顶着 —— 界面在开启推流前
 *   先起服务，再开 `AudioRecord`。
 * - 读取阻塞时长 = [READ_SAMPLES]（20 ms），因此 [stop] 不需要等很久（join 1 s 足够）。
 * - 峰值电平写在 [level]，由界面轮询读取，不做回调（避免高频 post）。
 */
class MicUplink(
    private val context: Context,
    /** 一个 Opus 包（可直接丢给 `WsClient.sendAudio`）。 */
    private val onPacket: (ByteArray) -> Unit,
    /** 故障原因（人类可读），界面直接显示。 */
    private val onError: (String) -> Unit,
) {

    companion object {
        private const val TAG = "PureVoxMic"

        const val SAMPLE_RATE = Proto.SAMPLE_RATE
        private const val CHANNEL_MASK = AudioFormat.CHANNEL_IN_MONO

        /** 一次读多少样本：20 ms @48 kHz。 */
        private const val READ_SAMPLES = 960

        private const val REC_PERMISSION = Manifest.permission.RECORD_AUDIO
    }

    /** 实际生效的采集 PCM 格式。 */
    var recordEncoding: Int = AudioFormat.ENCODING_PCM_16BIT
        private set

    /** 实际生效的 Opus 编码器名。 */
    var encoderName: String = ""
        private set

    /** 最近一块音频的峰值（0..1）。 */
    @Volatile
    var level: Float = 0f
        private set

    private var record: AudioRecord? = null
    private var encoder: OpusEncoder? = null
    private var thread: Thread? = null

    @Volatile
    private var running = false

    /** 成功返回 true；失败时 [onError] 已给出原因。 */
    fun start(): Boolean {
        if (running) return true
        if (!hasPermission()) {
            onError("未授予麦克风权限，无法上行推流（系统设置里给 PureVox 的“麦克风”开权限）")
            return false
        }

        val enc = OpusEncoder()
        try {
            enc.start(onPacket)
        } catch (e: Throwable) {
            onError(e.message ?: "Opus 编码器不可用")
            Log.e(TAG, "编码器启动失败", e)
            return false
        }
        encoder = enc
        encoderName = enc.codecName

        val rec = buildRecord()
        if (rec == null) {
            enc.stop()
            encoder = null
            return false
        }
        try {
            rec.startRecording()
        } catch (e: Throwable) {
            onError("麦克风启动失败：${e.message ?: e.javaClass.simpleName}")
            rec.release()
            record = null
            enc.stop()
            encoder = null
            return false
        }
        if (rec.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
            onError("麦克风启动失败：AudioRecord 状态 = ${rec.recordingState}")
            rec.release()
            record = null
            enc.stop()
            encoder = null
            return false
        }

        record = rec
        level = 0f
        running = true
        thread = Thread({ captureLoop(rec, enc) }, "pv-mic").also { it.isDaemon = true; it.start() }
        Log.i(
            TAG,
            "上行采集启动：${OpusSupport.pcmName(recordEncoding)} @${SAMPLE_RATE}Hz → $encoderName",
        )
        return true
    }

    fun stop() {
        running = false
        thread?.join(1500)
        thread = null
        val rec = record
        record = null
        if (rec != null) {
            try {
                rec.stop()
            } catch (e: Throwable) {
                Log.w(TAG, "AudioRecord.stop 异常", e)
            }
            rec.release()
        }
        encoder?.stop()
        encoder = null
        level = 0f
    }

    private fun hasPermission(): Boolean =
        ContextCompat.checkSelfPermission(context, REC_PERMISSION) == PackageManager.PERMISSION_GRANTED

    // ---------------------------------------------------------------- 内部

    private fun buildRecord(): AudioRecord? {
        // 先建对象再开录：部分机型上 getMinBufferSize 能报一个正数，但真正
        // new AudioRecord 才暴露底层不支持（state != INITIALIZED / startRecording 失败）。
        // 所以这里两种格式都真建一次，建不成的记下来，取第一个能用的。
        var lastReason = "未知"
        for (enc in intArrayOf(AudioFormat.ENCODING_PCM_FLOAT, AudioFormat.ENCODING_PCM_16BIT)) {
            val minBytes = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_MASK, enc)
            if (minBytes <= 0) {
                lastReason = "AudioRecord.getMinBufferSize = $minBytes（${OpusSupport.pcmName(enc)}）"
                continue
            }
            val bufBytes = maxOf(minBytes, READ_SAMPLES * 4 * 4)
            val rec = try {
                val format = AudioFormat.Builder()
                    .setEncoding(enc)
                    .setSampleRate(SAMPLE_RATE)
                    .setChannelMask(CHANNEL_MASK)
                    .build()
                val builder = AudioRecord.Builder()
                    .setAudioSource(MediaRecorder.AudioSource.MIC)
                    .setAudioFormat(format)
                    .setBufferSizeInBytes(bufBytes)
                builder.build()
            } catch (e: Throwable) {
                lastReason = "${OpusSupport.pcmName(enc)}：${e.message ?: e.javaClass.simpleName}"
                Log.w(TAG, "AudioRecord 创建失败（$enc）", e)
                continue
            }
            if (rec.state != AudioRecord.STATE_INITIALIZED) {
                lastReason = "${OpusSupport.pcmName(enc)}：state=${rec.state}"
                rec.release()
                continue
            }
            recordEncoding = enc
            return rec
        }
        onError("本机 AudioRecord 不支持 ${SAMPLE_RATE} Hz 单声道采集 —— $lastReason")
        Log.e(TAG, "没有可用的采集格式：$lastReason")
        return null
    }

    private fun captureLoop(rec: AudioRecord, enc: OpusEncoder) {
        val floats = FloatArray(READ_SAMPLES)
        val shorts = ShortArray(READ_SAMPLES)
        val useFloat = recordEncoding == AudioFormat.ENCODING_PCM_FLOAT
        while (running) {
            val got = try {
                if (useFloat) rec.read(floats, 0, READ_SAMPLES, AudioRecord.READ_BLOCKING) else rec.read(shorts, 0, READ_SAMPLES)
            } catch (e: Throwable) {
                AudioRecord.ERROR_INVALID_OPERATION
            }
            if (got <= 0) {
                if (got == AudioRecord.ERROR_INVALID_OPERATION ||
                    got == AudioRecord.ERROR_BAD_VALUE ||
                    got == AudioRecord.ERROR_DEAD_OBJECT
                ) {
                    onError("麦克风读取中断：AudioRecord.read = $got")
                    running = false
                    return
                }
                // 极短暂无可用数据：让一下，别空转
                try {
                    Thread.sleep(2)
                } catch (e: InterruptedException) {
                    return
                }
                continue
            }
            var pk = 0f
            if (useFloat) {
                for (i in 0 until got) {
                    val a = if (floats[i] < 0f) -floats[i] else floats[i]
                    if (a > pk) pk = a
                }
            } else {
                for (i in 0 until got) {
                    val v = shorts[i] / 32768f
                    floats[i] = v
                    val a = if (v < 0f) -v else v
                    if (a > pk) pk = a
                }
            }
            level = pk
            enc.encode(floats, got)
        }
    }
}