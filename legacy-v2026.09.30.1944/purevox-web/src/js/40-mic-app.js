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

// Mic 版装配：对应 lite_mic（麦克风 → 降噪 → 本地输出，最简档）。
// 浏览器自带降噪/AEC/AGC 一律关掉——降噪只有 PureVox 一条实现路径。
(function (PV) {
    'use strict';

    const A = PV.ASSETS;
    const app = {
        cfg: null, pipeline: null, stream: null, running: false,
        deviceRate: 0, inRow: null, outRow: null,
    };
    PV.app = app;

    // 浏览器 DSP 全关：AEC / 降噪 / AGC 都由 PureVox 一条路径负责
    const GUM_CONSTRAINTS = {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
        channelCount: 1,
    };

    function setupDevices() {
        app.inRow = PV.session.setupInputRow('mic', app.cfg);
        app.outRow = PV.session.setupOutputRow('mic', app.cfg,
            (id) => (app.pipeline ? app.pipeline.setSinkId(id) : null));
    }

    async function start() {
        if (app.running) return;
        const inSel = PV.ui.el('in-select');
        const deviceId = inSel ? inSel.value : '';
        const audio = Object.assign({}, GUM_CONSTRAINTS);
        if (deviceId) audio.deviceId = { ideal: deviceId };

        PV.ui.log('请求麦克风权限…');
        let stream;
        try {
            stream = await navigator.mediaDevices.getUserMedia({ audio });
        } catch (e) {
            PV.ui.state('无权限', 'bad');
            PV.ui.log('麦克风不可用：' + (e && e.name ? e.name + ' ' + e.message : e));
            return;
        }
        app.stream = stream;
        const track = stream.getAudioTracks()[0];
        app.deviceRate = (track && track.getSettings && track.getSettings().sampleRate) || 0;
        PV.ui.log('麦克风已开：' + (track ? track.label : '') +
            ' · 设备原生 ' + (app.deviceRate ? app.deviceRate + ' Hz' : '未知') +
            ' · 引擎 48kHz');

        if (!PV.ort.ready) {
            const ok = await PV.session.bootEngine();
            if (!ok) {
                stream.getTracks().forEach((t) => t.stop());
                app.stream = null;
                return;
            }
        }

        app.pipeline = new PV.Pipeline();
        app.pipeline._onLevel = (peak) => PV.ui.level(PV.util.linearToDb(peak));
        app.pipeline._onStat = () => { /* 统计由 startStats 定时取 */ };
        try {
            await app.pipeline.start(stream, {
                preGainDb: app.cfg.pre_gain_db,
                postGainDb: app.cfg.post_gain_db,
            });
        } catch (e) {
            PV.ui.state('启动失败', 'bad');
            PV.ui.log('音频图启动失败：' + (e && e.message ? e.message : e));
            stream.getTracks().forEach((t) => t.stop());
            app.stream = null;
            app.pipeline = null;
            return;
        }

        // 权限已到手：设备 label 与输出列表此刻才解锁
        await app.inRow.refresh();
        const outSel = PV.ui.el('out-select');
        if (outSel && outSel.value) {
            if (await app.pipeline.setSinkId(outSel.value)) {
                app.cfg.output_device_id = outSel.value;
                app.cfg.output_device = outSel.options[outSel.selectedIndex].textContent;
                PV.util.saveCfg('mic', app.cfg);
                PV.ui.log('输出设备：' + app.cfg.output_device);
            }
        }

        app.running = true;
        PV.ui.running(true, '停止');
        PV.ui.state('运行中', 'good');
        PV.ui.status('运行中 · 48kHz 单声道 · 降噪常驻');
    }

    async function stop() {
        if (!app.running) return;
        app.running = false;
        if (app.pipeline) {
            await app.pipeline.stop();
            app.pipeline = null;
        }
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
            if (app.stream) app.stream.getTracks().forEach((t) => t.stop());
            PV.ort.dispose();
        });
    }

    async function main() {
        app.cfg = PV.session.init('mic');
        PV.ui.debug({ model: A.meta.modelLabel, hop: PV.util.HOP + ' / 10ms' });
        PV.ui.log('PureVox Web（Lite Mic）· ' + A.meta.modelLabel +
            ' · ONNX Runtime Web ' + A.meta.ortVersion);
        PV.ui.log('模型与 ONNX 运行时按需下载一次并长期缓存（两个页面共用同一份）');
        if (!PV.Pipeline.supported) {
            PV.ui.state('不支持', 'bad');
            PV.ui.log('当前浏览器不支持 AudioWorklet，无法运行');
            return;
        }
        setupDevices();
        bind();
        PV.session.startStats(() => ({
            dev: app.deviceRate ? app.deviceRate + ' Hz' : '--',
        }));
        await app.inRow.refresh();
        await app.outRow.refresh();
    }

    window.addEventListener('DOMContentLoaded', main);
})(window.PV = window.PV || {});
