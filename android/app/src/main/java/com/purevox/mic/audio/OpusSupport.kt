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
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import com.purevox.mic.Proto

/**
 * Opus 编解码器的**能力探测**（不崩、不猜：探测不到就报明确原因）。
 *
 * 为什么需要它：`audio/opus` 在 Android 上是「系统类型」，具体实现由厂商决定，
 * 名字可能是 `c2.android.opus.encoder` / `OMX.google.opus.encoder` / 厂商自研，
 * 而且不同实现接受的**输入/输出 PCM 格式**不同（有的只吃 16 bit，有的能吃 float）。
 * 直接 `MediaCodec.createEncoderByType("audio/opus")` 会拿到「第一个」实现，
 * 往往是不可预期的那一个。
 *
 * 这里做两件事：
 * 1. 用 [MediaCodecList] 列出所有 `audio/opus` 编解码器，挑一个（优先 AOSP 软件实现，
 *    行为最可预期），并把候选名都带出来，便于界面上说清「为什么选了这个」。
 * 2. 用 `MediaCodecInfo.isFormatSupported` 试探编码器能接受哪种输入 PCM 格式，
 *    给出**优先级列表**交给 [OpusEncoder] 逐个尝试（真正的兜底还是 configure 成不成功）。
 *
 * 探测不确定时（老系统/厂商实现不报 pcm-encoding）：优先 16 bit —— 那是 AOSP 的默认输入格式，
 * 所有实现都能吃；float 只在明确探测到支持时才用。
 */
object OpusSupport {

    const val MIME = Proto.CODEC

    /** 探测结果：[encoder] / [decoder] 为 null 表示本机没有对应能力。 */
    data class Probe(val encoder: String?, val decoder: String?, val detail: String)

    /** 列出本机所有 `audio/opus` 编解码器，并选出推荐的一个。 */
    fun probe(): Probe {
        val list = MediaCodecList(MediaCodecList.ALL_CODECS)
        val encoders = ArrayList<String>()
        val decoders = ArrayList<String>()
        for (info in list.codecInfos) {
            if (!isOpus(info)) continue
            if (info.isEncoder) encoders.add(info.name) else decoders.add(info.name)
        }
        encoders.sortBy { rank(it) }
        decoders.sortBy { rank(it) }

        val enc = encoders.firstOrNull()
        val dec = decoders.firstOrNull()
        val sb = StringBuilder()
        sb.append("enc=").append(enc ?: "无").append("  dec=").append(dec ?: "无")
        if (encoders.size > 1) sb.append("  候选 enc: ").append(encoders.joinToString(", "))
        if (decoders.size > 1) sb.append("  候选 dec: ").append(decoders.joinToString(", "))
        return Probe(enc, dec, sb.toString())
    }

    /**
     * 编码器输入 PCM 格式的**尝试顺序**（[OpusEncoder] 会按顺序 configure，第一个成功的生效）。
     *
     * 探测不到任何信息时返回 `[16bit, float]`：16 bit 是 AOSP 的默认输入格式，兼容性最好。
     */
    fun encoderPcmPreference(
        codecName: String,
        sampleRate: Int,
        channels: Int,
        bitRate: Int,
    ): List<Int> {
        val floatOk = supportsPcm(codecName, sampleRate, channels, bitRate, AudioFormat.ENCODING_PCM_FLOAT)
        return if (floatOk) {
            listOf(AudioFormat.ENCODING_PCM_FLOAT, AudioFormat.ENCODING_PCM_16BIT)
        } else {
            listOf(AudioFormat.ENCODING_PCM_16BIT, AudioFormat.ENCODING_PCM_FLOAT)
        }
    }

    /** 解码器输出 PCM 格式的偏好顺序；实际用哪个以 `MediaCodec.getOutputFormat()` 为准。 */
    fun decoderPcmPreference(): List<Int> = listOf(
        AudioFormat.ENCODING_PCM_FLOAT,
        AudioFormat.ENCODING_PCM_16BIT,
    )

    /** 人类可读的 PCM 格式名（状态栏用）。 */
    fun pcmName(encoding: Int): String = when (encoding) {
        AudioFormat.ENCODING_PCM_FLOAT -> "PCM_FLOAT"
        AudioFormat.ENCODING_PCM_16BIT -> "PCM_16BIT"
        else -> "编码 $encoding"
    }

    private fun isOpus(info: MediaCodecInfo): Boolean = try {
        info.supportedTypes.any { it.equals(MIME, ignoreCase = true) }
    } catch (e: Throwable) {
        false
    }

    /** 优先 AOSP 软件实现（帧长/延迟行为最可预期），其次 Google 的 OMX，最后才是厂商实现。 */
    private fun rank(name: String): Int {
        val lower = name.lowercase()
        return when {
            lower.contains("c2.android.opus") -> 0
            lower.contains("google.opus") -> 1
            else -> 2
        }
    }

    private fun lookup(codecName: String, encoder: Boolean): MediaCodecInfo? {
        val list = MediaCodecList(MediaCodecList.ALL_CODECS)
        for (info in list.codecInfos) {
            if (info.isEncoder != encoder) continue
            if (info.name == codecName) return info
        }
        return null
    }

    private fun supportsPcm(
        codecName: String,
        sampleRate: Int,
        channels: Int,
        bitRate: Int,
        pcmEncoding: Int,
    ): Boolean {
        val info = lookup(codecName, true) ?: return false
        return try {
            val format = MediaFormat().apply {
                setString(MediaFormat.KEY_MIME, MIME)
                setInteger(MediaFormat.KEY_SAMPLE_RATE, sampleRate)
                setInteger(MediaFormat.KEY_CHANNEL_COUNT, channels)
                setInteger(MediaFormat.KEY_BIT_RATE, bitRate)
                setInteger(MediaFormat.KEY_PCM_ENCODING, pcmEncoding)
            }
            info.getCapabilitiesForType(MIME).isFormatSupported(format)
        } catch (e: Throwable) {
            false
        }
    }
}