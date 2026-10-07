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

// 共用 UI 外壳：主题（跟随系统/亮/暗）、日志窗、三段电平表、状态行、
// 调试网格。样式沿用 html/ 推流客户端的复古极简配色（--vu-* 变量），
// 控件排布对齐 Lite 桌面端（输入/输出/前增益/后增益）。
(function (PV) {
    'use strict';

    const THEME_KEY = 'purevox_theme';   // 与 html/ 客户端共用同一偏好键

    function el(id) {
        return document.getElementById(id);
    }

    function setText(id, text) {
        const e = el(id);
        if (e) e.textContent = text;
    }

    function setHtml(id, html) {
        const e = el(id);
        if (e) e.innerHTML = html;
    }

    // ── 主题：◐ 跟随系统 / ☀ 亮色 / ☾ 深色（循环切换）──
    function applyTheme() {
        const root = document.documentElement;
        const saved = localStorage.getItem(THEME_KEY);
        if (saved !== null) root.className = saved === 'light' ? 'theme-light'
            : saved === 'dark' ? 'theme-dark' : '';
        updateThemeIcon();
    }

    function updateThemeIcon() {
        const cls = document.documentElement.className;
        const btn = el('theme-toggle');
        if (!btn) return;
        btn.textContent = !cls ? '◐' : cls === 'theme-light' ? '☀' : '☾';
        btn.title = !cls ? '跟随系统' : cls === 'theme-light' ? '亮色' : '深色';
    }

    function initTheme() {
        applyTheme();
        const btn = el('theme-toggle');
        if (btn) {
            btn.addEventListener('click', () => {
                const cur = document.documentElement.className;
                const next = !cur ? 'light' : cur === 'theme-light' ? 'dark' : '';
                document.documentElement.className =
                    next === 'light' ? 'theme-light' : next === 'dark' ? 'theme-dark' : '';
                localStorage.setItem(THEME_KEY, next);
                updateThemeIcon();
            });
        }
        const mq = window.matchMedia('(prefers-color-scheme: dark)');
        if (mq.addEventListener) mq.addEventListener('change', updateThemeIcon);
    }

    // ── 日志（常驻，最多 200 行）──
    function log(msg) {
        const box = el('log-content');
        if (!box) return;
        const line = document.createElement('div');
        line.className = 'log-line';
        line.textContent = '[' + PV.util.nowStamp() + '] ' + msg;
        box.appendChild(line);
        while (box.children.length > 200) box.removeChild(box.firstChild);
        box.scrollTop = box.scrollHeight;
    }

    // ── 状态行（Lite 同文案口径）──
    function status(text) {
        setText('status-text', text);
    }

    function state(text, kind) {
        const e = el('state-pill');
        if (!e) return;
        e.textContent = text;
        e.className = 'pill ' + (kind || 'idle');
    }

    // ── 三段电平表：绿(0~-20dB) 黄(-20~-5) 红(-5~0)，带 3s 回落峰值 ──
    const PEAK_DECAY = 2;      // dB/s
    let peakDb = -60;
    let peakAt = 0;

    function level(db) {
        const pct = PV.util.clamp((db + 60) / 60 * 100, 0, 99);
        const f1 = el('vu-fill1');
        const f2 = el('vu-fill2');
        const f3 = el('vu-fill3');
        if (f1) f1.style.width = Math.min(pct, 66.67) + '%';
        if (f2) f2.style.width = Math.max(0, Math.min(pct - 66.67, 18.33)) + '%';
        if (f3) f3.style.width = Math.max(0, Math.min(pct - 85, 15)) + '%';

        const now = performance.now();
        if (db > peakDb) { peakDb = db; peakAt = now; }
        else if (now - peakAt > 3000) {
            peakDb = Math.max(-60, peakDb - PEAK_DECAY * (now - peakAt) / 1000);
            peakAt = now;
        }
        const mark = el('vu-peak');
        if (mark) {
            if (peakDb > -59) {
                const pp = PV.util.clamp((peakDb + 60) / 60 * 100, 0, 99);
                mark.style.left = pp + '%';
                const cs = getComputedStyle(document.documentElement);
                const color = pp <= 66.67 ? cs.getPropertyValue('--vu-fill1')
                    : pp <= 85 ? cs.getPropertyValue('--vu-fill2')
                        : cs.getPropertyValue('--vu-fill3');
                mark.style.background = (color || '').trim() || 'var(--text)';
                mark.style.display = 'block';
            } else {
                mark.style.display = 'none';
            }
        }
    }

    function resetLevel() {
        peakDb = -60;
        peakAt = 0;
        level(-60);
    }

    // ── 调试网格：{键: 值} 批量刷新 ──
    function debug(pairs) {
        for (const k in pairs) {
            const e = el('dbg-' + k);
            if (e) e.textContent = pairs[k];
        }
    }

    // ── 启停主按钮（横条）──
    function running(on, text) {
        const btn = el('btn-run');
        if (!btn) return;
        btn.classList.toggle('active', !!on);
        const icon = btn.querySelector('.strip-icon');
        const label = el('strip-text');
        if (icon) icon.textContent = on ? '■' : '▶';
        if (label) label.textContent = text || (on ? '停止' : '启动');
    }

    // ── 增益步进器（− / 数值 / +，对齐 Lite 桌面端）──
    function gainStepper(which, delta) {
        const box = el('gain-' + which);
        if (!box) return;
        const cur = PV.util.clamp(parseInt(box.textContent, 10) + delta, -20, 30);
        box.textContent = String(cur);
        const ev = new CustomEvent('pv-gain', { detail: { which, value: cur } });
        document.dispatchEvent(ev);
    }

    function setGain(which, db) {
        setText('gain-' + which, String(db));
    }

    function bindGain() {
        for (const which of ['pre', 'post']) {
            const dec = el('gain-' + which + '-dec');
            const inc = el('gain-' + which + '-inc');
            if (dec) dec.addEventListener('click', () => gainStepper(which, -1));
            if (inc) inc.addEventListener('click', () => gainStepper(which, 1));
        }
    }

    function init() {
        initTheme();
        bindGain();
        const logBox = el('log-section');
        if (logBox) {
            const title = el('log-title');
            logBox.addEventListener('click', () => {
                const hidden = logBox.classList.toggle('collapsed');
                if (title) title.textContent = hidden ? '▸ 日志（点击展开）' : '▾ 日志';
            });
        }
        resetLevel();
    }

    PV.ui = {
        el, setText, setHtml, log, status, state, level, resetLevel,
        debug, running, setGain, init,
    };
})(window.PV = window.PV || {});
