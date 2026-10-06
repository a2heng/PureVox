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

// 推理 Worker（Web Worker 作用域；以 blob URL 启动，源码由单 HTML 内联）。
// 契约与桌面端 LiteDenoiseEngine 完全一致：波形 hop 进出（480 = 10ms @48k）、
// STFT 在模型图内、enh_hop 滞后 1 hop、cache 扁平形状按模型输入自适应。
// 前增益在进模型前施加（对齐 Lite「前增益 → 引擎」）。
// 同一时刻只处理一跳：主线程侧一次只发一跳，天然背压。
'use strict';

const HOP = 480;               // 10ms @48kHz（按时间派生，非固定经验值）

let ort = null;
let session = null;
let cache = null;

function post(msg, transfer) {
    self.postMessage(msg, transfer || []);
}

async function boot(m) {
    const t0 = performance.now();
    // ORT 胶水是 ESM：以 blob URL 动态 import（单文件内联，无外部相对路径）
    const url = URL.createObjectURL(new Blob([m.glue], { type: 'text/javascript' }));
    const mod = await import(url);
    URL.revokeObjectURL(url);
    ort = mod.default;
    // 单线程：单 HTML 部署拿不到跨源隔离（COOP/COEP），numThreads=1 即可，
    // 实测 b 档 ~1.5ms/hop（预算 10ms），富余 6 倍。
    ort.env.wasm.numThreads = 1;
    ort.env.wasm.wasmBinary = m.wasm;   // ArrayBuffer，免 fetch
    ort.env.logLevel = 'error';

    session = await ort.InferenceSession.create(new Uint8Array(m.model), {
        executionProviders: ['wasm'],
    });
    const meta = session.inputMetadata.find((x) => x.name === 'cache_in');
    if (!meta) throw new Error('模型缺少 cache_in 输入（契约不符）');
    resetCache(meta.shape);

    post({
        type: 'ready',
        createMs: performance.now() - t0,
        cacheDims: meta.shape,
        inputNames: session.inputNames,
        outputNames: session.outputNames,
    });
}

function resetCache(dims) {
    const d = dims || (cache ? Array.from(cache.dims) : null) || [1, 15306];
    cache = new ort.Tensor('float32', new Float32Array(d[0] * d[1]), d);
}

function fail(msg) {
    post({ type: 'error', msg: String((msg && msg.stack) || msg) });
}

self.onmessage = async (e) => {
    const m = e.data;
    try {
        if (m.type === 'boot') {
            await boot(m);
        } else if (m.type === 'hop') {
            if (!session) throw new Error('推理引擎未就绪');
            let x = new Float32Array(m.data);
            if (m.pre !== 1) {
                for (let i = 0; i < x.length; i++) x[i] *= m.pre;
            }
            const t0 = performance.now();
            const res = await session.run({
                mix_hop: new ort.Tensor('float32', x, [1, HOP]),
                cache_in: cache,
            });
            const ms = performance.now() - t0;
            cache = res.cache_out;
            // ORT 张量的 data 可能是 wasm 堆视图，必须复制后才能转移
            const out = new Float32Array(res.enh_hop.data);
            post({ type: 'out', seq: m.seq, ms, data: out.buffer }, [out.buffer]);
        } else if (m.type === 'flush') {
            resetCache();
        }
    } catch (err) {
        fail((err && err.message) || err);
    }
};
