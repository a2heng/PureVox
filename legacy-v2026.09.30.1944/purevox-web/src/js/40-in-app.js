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

// 输入端装配：麦克风 → 48k 单声道 Int16（每帧 480 样本 = 10ms）→ WS 推给服务端。
//
// 本页不跑降噪、不跑 Opus、不做任何处理——采集 worklet 出 hop 就转 Int16
// 发出去。降噪只在输出页做。服务端收到字节后原样转给输出端，它也不碰音频。
(function (PV) {
    'use strict';

    const app = {
        cfg: null, ctx: null, capNode: null, stream: null, ws: null,
        running: false, inRow: null, sentFrames: 0, dropped: 0, ep: null,
    };
    PV.app = app;

    // 中继地址：?srv= 优先，其次记住的上次，缺省与本页同源
    function endpoint() {
        const box = PV.ui.el('srv-input');
        const typed = box ? box.value.trim() : '';
        if (typed && typed !== app.cfg.srv) {
            app.cfg.srv = typed;
            PV.util.saveCfg('in', app.cfg);
        }
        app.ep = PV.util.resolveServer('in', app.cfg);
        if (box && !typed && app.ep.srv) box.value = app.ep.srv;
        return app.ep;
    }

    function wsUrl() {
        return endpoint().wsBase + '/ws/up';
    }

    async function ensureContext() {
        if (app.ctx) return;
        const Ctor = window.AudioContext || window.webkitAudioContext;
        app.ctx = new Ctor({ sampleRate: PV.util.SAMPLE_RATE, latencyHint: 'interactive' });
        const src = PV.util.b64ToText(PV.ASSETS.workerCaptureB64);
        await app.ctx.audioWorklet.addModule(
            URL.createObjectURL(new Blob([src], { type: 'text/javascript' })));
        app.capNode = new AudioWorkletNode(app.ctx, 'pv-capture', {
            numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
            processorOptions: { ratio: app.ctx.sampleRate / PV.util.SAMPLE_RATE },
        });
        app.capNode.port.onmessage = (e) => onCapture(e.data);
        const mute = app.ctx.createGain();
        mute.gain.value = 0;
        app.capNode.connect(mute).connect(app.ctx.destination);
        if (app.ctx.state === 'suspended') await app.ctx.resume();
    }

    function onCapture(msg) {
        if (msg.type === 'level') {
            PV.ui.level(PV.util.linearToDb(msg.peak));
            return;
        }
        if (msg.type !== 'hop' || !app.running) return;
        // 背压：socket 发不出去就丢当前跳，不堆积——延迟有界比一帧不丢重要
        if (!app.ws || app.ws.readyState !== WebSocket.OPEN || app.ws.bufferedAmount > 48000) {
            app.dropped++;
            return;
        }
        const f32 = new Float32Array(msg.data);
        const i16 = new Int16Array(f32.length);
        for (let i = 0; i < f32.length; i++) {
            const v = f32[i] < -1 ? -1 : (f32[i] > 1 ? 1 : f32[i]);
            i16[i] = Math.round(v * 32767);
        }
        app.ws.send(i16.buffer);
        app.sentFrames++;
    }

    async function start() {
        if (app.running) return;
        if (!PV.Pipeline.supported) {
            PV.ui.state('不支持', 'bad');
            PV.ui.log('当前浏览器不支持 AudioWorklet，无法运行');
            return;
        }
        const inSel = PV.ui.el('in-select');
        const audio = {
            echoCancellation: false, noiseSuppression: false,
            autoGainControl: false, channelCount: 1,
        };
        if (inSel && inSel.value) audio.deviceId = { ideal: inSel.value };

        PV.ui.log('请求麦克风权限…');
        let stream;
        try {
            stream = await navigator.mediaDevices.getUserMedia({ audio });
        } catch (e) {
            PV.ui.state('无权限', 'bad');
            PV.ui.log('麦克风不可用：' + (e && e.name ? e.name + ' ' + e.message : e));
            return;
        }

        let ws;
        try {
            ws = new WebSocket(wsUrl());
            await new Promise((res, rej) => {
                ws.onopen = res;
                ws.onerror = () => rej(new Error('连不上 ' + wsUrl()));
            });
        } catch (e) {
            PV.ui.state('无服务', 'bad');
            PV.ui.log('推流失败：' + (e && e.message ? e.message : e));
            stream.getTracks().forEach((t) => t.stop());
            return;
        }

        await ensureContext();
        const srcNode = app.ctx.createMediaStreamSource(stream);
        srcNode.connect(app.capNode);
        app._srcNode = srcNode;
        app.stream = stream;
        app.ws = ws;
        app.sentFrames = 0;
        app.dropped = 0;
        ws.onclose = () => {
            if (app.running) {
                PV.ui.log('与服务器断开');
                stop();
            }
        };
        // 权限已到手：设备 label 此刻才解锁
        await app.inRow.refresh();

        app.running = true;
        PV.ui.running(true, '停止');
        PV.ui.state('推送中', 'good');
        PV.ui.status('推送中 · 48kHz 单声道 · 10ms/帧');
        PV.ui.log('已连接 ' + wsUrl() + '，开始推送');
    }

    async function stop() {
        if (!app.running) return;
        app.running = false;
        try {
            if (app.ws && app.ws.readyState === WebSocket.OPEN) {
                app.ws.send(JSON.stringify({ type: 'stop' }));
                app.ws.close();
            }
        } catch (e) { /* 已断开 */ }
        app.ws = null;
        try { if (app._srcNode) app._srcNode.disconnect(); } catch (e) { /* 已断开 */ }
        app._srcNode = null;
        if (app.stream) {
            app.stream.getTracks().forEach((t) => t.stop());
            app.stream = null;
        }
        PV.ui.running(false, '启动');
        PV.ui.state('已停止', 'idle');
        PV.ui.status('已停止');
        PV.ui.resetLevel();
    }

    function bind() {
        const btn = PV.ui.el('btn-run');
        if (btn) btn.addEventListener('click', () => (app.running ? stop() : start()));
        window.addEventListener('beforeunload', () => {
            try {
                if (app.ws && app.ws.readyState === WebSocket.OPEN) {
                    app.ws.send(JSON.stringify({ type: 'stop' }));
                }
            } catch (e) { /* 关闭中 */ }
            if (app.stream) app.stream.getTracks().forEach((t) => t.stop());
        });
    }

    async function main() {
        app.cfg = PV.session.init('in');
        // 输入页不输出、不调增益：整组隐藏（与输出页的输出行/增益行区分职责）
        const controls = PV.ui.el('rx-controls');
        if (controls) controls.style.display = 'none';
        const ep = endpoint();
        const box = PV.ui.el('srv-input');
        if (box && ep.srv) box.value = ep.srv;
        PV.ui.el('server-note').textContent =
            '推流到 ' + (ep.remote ? ep.srv : '本页所在的服务器') +
            '（只传声音，不做处理，降噪在输出那一端做）';
        PV.ui.debug({ model: '无（本页不降噪）', hop: PV.util.HOP + ' / 10ms' });
        PV.ui.log('PureVox Web（网络输入端）· 麦克风 → Int16 10ms 帧 → 服务器');
        PV.ui.log('降噪在输出端做，本页只采集发送');
        app.inRow = PV.session.setupInputRow('in', app.cfg);
        bind();
        PV.session.startStats(() => ({
            ws: app.ws ? String(app.ws.readyState) : '--',
            sent: app.sentFrames ? (app.sentFrames / 100).toFixed(1) + ' s' : '--',
        }));
        await app.inRow.refresh();
    }

    window.addEventListener('DOMContentLoaded', main);
})(window.PV = window.PV || {});
