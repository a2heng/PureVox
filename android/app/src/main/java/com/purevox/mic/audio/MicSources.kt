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

import android.content.Context
import android.media.MediaRecorder
import com.purevox.mic.R

/**
 * 麦克风采集 API（`MediaRecorder.AudioSource`）清单。
 *
 * 为什么做成下拉：同一台手机上不同 AudioSource 走的是**不同的底层通路/处理链**
 * （是否开 AEC/NS/AGC、是否走低延迟、是否允许内录）。和电脑版「设备下拉」是同一个
 * 思路：把选择权交给用户，但默认用最常见的 MIC。
 *
 * 顺序 = **常用程度**，从上到下；列表末尾是系统专用 / 隐藏 API（普通应用通常没有
 * 权限，[MicUplink] 建流失败会自动回退到 MIC，并把原因显示出来，不静默）。
 * 新增 / 调整时保持「常用在前、隐藏在后」，选择按 [Source.id] 持久化。
 */
object MicSources {

    data class Source(val id: Int, val labelRes: Int)

    @Suppress("InlinedApi")
    val ALL: List<Source> = listOf(
        Source(MediaRecorder.AudioSource.MIC, R.string.mic_src_mic),
        Source(MediaRecorder.AudioSource.VOICE_RECOGNITION, R.string.mic_src_voice_recognition),
        Source(MediaRecorder.AudioSource.VOICE_COMMUNICATION, R.string.mic_src_voice_communication),
        Source(MediaRecorder.AudioSource.CAMCORDER, R.string.mic_src_camcorder),
        Source(MediaRecorder.AudioSource.UNPROCESSED, R.string.mic_src_unprocessed),
        Source(MediaRecorder.AudioSource.VOICE_PERFORMANCE, R.string.mic_src_voice_performance),
        Source(MediaRecorder.AudioSource.DEFAULT, R.string.mic_src_default),
        // 需要 CAPTURE_AUDIO_OUTPUT / 系统签名；普通应用会失败，保留是为了覆盖
        // 「能拿到系统权限」的设备（车机 / 定制 ROM）并给失败原因，不是死路一条。
        Source(MediaRecorder.AudioSource.VOICE_CALL, R.string.mic_src_voice_call),
        Source(MediaRecorder.AudioSource.VOICE_UPLINK, R.string.mic_src_voice_uplink),
        Source(MediaRecorder.AudioSource.VOICE_DOWNLINK, R.string.mic_src_voice_downlink),
        Source(MediaRecorder.AudioSource.REMOTE_SUBMIX, R.string.mic_src_remote_submix),
        // 隐藏常量（AOSP 里有、公开 SDK 没有）：用数值，避免依赖隐藏符号编译。
        Source(1999, R.string.mic_src_hotword),
        Source(1997, R.string.mic_src_echo_reference),
        Source(1998, R.string.mic_src_fm_tuner),
        Source(1996, R.string.mic_src_voice_signalling),
    )

    fun find(id: Int): Source = ALL.firstOrNull { it.id == id } ?: ALL[0]

    fun label(context: Context, id: Int): String = context.getString(find(id).labelRes)
}
