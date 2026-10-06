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

class OpusEncoder(sampleRate: Int = 48000, channels: Int = 1) {
    private var nativeHandle: Long = 0

    init {
        nativeHandle = nativeCreate(sampleRate, channels)
    }

    fun encode(pcm: ShortArray): ByteArray? {
        if (nativeHandle == 0L) return null
        return nativeEncode(nativeHandle, pcm)
    }

    fun destroy() {
        if (nativeHandle != 0L) {
            nativeDestroy(nativeHandle)
            nativeHandle = 0
        }
    }

    companion object {
        init {
            System.loadLibrary("opus_jni")
        }

        @JvmStatic private external fun nativeCreate(sampleRate: Int, channels: Int): Long
        @JvmStatic private external fun nativeEncode(handle: Long, pcm: ShortArray): ByteArray?
        @JvmStatic private external fun nativeDestroy(handle: Long)
    }
}
