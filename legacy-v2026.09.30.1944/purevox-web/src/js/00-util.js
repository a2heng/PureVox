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

// 共用基础件：时间网格常量、base64 解码、dB 换算、localStorage 配置、日志。
// 全部内容由 purevox-web/build/build_web.py 以 base64 注入单 HTML，无外部依赖。
(function (PV) {
    'use strict';

    // ── 10ms hop 规约（与桌面引擎/模型契约一致，按时间派生）──
    const SAMPLE_RATE = 48000;      // 引擎内部唯一格式：F32 单声道 48kHz
    const HOP = SAMPLE_RATE / 100;   // 480 样本 = 10ms
    const HOP_MS = 10;

    function clamp(v, lo, hi) {
        return v < lo ? lo : (v > hi ? hi : v);
    }

    // base64 → UTF-8 文本（内联的 worker / worklet 源码；必须按 UTF-8 解码，
    // 直接 atob 会把非 ASCII 字符读成乱码）。重资源走 URL，不进 base64。
    function b64ToText(b64) {
        const bin = atob(b64);
        const n = bin.length;
        const bytes = new Uint8Array(n);
        for (let i = 0; i < n; i++) bytes[i] = bin.charCodeAt(i);
        return new TextDecoder('utf-8').decode(bytes);
    }

    function dbToLinear(db) {
        return Math.pow(10, db / 20);
    }

    function linearToDb(v) {
        return 20 * Math.log10(Math.max(v, 1e-7));
    }

    function nowStamp() {
        const d = new Date();
        const p = (n) => String(n).padStart(2, '0');
        return p(d.getHours()) + ':' + p(d.getMinutes()) + ':' + p(d.getSeconds());
    }

    // ── 配置（localStorage，强配置：只认已知键，缺失回默认）──
    // 增益区间与 Lite 一致（-20~30 dB）。
    const CFG_STORE = {
        mic: 'purevox_web_mic_cfg',
        in: 'purevox_web_in_cfg',
        out: 'purevox_web_out_cfg',
    };

    const CFG_SCHEMA = {
        pre_gain_db: { def: 0, lo: -20, hi: 30 },
        post_gain_db: { def: 0, lo: -20, hi: 30 },
        input_device: { def: '' },      // 设备名（跨重启恢复用）
        input_device_id: { def: '' },   // deviceId（每源加盐，同源内有效）
        output_device: { def: '' },
        output_device_id: { def: '' },
        lan_ip: { def: '' },            // 输出页选中的局域网 IP（二维码按它生成）
        srv: { def: '' },               // 中继服务器 host:port（跨站部署用，
                                        // 如 github.io 页连回局域网 serve.py；
                                        // 为空 = 与本页同源）
    };

    function loadCfg(flavor) {
        const out = {};
        let raw = null;
        try {
            raw = JSON.parse(localStorage.getItem(CFG_STORE[flavor]) || '{}');
        } catch (e) {
            raw = {};
        }
        for (const key in CFG_SCHEMA) {
            const spec = CFG_SCHEMA[key];
            let v = raw[key];
            if (v === undefined || v === null) v = spec.def;
            if (spec.lo !== undefined) {
                const iv = parseInt(v, 10);
                v = Number.isFinite(iv) ? clamp(iv, spec.lo, spec.hi) : spec.def;
            } else {
                v = String(v);
            }
            out[key] = v;
        }
        return out;
    }

    function saveCfg(flavor, cfg) {
        try {
            const keep = {};
            for (const key in CFG_SCHEMA) keep[key] = cfg[key];
            localStorage.setItem(CFG_STORE[flavor], JSON.stringify(keep));
        } catch (e) {
            /* 隐私模式等写不进去：忽略，不影响运行 */
        }
    }

    // ── 中继服务器地址解析（跨站部署用）──
    // 页面默认与 serve.py 同源（srv 为空）。部署到 github.io 这类静态站后，
    // 页面与中继不在同源，必须显式指定：`?srv=192.168.0.105:59124`（进页面即
    // 记住），或在页面上的服务器输入框里改。scheme 缺省跟页面走（https 页
    // 默认 wss/https），也可写全 `https://host:port`。
    function resolveServer(flavor, cfg) {
        let q = '';
        try { q = (new URLSearchParams(location.search).get('srv') || '').trim(); }
        catch (e) { q = ''; }
        if (q) {
            cfg.srv = q;
            saveCfg(flavor, cfg);
        }
        const raw = (cfg.srv || '').trim();
        if (!raw) {
            const wsProto = location.protocol === 'https:' ? 'wss:' : 'ws:';
            return { srv: '', httpBase: '', wsBase: wsProto + '//' + location.host, remote: false };
        }
        let scheme = location.protocol === 'https:' ? 'https:' : 'http:';
        let host = raw;
        const m = raw.match(/^(https?):\/\/(.+)$/i);
        if (m) {
            scheme = m[1].toLowerCase() + ':';
            host = m[2].replace(/\/+$/, '');
        }
        host = host.replace(/\/+$/, '');
        return {
            srv: host,
            httpBase: scheme + '//' + host,
            wsBase: (scheme === 'https:' ? 'wss:' : 'ws:') + '//' + host,
            remote: true,
        };
    }

    PV.util = {
        SAMPLE_RATE, HOP, HOP_MS,
        clamp, b64ToText, dbToLinear, linearToDb,
        nowStamp, loadCfg, saveCfg, resolveServer,
    };
})(window.PV = window.PV || {});
