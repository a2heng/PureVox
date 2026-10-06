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

// 会话层（两个 flavor 共用）：配置读写、增益事件、引擎引导、
// 设备行（输入=麦克风 / 输出=扬声器）、统计刷新。
// Mic / Net 的差异只在「音源从哪来」与连接流程上。
(function (PV) {
    'use strict';

    function init(flavor) {
        const cfg = PV.util.loadCfg(flavor);
        PV.ui.setGain('pre', cfg.pre_gain_db);
        PV.ui.setGain('post', cfg.post_gain_db);
        PV.ui.init();

        // 安全上下文门槛：浏览器只在安全上下文（HTTPS 或 localhost）里给麦克风
        // 权限、才允许 WebRTC。直接双击单 HTML（file://）或用局域网 http 打开
        // 都会踩这个坑——先把话说清楚，别让人对着没反应的按钮猜。
        if (!window.isSecureContext) {
            PV.ui.state('需 HTTPS', 'bad');
            PV.ui.log('当前不是安全上下文：麦克风与 WebRTC 都不会授权。');
            PV.ui.log('请改用 HTTPS 打开：python purevox-web/serve.py（会打印本机与局域网地址）');
        }

        document.addEventListener('pv-gain', (e) => {
            const { which, value } = e.detail;
            cfg[which === 'pre' ? 'pre_gain_db' : 'post_gain_db'] = value;
            PV.util.saveCfg(flavor, cfg);
            if (PV.app && PV.app.pipeline) {
                if (which === 'pre') PV.app.pipeline.setPreGainDb(value);
                else PV.app.pipeline.setPostGainDb(value);
            }
        });
        return cfg;
    }

    // 引擎引导：先起 worker，再解三件资源（每步让出一帧刷新进度文案）
    async function bootEngine() {
        PV.ui.state('加载中', 'busy');
        try {
            await PV.ort.boot((stage) => PV.ui.state(stage, 'busy'));
            return true;
        } catch (e) {
            PV.ui.state('引擎失败', 'bad');
            PV.ui.log('引擎加载失败：' + (e && e.message ? e.message : e));
            return false;
        }
    }

    // ── 设备枚举 ──
    // 浏览器的隐私规则：未授予设备权限时，条目的 deviceId 与 label 都是空串，
    // 空 deviceId 不可用（setSinkId 会失败），一律过滤掉——宁可显示「未解锁」
    // 也不给一个点了就报错的假下拉。
    // kind 严格过滤：输入行只可能出现麦克风，输出行只可能出现扬声器。
    async function listDevices(kind) {
        if (!navigator.mediaDevices || !navigator.mediaDevices.enumerateDevices) return [];
        const all = await navigator.mediaDevices.enumerateDevices();
        return all.filter((d) => d.kind === kind && d.deviceId);
    }

    // Chrome 会额外给出 default / communications 两个伪设备，标注清楚，
    // 免得和真设备混在一起看不出哪个是系统默认。
    function deviceText(d, i, fallback) {
        if (d.deviceId === 'default') return '默认（系统）';
        if (d.deviceId === 'communications') return '默认（通信）';
        return d.label || ((fallback || '设备') + ' ' + (i + 1));
    }

    // 恢复已选：先按 deviceId，再按 label（deviceId 每源加盐，label 更耐重启）
    function fillSelect(sel, items, savedId, savedLabel, placeholder, fallback) {
        if (!sel) return '';
        sel.innerHTML = '';
        if (placeholder) {
            const o0 = document.createElement('option');
            o0.value = '';
            o0.textContent = placeholder;
            sel.appendChild(o0);
        }
        items.forEach((d, i) => {
            const o = document.createElement('option');
            o.value = d.deviceId;
            o.textContent = deviceText(d, i, fallback);
            sel.appendChild(o);
        });
        if (savedId && items.some((d) => d.deviceId === savedId)) {
            sel.value = savedId;
        } else if (savedLabel) {
            const hit = items.find((d) => d.label === savedLabel);
            if (hit) sel.value = hit.deviceId;
        }
        if (sel.selectedIndex < 0 && sel.options.length) sel.selectedIndex = 0;
        return sel.value;
    }

    // 只借权限、不录音：拿到设备权限后立刻释放轨道。
    // 浏览器没有「扬声器权限」这种概念——输出不是隐私受限资源，
    // AudioContext.setSinkId 无需授权；只是设备 label/列表要先有任意设备
    // 权限才可见（实测 Chrome：麦克风授权后 audiooutput 才带 deviceId）。
    async function unlockDeviceLabels() {
        try {
            const s = await navigator.mediaDevices.getUserMedia({ audio: true });
            s.getTracks().forEach((t) => t.stop());
            return true;
        } catch (e) {
            return false;
        }
    }

    // 已注册设备行的 refresh 队列：任何一次解锁都要把两行一起刷新
    const registered = [];

    async function refreshAll() {
        for (const fn of registered) await fn();
    }

    // 设备行唯一实现（输入行与输出行共用）：
    //   kind      audioinput / audiooutput —— 保证列表里只有麦克风或扬声器
    //   selId/unlockId/rowId  模板里的元素 id
    //   saved     [cfg 里的 deviceId 键, cfg 里的 label 键]
    //   placeholder 未解锁时的占位项；lockedText 未解锁时列表为空时的文案
    //   unlockLabel 解锁按钮文案（输出行刻意不叫「授权」，见 setupOutputRow）
    //   apply     选中变化后的回调（输出行用来 setSinkId）
    //   supported 额外的可用性判断（输出行要求 AudioContext.setSinkId）
    function setupDeviceRow(o) {
        const sel = PV.ui.el(o.selId);
        const unlock = PV.ui.el(o.unlockId);
        const row = PV.ui.el(o.rowId);
        const cfg = o.cfg;

        const refresh = async () => {
            if (row && o.supported && !o.supported()) {
                if (row.style.display !== 'none') {
                    row.style.display = 'none';
                    PV.ui.log(o.unsupportedMsg);
                }
                return false;
            }
            const items = await listDevices(o.kind);
            if (!items.length) {
                fillSelect(sel, [], '', '', o.lockedText, o.fallback);
                if (sel) sel.disabled = true;
                if (unlock) {
                    unlock.style.display = '';
                    if (o.unlockLabel) unlock.textContent = o.unlockLabel;
                    if (o.unlockHint) unlock.title = o.unlockHint;
                }
                return false;
            }
            if (sel) sel.disabled = false;
            if (unlock) unlock.style.display = 'none';
            fillSelect(sel, items, cfg[o.saved[0]], cfg[o.saved[1]], o.placeholder, o.fallback);
            return true;
        };
        registered.push(refresh);

        if (unlock) {
            unlock.addEventListener('click', async () => {
                PV.ui.log(o.unlockLog);
                if (await unlockDeviceLabels()) {
                    PV.ui.log('已授权，设备列表已解锁');
                    await refreshAll();
                } else {
                    PV.ui.log('未获得设备权限，设备列表保持为空');
                }
            });
        }
        if (sel) {
            sel.addEventListener('change', async () => {
                cfg[o.saved[0]] = sel.value;
                cfg[o.saved[1]] = sel.options[sel.selectedIndex].textContent;
                PV.util.saveCfg(o.flavor, cfg);
                if (!sel.value) {
                    PV.ui.log(o.name + '：系统默认');
                    return;
                }
                if (o.apply) await o.apply(sel.value);
                PV.ui.log(o.name + '：' + cfg[o.saved[1]]);
            });
        }
        return { refresh };
    }

    // 输入行：只列麦克风。这里的「授权」就是字面意义的麦克风权限。
    function setupInputRow(flavor, cfg) {
        return setupDeviceRow({
            flavor, cfg,
            kind: 'audioinput',
            selId: 'in-select', unlockId: 'in-unlock', rowId: 'in-row',
            saved: ['input_device_id', 'input_device'],
            placeholder: null, lockedText: '麦克风（未授权，不可见）',
            fallback: '麦克风', name: '输入设备',
            unlockLabel: '授权', unlockHint: '授予麦克风权限',
            unlockLog: '请求麦克风权限…',
        });
    }

    // 输出行：只列扬声器（Chrome/Edge 110+ 的 AudioContext.setSinkId）。
    //
    // 平台事实（Chrome 实测）：浏览器**没有**「扬声器权限」这种东西——
    // selectAudioOutput() 已移除，speaker-selection 权限不弹窗；而
    // enumerateDevices() 会把扬声器条目和麦克风条目一起锁在麦克风权限
    // 后面（未授权时两者 deviceId/label 全空）。所以这里不叫「授权」而叫
    // 「解锁」：它借的确实是麦克风权限，但只为拿到设备名，不录音。
    function setupOutputRow(flavor, cfg, applySink) {
        const hint = '浏览器没有独立的扬声器权限：扬声器列表与麦克风共用一次设备授权，'
            + '点此授权麦克风即可看到扬声器（不录音）';
        const row = setupDeviceRow({
            flavor, cfg,
            kind: 'audiooutput',
            selId: 'out-select', unlockId: 'out-unlock', rowId: 'out-row',
            saved: ['output_device_id', 'output_device'],
            placeholder: '系统默认', lockedText: '扬声器（需麦克风权限后才可见）',
            fallback: '扬声器', name: '输出设备',
            unlockLabel: '解锁', unlockHint: hint,
            unlockLog: '浏览器无独立扬声器权限，此处借用麦克风权限解锁设备列表（不录音）…',
            supported: () => PV.Pipeline.sinkSelectable,
            unsupportedMsg: '当前浏览器不支持输出设备选择（AudioContext.setSinkId），播放到系统默认输出',
            apply: applySink,
        });
        if (!PV.Pipeline.sinkSelectable) {
            PV.ui.log('扬声器无法选择：浏览器不支持 AudioContext.setSinkId，将播放到系统默认输出');
        }
        return row;
    }

    // 统计刷新（250ms）：调试网格
    function startStats(extra) {
        setInterval(() => {
            const p = PV.app && PV.app.pipeline;
            if (!p) return;
            const st = p._stat;
            const o = PV.ort.stats();
            const rows = {
                'ctx-sr': p.ctx ? p.ctx.sampleRate + ' Hz' : '--',
                actx: p._stat.actx || '--',
                ratio: Math.abs(st.ratio - 1) < 1e-6 ? '直通' : st.ratio.toFixed(4),
                infer: o.avg ? o.avg.toFixed(2) + ' ms' : '--',
                wm: st.watermark.toFixed(1) + ' hop',
                under: String(st.under),
                drop: String(st.dropped + st.qdrop),
                lat: p.latencyMs() + ' ms',
            };
            if (extra) Object.assign(rows, extra());
            PV.ui.debug(rows);
        }, 250);
    }

    // 对外只暴露 flavor 装配真正要用的五个入口；设备枚举 / 下拉填充 /
    // 解锁都在本模块内闭环，不外泄成第二套实现。
    PV.session = {
        init, bootEngine, setupInputRow, setupOutputRow, startStats,
    };
})(window.PV = window.PV || {});
