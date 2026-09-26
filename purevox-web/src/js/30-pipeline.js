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

// 数据面编排（两个 flavor 共用，唯一实现路径）：
//   音源 → 采集 worklet（重采样到 48k，按 480 hop 切帧）
//        → 主线程投递 ORT worker（前增益 → 模型，10ms hop 进出）
//        → 播放 worklet 的播放环（后增益，设备时钟拉取）→ 扬声器
// Mic 版音源 = 本机麦克风；Net 版音源 = WebRTC 远端音轨。降噪与播放链路完全相同。
(function (PV) {
    'use strict';

    const A = PV.ASSETS;
    const HOP = PV.util.HOP;
    const QUEUE_CAP = 20;         // 等待推理的 hop 上限（200ms），封顶丢最旧

    class Pipeline {
        constructor() {
            this.ctx = null;
            this.srcNode = null;
            this.capNode = null;
            this.playNode = null;
            this.stream = null;
            this._queue = [];
            this._busy = false;
            this._preGain = 1;
            this._postDb = 0;
            this._stat = {
                watermark: 0, started: false, under: 0, dropped: 0,
                qdrop: 0, hops: 0, ratio: 1, actx: '--',
            };
            this._onLevel = null;
            this._onStat = null;
            this._running = false;
            this._resumeHooked = false;
        }

        static get supported() {
            return typeof AudioWorkletNode !== 'undefined' &&
                typeof AudioContext !== 'undefined';
        }

        // 输出设备选择（Chrome/Edge 110+ 的 AudioContext.setSinkId）；
        // 浏览器不支持时返回 false，调用方隐藏该行——不静默播到别的设备。
        static get sinkSelectable() {
            return typeof AudioContext !== 'undefined' &&
                typeof AudioContext.prototype.setSinkId === 'function';
        }

        async _ensureContext() {
            if (this.ctx) return;
            const Ctor = window.AudioContext || window.webkitAudioContext;
            // 采样率自适应：请求 48k，拿不到则由采集 worklet 重采样（无门禁）
            this.ctx = new Ctor({ sampleRate: PV.util.SAMPLE_RATE, latencyHint: 'interactive' });
            this._stat.actx = this.ctx.state;
            // 自动播放策略：无人交互时建出的 AudioContext 是 suspended，
            // 表现为「连接正常、有数据、但一点声音没有」且不报错——最容易被
            // 误判成坏了。故每次挂起都记下来，并在首次交互 / 页面回到前台时拉起。
            this.ctx.onstatechange = () => {
                this._stat.actx = this.ctx.state;
                if (this.ctx.state === 'suspended' && this._running) {
                    PV.ui.log('浏览器挂起了音频（自动播放策略）——点一下页面即可恢复');
                }
            };
            if (this.ctx.state === 'suspended') await this.resume();

            const captureSrc = PV.util.b64ToText(A.workerCaptureB64);
            const playbackSrc = PV.util.b64ToText(A.workerPlaybackB64);
            await this.ctx.audioWorklet.addModule(
                URL.createObjectURL(new Blob([captureSrc], { type: 'text/javascript' })));
            await this.ctx.audioWorklet.addModule(
                URL.createObjectURL(new Blob([playbackSrc], { type: 'text/javascript' })));

            this.capNode = new AudioWorkletNode(this.ctx, 'pv-capture', {
                numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
                processorOptions: { ratio: this.ctx.sampleRate / PV.util.SAMPLE_RATE },
            });
            this.playNode = new AudioWorkletNode(this.ctx, 'pv-playback', {
                numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [1],
            });

            this.capNode.port.onmessage = (e) => this._onCapture(e.data);
            this.playNode.port.onmessage = (e) => {
                if (e.data.type === 'stat') {
                    Object.assign(this._stat, e.data);
                    if (this._onStat) this._onStat(this._stat);
                }
            };

            // 采集节点输出恒静音，但必须挂到 destination 才会被音频线程拉动
            const mute = this.ctx.createGain();
            mute.gain.value = 0;
            this.capNode.connect(mute).connect(this.ctx.destination);
            this.playNode.connect(this.ctx.destination);
            this._stat.ratio = this.ctx.sampleRate / PV.util.SAMPLE_RATE;
        }

        _onCapture(msg) {
            if (msg.type === 'level') {
                if (this._onLevel) this._onLevel(msg.peak);
                return;
            }
            if (msg.type !== 'hop') return;
            this._queue.push(new Float32Array(msg.data));
            if (this._queue.length > QUEUE_CAP) {
                this._queue.shift();
                this._stat.qdrop++;
            }
            this._pump();
        }

        // 一次只在途一跳：Worker 慢于实时表现为播放环欠载（可见统计），
        // 而不是在 JS 侧堆一个无界队列。
        _pump() {
            if (this._busy || !this._queue.length || !this._running) return;
            const hop = this._queue.shift();
            this._busy = true;
            PV.ort.infer(hop, this._preGain).then((enh) => {
                this._busy = false;
                this._stat.hops++;
                const buf = enh.buffer;
                this.playNode.port.postMessage({ type: 'hop', data: buf }, [buf]);
                this._pump();
            }).catch((err) => {
                this._busy = false;
                PV.ui.log('推理失败：' + (err && err.message ? err.message : err));
                this._pump();
            });
        }

        // 拉起被自动播放策略挂起的 AudioContext；首次交互与页面回前台都会调
        async resume() {
            if (!this.ctx || this.ctx.state !== 'suspended') return;
            try {
                await this.ctx.resume();
                PV.ui.log('音频已恢复：' + this.ctx.state);
            } catch (e) {
                PV.ui.log('音频仍被挂起，需要点一下页面：' + (e && e.message ? e.message : e));
            }
            this._stat.actx = this.ctx.state;
        }

        // 采集 AudioWorkletProcessor：渲染量子（128 帧）只进不出，出帧恒为
        // 提前把 AudioContext 建好并拉起。浏览器的用户激活大约 5 秒就过期：
        // 网络版是「点连接 → ICE 8 秒后音轨才到」，那时再建 AudioContext 会被
        // 自动播放策略挂起（连上了却一点声音没有）。所以在点连接那一刻就 prepare。
        async prepare() {
            await this._ensureContext();
            await this.resume();
            this._watchResume();
        }

        async start(stream, opts) {
            opts = opts || {};
            this._running = true;
            this.stream = stream;
            this._preGain = PV.util.dbToLinear(opts.preGainDb || 0);
            this._postDb = opts.postGainDb || 0;

            await this._ensureContext();
            this.playNode.port.postMessage({ type: 'gain', db: this._postDb });
            this.playNode.port.postMessage({ type: 'flush' });
            PV.ort.flush();

            this.srcNode = this.ctx.createMediaStreamSource(stream);
            this.srcNode.connect(this.capNode);
            if (this.ctx.state === 'suspended') await this.resume();
            this._watchResume();
        }

        // 交互 / 可见性兜底：自动播放策略下第一次点页面就会放行音频
        _watchResume() {
            if (this._resumeHooked || !this.ctx) return;
            this._resumeHooked = true;
            const kick = () => this.resume();
            document.addEventListener('pointerdown', kick);
            document.addEventListener('keydown', kick);
            document.addEventListener('visibilitychange', () => {
                if (!document.hidden) kick();
            });
        }

        async setSinkId(deviceId) {
            if (!this.ctx || !Pipeline.sinkSelectable || !deviceId) return false;
            try {
                await this.ctx.setSinkId(deviceId);
                return true;
            } catch (e) {
                PV.ui.log('切换输出设备失败：' + (e && e.message ? e.message : e));
                return false;
            }
        }

        setPreGainDb(db) {
            this._preGain = PV.util.dbToLinear(db);
        }

        setPostGainDb(db) {
            this._postDb = db;
            if (this.playNode) this.playNode.port.postMessage({ type: 'gain', db });
        }

        // 端到端延迟估计：预缓冲 5 hop + 一跳模型滞后 + 设备缓冲
        latencyMs() {
            if (!this.ctx) return 0;
            const dev = ((this.ctx.baseLatency || 0) + (this.ctx.outputLatency || 0)) * 1000;
            return Math.round(5 * PV.util.HOP_MS + PV.util.HOP_MS + dev);
        }

        async stop() {
            this._running = false;
            this._queue.length = 0;
            this._busy = false;
            try { if (this.srcNode) this.srcNode.disconnect(); } catch (e) { /* 已断开 */ }
            try { if (this.capNode) this.capNode.disconnect(); } catch (e) { /* 已断开 */ }
            try { if (this.playNode) this.playNode.disconnect(); } catch (e) { /* 已断开 */ }
            this.srcNode = null;
            if (this.ctx) {
                try { await this.ctx.close(); } catch (e) { /* 已关闭 */ }
                this.ctx = null;
                this.capNode = null;
                this.playNode = null;
            }
            this._stat = {
                watermark: 0, started: false, under: 0, dropped: 0,
                qdrop: 0, hops: 0, ratio: 1,
            };
        }
    }

    PV.Pipeline = Pipeline;
})(window.PV = window.PV || {});
