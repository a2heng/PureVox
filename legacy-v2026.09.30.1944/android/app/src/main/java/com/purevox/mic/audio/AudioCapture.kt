/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
# 
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
# 
# SPDX-License-Identifier: GPL-3.0-or-later
 */

package com.purevox.mic.audio

import android.Manifest
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.core.app.ActivityCompat

class AudioCapture(private val context: android.content.Context) {
    private var recorder: AudioRecord? = null
    private var isRecording = false
    private var captureThread: Thread? = null
    var onPcmData: ((ShortArray) -> Unit)? = null
    var onLevel: ((Float) -> Unit)? = null

    companion object {
        const val SAMPLE_RATE = 48000
        const val CHANNEL = AudioFormat.CHANNEL_IN_MONO
        const val ENCODING = AudioFormat.ENCODING_PCM_16BIT
        const val FRAME_SIZE = 480 // 10ms @ 48kHz（与模型 hop 一致）
    }

    fun start(audioSource: Int = MediaRecorder.AudioSource.MIC): Boolean {
        if (ActivityCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED) return false

        val bufferSize = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL, ENCODING)
        recorder = AudioRecord(audioSource, SAMPLE_RATE, CHANNEL, ENCODING,
            maxOf(bufferSize, FRAME_SIZE * 2))

        if (recorder?.state != AudioRecord.STATE_INITIALIZED) return false

        isRecording = true
        recorder?.startRecording()
        captureThread = Thread({ captureLoop() }, "AudioCapture").also { it.start() }
        return true
    }

    fun stop() {
        isRecording = false
        captureThread?.join(1000)
        recorder?.stop()
        recorder?.release()
        recorder = null
    }

    private fun captureLoop() {
        val frame = ShortArray(FRAME_SIZE)
        while (isRecording) {
            val read = recorder?.read(frame, 0, FRAME_SIZE) ?: 0
            if (read > 0) {
                val data = if (read == FRAME_SIZE) frame.copyOf()
                           else frame.copyOf(read)
                onPcmData?.invoke(data)
                var peak = 0f
                for (s in data) {
                    val abs = kotlin.math.abs(s.toFloat() / 32768f)
                    if (abs > peak) peak = abs
                }
                onLevel?.invoke(peak)
            }
        }
    }
}