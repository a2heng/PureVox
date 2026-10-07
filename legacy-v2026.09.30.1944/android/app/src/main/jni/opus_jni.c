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

#include <jni.h>
#include <stdlib.h>
#include <string.h>
#include "opus.h"

JNIEXPORT jlong JNICALL
Java_com_purevox_mic_audio_OpusEncoder_nativeCreate(
        JNIEnv *env, jobject thiz, jint sample_rate, jint channels) {
    int error;
    OpusEncoder *encoder = opus_encoder_create(sample_rate, channels, OPUS_APPLICATION_VOIP, &error);
    if (error != OPUS_OK || encoder == NULL) {
        return 0;
    }
    opus_encoder_ctl(encoder, OPUS_SET_COMPLEXITY(5));
    opus_encoder_ctl(encoder, OPUS_SET_BITRATE(32000));
    return (jlong)(intptr_t)encoder;
}

JNIEXPORT jbyteArray JNICALL
Java_com_purevox_mic_audio_OpusEncoder_nativeEncode(
        JNIEnv *env, jobject thiz, jlong handle, jshortArray pcm) {
    if (handle == 0) return NULL;
    OpusEncoder *encoder = (OpusEncoder *)(intptr_t)handle;

    jsize len = (*env)->GetArrayLength(env, pcm);
    jshort *pcm_data = (*env)->GetShortArrayElements(env, pcm, NULL);
    if (pcm_data == NULL) return NULL;

    unsigned char output[4000];
    int nb_bytes = opus_encode(encoder, pcm_data, len, output, sizeof(output));
    (*env)->ReleaseShortArrayElements(env, pcm, pcm_data, JNI_ABORT);

    if (nb_bytes < 0) return NULL;

    jbyteArray result = (*env)->NewByteArray(env, nb_bytes);
    if (result != NULL) {
        (*env)->SetByteArrayRegion(env, result, 0, nb_bytes, (jbyte *)output);
    }
    return result;
}

JNIEXPORT void JNICALL
Java_com_purevox_mic_audio_OpusEncoder_nativeDestroy(
        JNIEnv *env, jobject thiz, jlong handle) {
    if (handle != 0) {
        OpusEncoder *encoder = (OpusEncoder *)(intptr_t)handle;
        opus_encoder_destroy(encoder);
    }
}
