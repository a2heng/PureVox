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
import android.media.AudioFormat
import com.purevox.mic.Proto
import com.purevox.mic.R

/**
 * Opus 的**协议常量与显示格式**。
 *
 * 编解码实现见 [OpusNative]（内置 libopus 1.5.2）。这里只放「协议侧固定的东西」：
 * 帧长、PCM 格式名。[Proto.CODEC] 是**协议字段**，不是 MediaCodec 的 mime
 * （Android 上从来没有 `audio/opus` 这个 mime，所以本工程完全不走 MediaCodec）。
 *
 * **为什么不用系统 MediaCodec**：系统 Opus codec 注册的是 `audio/raw` /
 * `audio/ogg;codecs=opus` / `audio/webm;codecs=opus`，后两者只认容器封装，喂裸 Opus 包会
 * 照单全收并输出垃圾 PCM（听感满屏刺啦且不报错）。想吃裸包就得自己实现 Ogg / WebM
 * 封装，那是拿「省一个依赖」换「自己维护一个协议」。详见 [OpusNative] 与 `cpp/pv_opus.cpp`。
 */
object OpusSupport {

    /** 与电脑端协议一致的编解码器名（[Proto.CODEC]）；**只是协议字段，不是 mime**。 */
    const val CODEC = Proto.CODEC

    /** 协议帧长 10 ms @48 kHz；编码器的投喂粒度与解码器的 PLC 都按它。 */
    const val FRAME_SAMPLES = 480

    /**
     * 状态栏 / 日志用的一行描述：**现在在用什么**。
     *
     * 只报在用的编解码器，不报「本机系统 codec 如何如何」——那是一段解释设计取舍的
     * 长文，放在状态栏只会占位置；真要查设计理由看 `cpp/pv_opus.cpp` 与 AGENTS.md 即可。
     */
    fun describe(context: Context): String =
        if (OpusNative.available) {
            OpusNative.version
        } else {
            context.getString(R.string.err_opus_unavailable, OpusNative.loadError)
        }

    /** 人类可读的 PCM 格式名（状态栏用）。 */
    fun pcmName(context: Context, encoding: Int): String = when (encoding) {
        AudioFormat.ENCODING_PCM_FLOAT -> "PCM_FLOAT"
        AudioFormat.ENCODING_PCM_16BIT -> "PCM_16BIT"
        else -> context.getString(R.string.err_encoding, encoding)
    }
}