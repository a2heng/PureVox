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

// 输出端装配：WS 订阅远端 Int16 帧 → 本页 ONNX 降噪 → 本地扬声器。
//
// 降噪只在这里做（与 Mic 页同一条 Pipeline + 同一个模型）。
// 本页还负责「选局域网 IP → 生成二维码」：二维码内容就是输入页地址，
// 手机扫完打开 in.html，两边各点一次启动即连通，全程零手工交换。
(function (PV) {
    'use strict';

    const A = PV.ASSETS;
    const app = {
        cfg: null, pipeline: null, ws: null,
        running: false, engineReady: false, outRow: null,
        rxFrames: 0, ep: null,
    };
    PV.app = app;

    function endpoint() {
        const box = PV.ui.el('srv-input');
        const typed = box ? box.value.trim() : '';
        if (typed && typed !== app.cfg.srv) {
            app.cfg.srv = typed;
            PV.util.saveCfg('out', app.cfg);
        }
        app.ep = PV.util.resolveServer('out', app.cfg);
        if (box && !typed && app.ep.srv) box.value = app.ep.srv;
        return app.ep;
    }

    function wsUrl() {
        return endpoint().wsBase + '/ws/down';
    }

    // 二维码内容：本页与中继同源时直接给局域网 URL；跨站部署时
    // （如 github.io）给「本站 in.html + ?srv=中继」，手机打开后自动连回去。
    function inPageUrl(ip) {
        const ep = app.ep || endpoint();
        if (ep.remote) {
            let port = '';
            try { port = new URL(ep.httpBase).port; } catch (e) { port = ''; }
            if (!port) port = location.protocol === 'https:' ? '443' : '80';
            return location.origin + '/in.html?srv=' + ip + ':' + port;
        }
        return location.protocol + '//' + ip + ':' + location.port + '/in.html';
    }

    // ── 局域网 IP 下拉 + 二维码 ──
    async function loadLan() {
        const sel = PV.ui.el('ip-select');
        const ep = endpoint();
        let ips = [];
        try {
            const resp = await fetch(ep.httpBase + '/api/lan', { cache: 'no-store' });
            const info = await resp.json();
            ips = info.ips || [];
        } catch (e) {
            PV.ui.log('取网卡列表失败：' + (e && e.message ? e.message : e));
        }
        sel.innerHTML = '';
        if (!ips.length) {
            const o = document.createElement('option');
            o.value = '';
            o.textContent = '无网卡信息（手动输 IP）';
            sel.appendChild(o);
        }
        ips.forEach((item) => {
            const o = document.createElement('option');
            o.value = item.ip;
            o.textContent = item.name + ' · ' + item.ip;
            sel.appendChild(o);
        });
        // 默认选中「浏览器当前打开用的那个 IP」——二维码指回去一定能连上
        const here = location.hostname;
        const hit = ips.find((item) => item.ip === here);
        sel.value = (hit && hit.ip) || app.cfg.lan_ip || (ips.length ? ips[0].ip : '');
        if (!sel.value && sel.options.length) sel.selectedIndex = 0;
        sel.addEventListener('change', () => {
            app.cfg.lan_ip = sel.value;
            PV.util.saveCfg('out', app.cfg);
            refreshQr();
        });
        refreshQr();
    }

    function refreshQr() {
        const sel = PV.ui.el('ip-select');
        const ip = sel ? sel.value : '';
        const urlText = PV.ui.el('url-text');
        const canvas = PV.ui.el('qr-canvas');
        if (!ip) {
            if (urlText) urlText.textContent = '先选一张网卡';
            return;
        }
        const url = inPageUrl(ip);
        if (urlText) urlText.textContent = url;
        try {
            PV.qr.draw(canvas, url, { size: 160 });
        } catch (e) {
            PV.ui.log('二维码生成失败：' + (e && e.message ? e.message : e));
        }
    }

    // ── 订阅远端帧 ──
    function onWsMessage(e) {
        if (typeof e.data === 'string') {
            let msg = null;
            try { msg = JSON.parse(e.data); } catch (err) { /* 心跳外 */ }
            if (msg && msg.type === 'hello') {
                PV.ui.log('已订阅：帧 ' + msg.frameSamples + ' 样本/10ms');
            }
            return;
        }
        if (!app.running || !app.pipeline) return;
        const i16 = new Int16Array(e.data);
        app.rxFrames += i16.length / PV.util.HOP;
        app.pipeline.pushRemoteHop(i16);
    }

    async function start() {
        if (app.running) return;
        if (!PV.Pipeline.supported) {
            PV.ui.state('不支持', 'bad');
            PV.ui.log('当前浏览器不支持 AudioWorklet，无法运行');
            return;
        }
        if (!app.engineReady) {
            const ok = await PV.session.bootEngine();
            if (!ok) return;
            app.engineReady = true;
        }

        let ws;
        try {
            ws = new WebSocket(wsUrl());
            ws.binaryType = 'arraybuffer';
            await new Promise((res, rej) => {
                ws.onopen = res;
                ws.onerror = () => rej(new Error('连不上 ' + wsUrl()));
            });
        } catch (e) {
            PV.ui.state('无服务', 'bad');
            PV.ui.log('订阅失败：' + (e && e.message ? e.message : e));
            return;
        }

        app.pipeline = new PV.Pipeline();
        app.pipeline._onLevel = (peak) => PV.ui.level(PV.util.linearToDb(peak));
        try {
            await app.pipeline.startRemote({
                preGainDb: app.cfg.pre_gain_db,
                postGainDb: app.cfg.post_gain_db,
            });
        } catch (e) {
            PV.ui.state('启动失败', 'bad');
            PV.ui.log('音频图启动失败：' + (e && e.message ? e.message : e));
            try { ws.close(); } catch (err) { /* 已断开 */ }
            app.pipeline = null;
            return;
        }

        const outSel = PV.ui.el('out-select');
        if (outSel && outSel.value && await app.pipeline.setSinkId(outSel.value)) {
            app.cfg.output_device_id = outSel.value;
            app.cfg.output_device = outSel.options[outSel.selectedIndex].textContent;
            PV.util.saveCfg('out', app.cfg);
            PV.ui.log('输出设备：' + app.cfg.output_device);
        }

        app.ws = ws;
        app.rxFrames = 0;
        ws.onmessage = onWsMessage;
        ws.onclose = () => {
            if (app.running) {
                PV.ui.log('与服务器断开');
                stop();
            }
        };

        app.running = true;
        PV.ui.running(true, '停止');
        PV.ui.state('运行中', 'good');
        PV.ui.status('运行中 · 48kHz 单声道 · 降噪常驻');
        PV.ui.log('已订阅 ' + wsUrl() + '，等输入端推流');
    }

    async function stop() {
        if (!app.running) return;
        app.running = false;
        try { if (app.ws) app.ws.close(); } catch (e) { /* 已断开 */ }
        app.ws = null;
        if (app.pipeline) {
            await app.pipeline.stop();
            app.pipeline = null;
        }
        PV.ort.flush();
        PV.ui.running(false, '启动');
        PV.ui.state('已停止', 'idle');
        PV.ui.status('已停止');
        PV.ui.resetLevel();
    }

    function setupDevices() {
        app.outRow = PV.session.setupOutputRow('out', app.cfg,
            (id) => (app.pipeline ? app.pipeline.setSinkId(id) : null));
    }

    function bind() {
        const btn = PV.ui.el('btn-run');
        if (btn) btn.addEventListener('click', () => (app.running ? stop() : start()));
        const copy = PV.ui.el('url-copy');
        if (copy) copy.addEventListener('click', async () => {
            const text = PV.ui.el('url-text').textContent;
            if (!text || text.indexOf('://') < 0) return;
            try {
                await navigator.clipboard.writeText(text);
                PV.ui.log('输入页地址已复制');
            } catch (e) {
                PV.ui.log('剪贴板不可用，请手动复制地址');
            }
        });
        window.addEventListener('beforeunload', () => {
            try { if (app.ws) app.ws.close(); } catch (e) { /* 关闭中 */ }
            PV.ort.dispose();
        });
    }

    async function main() {
        app.cfg = PV.session.init('out');
        PV.ui.debug({ model: A.meta.modelLabel, hop: PV.util.HOP + ' / 10ms' });
        PV.ui.log('PureVox Web（网络输出端）· ' + A.meta.modelLabel +
            ' · ONNX Runtime Web ' + A.meta.ortVersion);
        PV.ui.log('选一张手机能连上的网卡 → 手机扫码打开输入页 → 两边都点启动');
        if (!PV.Pipeline.supported) {
            PV.ui.state('不支持', 'bad');
            PV.ui.log('当前浏览器不支持 AudioWorklet，无法运行');
            return;
        }
        setupDevices();
        bind();
        PV.session.startStats(() => ({
            ws: app.ws ? String(app.ws.readyState) : '--',
            rx: app.rxFrames ? (app.rxFrames / 100).toFixed(1) + ' s' : '--',
        }));
        await app.outRow.refresh();
        await loadLan();
        // 主角色：进页面先把引擎热起来，点启动即出声
        PV.session.bootEngine().then((ok) => {
            app.engineReady = ok;
            if (ok && app.running) {
                PV.ui.state('运行中', 'good');
                PV.ui.status('运行中 · 48kHz 单声道 · 降噪常驻');
            }
        });
    }

    window.addEventListener('DOMContentLoaded', main);
})(window.PV = window.PV || {});
