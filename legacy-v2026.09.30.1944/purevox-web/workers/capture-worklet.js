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

// 采集 AudioWorkletProcessor：渲染量子（128 帧）只进不出，出帧恒为
// 480 样本（10ms @48kHz，与模型 hop 对齐）——数据面不出现 1024/2048
// 等错位块。输入按 AudioContext 实际采样率线性重采样到 48k（自适应，
// 不做采样率门禁）；本节点输出恒静音，仅靠 destination 侧的静音增益
// 保持被音频线程拉动。
'use strict';

const HOP = 480;             // 10ms @48kHz
const TARGET_RATE = 48000;   // 引擎内部唯一格式
const COMPACT_AT = 1024;     // 输入 FIFO 整理阈值

class CaptureProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();
        const opts = (options && options.processorOptions) || {};
        this._ratio = opts.ratio || 1;          // 输入率 / 48k
        this._fifo = new Float32Array(8192);
        this._len = 0;                          // FIFO 内有效样本数
        this._rp = 0;                           // 读游标（输入域，浮点）
        this._pend = new Float32Array(HOP);
        this._filled = 0;
        this._pendingOut = null;
        this._peak = 0;
        this._reportCount = 0;
    }

    process(inputs, outputs) {
        const out = outputs[0] && outputs[0][0];
        if (out) out.fill(0);
        const input = inputs[0] && inputs[0][0];
        if (!input || !input.length) return true;

        // 电平（每 4 个量子上报一次，约 90Hz，避免消息洪水）
        for (let i = 0; i < input.length; i++) {
            const a = input[i] < 0 ? -input[i] : input[i];
            if (a > this._peak) this._peak = a;
        }
        if ((++this._reportCount & 3) === 0) {
            this.port.postMessage({ type: 'level', peak: this._peak });
            this._peak = 0;
        }

        // 追加到输入 FIFO
        if (this._len + input.length > this._fifo.length) {
            const grow = new Float32Array(Math.max(this._fifo.length * 2,
                this._len + input.length));
            grow.set(this._fifo.subarray(0, this._len));
            this._fifo = grow;
        }
        this._fifo.set(input, this._len);
        this._len += input.length;

        // 按目标率线性插值出满一 hop（单一路径：ratio===1 时 frac 恒 0，
        // 退化为直取，不另开分支）
        while (this._filled < HOP && this._rp + 1 < this._len) {
            const i0 = this._rp | 0;
            const frac = this._rp - i0;
            const a = this._fifo[i0];
            const b = this._fifo[i0 + 1];
            this._pend[this._filled++] = a + (b - a) * frac;
            this._rp += this._ratio;
        }

        // 丢弃已消费前缀（FIFO 保持有界）
        const used = this._rp | 0;
        if (used >= COMPACT_AT) {
            this._fifo.copyWithin(0, used, this._len);
            this._len -= used;
            this._rp -= used;
        }

        if (this._filled >= HOP) {
            const hop = this._pend;
            this._pend = new Float32Array(HOP);
            this._filled = 0;
            this.port.postMessage(
                { type: 'hop', data: hop.buffer, backlog: this._len - used },
                [hop.buffer]);
        }
        return true;
    }
}

registerProcessor('pv-capture', CaptureProcessor);
