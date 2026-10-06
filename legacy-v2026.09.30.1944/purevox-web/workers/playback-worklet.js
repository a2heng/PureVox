// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// The built-in AI models are NOT covered by the GPL; they are the
// property of a2heng and may only be used with PureVox under
// authorization.  See MODEL-LICENSE.md for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

// 播放 AudioWorkletProcessor：节点内持有播放环（主线程按 hop 投递），
// 渲染量子按设备时钟拉取；预缓冲 5 hop（50ms）后开声，欠载静音重同步，
// 封顶 30 hop（300ms）丢最旧——与桌面端 PlaybackSink 同一套水位语义。
// 后增益在本节点施加（对齐 Lite「引擎 → 后增益 → 设备」）。
'use strict';

const HOP = 480;                 // 10ms @48kHz
const PRE_HOP = 5;               // 预缓冲 50ms
const CAP_HOP = 30;              // 封顶 300ms
const FADE = 64;                 // 欠载/重同步淡入样本数

class PlaybackProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        const cap = CAP_HOP * HOP;
        this._buf = new Float32Array(cap);
        this._cap = cap;
        this._w = 0;                // 写游标（累计样本数，单调）
        this._r = 0;                // 读游标
        this._started = false;
        this._gain = 1;
        this._gainTarget = 1;
        this._fade = FADE;          // 开声前淡入计数
        this._under = 0;
        this._dropped = 0;
        this._quant = 0;

        // 主线程投递：hop 写入 / 增益 / 清空（端口消息在音频线程有序处理）
        this.port.onmessage = (e) => {
            const m = e.data;
            if (m.type === 'hop') {
                this._push(new Float32Array(m.data));
            } else if (m.type === 'gain') {
                this._gainTarget = Math.pow(10, m.db / 20);
            } else if (m.type === 'flush') {
                this._w = 0;
                this._r = 0;
                this._started = false;
                this._fade = FADE;
                this._gain = 1;
                this._gainTarget = 1;
            }
        };
    }

    get _size() {
        return this._w - this._r;
    }

    _push(chunk) {
        // 封顶丢最旧（速率差/调度抖动由水位吸收，不垫零、不复用上一帧）
        const over = this._size + chunk.length - this._cap;
        if (over > 0) {
            this._r += over;
            this._dropped += over;
        }
        for (let i = 0; i < chunk.length; i++) {
            this._buf[(this._w + i) % this._cap] = chunk[i];
        }
        this._w += chunk.length;
        if (!this._started && this._size >= PRE_HOP * HOP) {
            this._started = true;
            this._fade = FADE;
        }
    }

    process(inputs, outputs) {
        const out = outputs[0] && outputs[0][0];
        if (!out) return true;
        const n = out.length;

        // 增益一阶跟随（点按不爆音）
        this._gain += (this._gainTarget - this._gain) * 0.2;

        for (let i = 0; i < n; i++) {
            let v = 0;
            if (this._started) {
                if (this._r < this._w) {
                    v = this._buf[this._r % this._cap];
                    this._r++;
                } else {
                    // 欠载：静音 + 重同步（下次满预缓冲再开声）
                    this._started = false;
                    this._under++;
                    this._fade = FADE;
                }
            }
            let g = this._gain;
            if (this._started && this._fade > 0) {
                g *= 1 - this._fade / FADE;
                this._fade--;
            }
            out[i] = v * g;
        }

        if ((this._quant++ % 8) === 0) {
            this.port.postMessage({
                type: 'stat',
                watermark: this._size / HOP,
                started: this._started,
                under: this._under,
                dropped: this._dropped,
            });
        }
        return true;
    }
}

registerProcessor('pv-playback', PlaybackProcessor);
