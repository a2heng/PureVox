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

// ORT 推理桥（主线程侧）：worker 由内联的 worker 源码 blob 化启动。
// 两件重资源（ONNX 运行时 wasm + 降噪模型）**不内联进 HTML**，按 URL 并行
// 下载后 transfer 给 worker（零拷贝）——base64 内联会凭空多 33% 体积，
// 且每次改页面都要重传 20MB；拆开后它们能被浏览器/CDN 长期缓存。
// 同一时刻只允许一跳在途：天然背压，Worker 慢于实时不会堆积无界队列。
(function (PV) {
    'use strict';

    const A = PV.ASSETS;

    let worker = null;
    let ready = false;
    let seq = 0;
    const waiters = new Map();
    const msRing = new Float32Array(32);
    let msIdx = 0;
    let msCount = 0;
    let bootResolve = null;
    let bootReject = null;

    function stats() {
        if (msCount === 0) return { avg: 0, last: 0 };
        let sum = 0;
        for (let i = 0; i < msCount; i++) sum += msRing[i];
        return { avg: sum / msCount, last: msRing[(msIdx - 1 + msRing.length) % msRing.length] };
    }

    // 带进度的下载：有 Content-Length 就逐步报，没有就一次性给 unknown。
    async function fetchBytes(url, label, onProgress) {
        let resp;
        try {
            resp = await fetch(url, { cache: 'force-cache' });
        } catch (e) {
            throw new Error(label + '下载失败（' + url + '）：' +
                (e && e.message ? e.message : e) +
                '。若是 file:// 打开的，请改用 python purevox-web/serve.py');
        }
        if (!resp.ok) {
            throw new Error(label + '下载失败：HTTP ' + resp.status + ' ' + url);
        }
        const len = Number(resp.headers.get('content-length')) || 0;
        if (!resp.body || !len) {
            const buf = await resp.arrayBuffer();
            onProgress(label, buf.byteLength, buf.byteLength, true);
            return new Uint8Array(buf);
        }
        const reader = resp.body.getReader();
        const chunks = [];
        let got = 0;
        for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
            got += value.length;
            onProgress(label, got, len, false);
        }
        const out = new Uint8Array(got);
        let off = 0;
        for (const c of chunks) { out.set(c, off); off += c.length; }
        onProgress(label, got, got, true);
        return out;
    }

    function boot(onStage) {
        if (ready) return Promise.resolve();
        const loaded = {};
        const totals = { ort: 0, model: 0 };
        function report(label, got, len, done) {
            if (label === 'ORT 运行时') totals.ort = got; else totals.model = got;
            const sum = totals.ort + totals.model;
            if (done && sum === 0) return;
            onStage('下载 ' + (sum / 1048576).toFixed(1) + ' MiB');
        }

        onStage('连接资源');
        return Promise.resolve().then(() => {
            const srcBlob = new Blob([PV.util.b64ToText(A.workerOrtB64)], { type: 'text/javascript' });
            worker = new Worker(URL.createObjectURL(srcBlob));
            worker.onmessage = onMessage;
            worker.onerror = (e) => fail('Worker 错误：' + (e.message || '未知'));
        }).then(() => Promise.all([
            fetchBytes(A.ortWasmUrl, 'ORT 运行时', report).then((b) => {
                loaded.wasm = b;
                return fetchText(A.ortGlueUrl);
            }),
            fetchBytes(A.modelUrl, '模型', report).then((b) => { loaded.model = b; }),
        ])).then(([glueText]) => {
            onStage('初始化引擎');
            return new Promise((res, rej) => {
                bootResolve = res;
                bootReject = rej;
                const wasmBuf = loaded.wasm.buffer;
                const modelBuf = loaded.model.buffer;
                worker.postMessage({ type: 'boot', glue: glueText, wasm: wasmBuf, model: modelBuf },
                    [wasmBuf, modelBuf]);
            });
        }).then((info) => {
            onStage('就绪');
            PV.ui.log('模型 ' + A.meta.modelLabel + ' · 引擎初始化 ' + Math.round(info.createMs) +
                'ms · cache ' + info.cacheDims[1]);
            return info;
        });
    }

    function fetchText(url) {
        return fetchBytes(url, 'ORT 胶水', () => { }).then(
            (b) => new TextDecoder('utf-8').decode(b));
    }

    function fail(msg) {
        PV.ui.log(msg);
        if (bootReject) {
            const r = bootReject;
            bootReject = null;
            bootResolve = null;
            r(new Error(msg));
        }
        for (const [, w] of waiters) w.rej(new Error(msg));
        waiters.clear();
    }

    function onMessage(e) {
        const m = e.data;
        if (m.type === 'ready') {
            ready = true;
            if (bootResolve) {
                const r = bootResolve;
                bootResolve = null;
                bootReject = null;
                r(m);
            }
            return;
        }
        if (m.type === 'error') {
            fail(m.msg);
            return;
        }
        if (m.type === 'out') {
            msRing[msIdx] = m.ms;
            msIdx = (msIdx + 1) % msRing.length;
            if (msCount < msRing.length) msCount++;
            const w = waiters.get(m.seq);
            if (w) {
                waiters.delete(m.seq);
                w.res(new Float32Array(m.data));
            }
        }
    }

    // 单跳推理：入参 hop（Float32Array 480，转移所有权），出参 enh_hop。
    function infer(hop, preGain) {
        if (!ready) return Promise.reject(new Error('推理引擎未就绪'));
        const id = ++seq;
        const buf = hop.buffer;
        return new Promise((res, rej) => {
            waiters.set(id, { res, rej });
            worker.postMessage({ type: 'hop', seq: id, data: buf, pre: preGain || 1 }, [buf]);
        });
    }

    // 重新开始一段会话：模型 cache 归零（与桌面端引擎 reset 同语义）。
    function flush() {
        if (worker && ready) worker.postMessage({ type: 'flush' });
    }

    function dispose() {
        if (worker) {
            worker.terminate();
            worker = null;
        }
        ready = false;
        waiters.clear();
    }

    PV.ort = { boot, infer, flush, dispose, stats, get ready() { return ready; } };
})(window.PV = window.PV || {});
