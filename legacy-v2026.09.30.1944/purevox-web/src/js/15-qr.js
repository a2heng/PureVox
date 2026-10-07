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

// 页面内二维码：byte 模式 + 纠错等级 M，版本 1~10 自选。
//
// 为什么自带编码器而不引第三方库、也不让服务端生成：
//   · 页面要能当**单 HTML** 产物部署到 GitHub Pages —— 那时没有服务端可问；
//   · 二维码的内容是一个 URL（三十来字符），页面里换 IP 下拉就该立刻重画，
//     走一次网络往返没有意义。
// 版本 1~10 的纠错 M 容量是 14~213 字节，URL 绰绰有余。
(function (PV) {
    'use strict';

    // ── 每个版本的纠错块结构（纠错等级 M）──
    // [每块纠错码字数, 块数]，其余按 Nayuki 的表推导总码字数，
    // 与 ISO/IEC 18004 一致（v1..v10 的数据码字 = 16/28/44/64/86/108/124/154/182/216）。
    const ECC_PER_BLOCK = [10, 16, 26, 18, 24, 16, 18, 22, 22, 26];
    const NUM_BLOCKS = [1, 1, 1, 2, 2, 4, 4, 4, 5, 5];
    const MAX_VERSION = ECC_PER_BLOCK.length;
    const FORMAT_BITS_M = 0;          // 纠错等级 M 在格式信息里的 2 bit
    const PENALTY = { N1: 3, N2: 3, N3: 40, N4: 10 };

    function rawDataModules(ver) {
        let n = (16 * ver + 128) * ver + 64;
        if (ver >= 2) {
            const numAlign = Math.floor(ver / 7) + 2;
            n -= (25 * numAlign - 10) * numAlign - 55;
            if (ver >= 7) n -= 36;
        }
        return n;                     // bit 数（除 8 得码字数）
    }

    function alignPositions(ver) {
        if (ver === 1) return [];
        const num = Math.floor(ver / 7) + 2;
        const step = Math.ceil((ver * 4 + 4) / (num * 2 - 2)) * 2;
        const out = [6];
        for (let pos = ver * 4 + 10; out.length < num; pos -= step) out.splice(1, 0, pos);
        return out;
    }

    // ── GF(256) 上的 Reed-Solomon ──
    function gfMul(x, y) {
        let z = 0;
        for (let i = 7; i >= 0; i--) {
            z = (z << 1) ^ ((z >>> 7) * 0x11D);
            z ^= ((y >>> i) & 1) * x;
        }
        return z & 0xFF;
    }

    function rsDivisor(degree) {
        const result = new Array(degree - 1).fill(0);
        result.push(1);
        let root = 1;
        for (let i = 0; i < degree; i++) {
            for (let j = 0; j < result.length; j++) {
                result[j] = gfMul(result[j], root);
                if (j + 1 < result.length) result[j] ^= result[j + 1];
            }
            root = gfMul(root, 0x02);
        }
        return result;
    }

    function rsRemainder(data, divisor) {
        const result = new Array(divisor.length).fill(0);
        for (const b of data) {
            const factor = b ^ result.shift();
            result.push(0);
            for (let i = 0; i < divisor.length; i++) result[i] ^= gfMul(divisor[i], factor);
        }
        return result;
    }

    // 加纠错码并交织成最终码字序列
    function addEccAndInterleave(ver, data) {
        const numBlocks = NUM_BLOCKS[ver - 1];
        const eccLen = ECC_PER_BLOCK[ver - 1];
        const rawCodewords = Math.floor(rawDataModules(ver) / 8);
        const numShort = numBlocks - (rawCodewords % numBlocks);
        const shortLen = Math.floor(rawCodewords / numBlocks);
        const divisor = rsDivisor(eccLen);
        const blocks = [];
        for (let i = 0, k = 0; i < numBlocks; i++) {
            const dat = data.slice(k, k + shortLen - eccLen + (i < numShort ? 0 : 1));
            k += dat.length;
            const ecc = rsRemainder(dat, divisor);
            if (i < numShort) dat.push(0);       // 短块补一个占位，交织时跳过
            blocks.push(dat.concat(ecc));
        }
        const out = [];
        for (let i = 0; i < blocks[0].length; i++) {
            for (let j = 0; j < blocks.length; j++) {
                if (i !== shortLen - eccLen || j >= numShort) out.push(blocks[j][i]);
            }
        }
        return out;
    }

    // ── 位流 → 码字（byte 模式：0100 + 长度 + 数据 + 终止符 + 填充）──
    function buildCodewords(bytes, ver) {
        // 数据容量 = 总码字 − 纠错码字（之前误用总数当数据容量，版本会选小一档）
        const totalCodewords = Math.floor(rawDataModules(ver) / 8);
        const dataCapacity = totalCodewords - NUM_BLOCKS[ver - 1] * ECC_PER_BLOCK[ver - 1];
        const capacityBits = dataCapacity * 8;
        const cci = ver < 10 ? 8 : 16;           // 长度字段位数
        const need = 4 + cci + bytes.length * 8;
        if (need > capacityBits) return null;

        const bits = [];
        const push = (val, len) => {
            for (let i = len - 1; i >= 0; i--) bits.push((val >>> i) & 1);
        };
        push(0b0100, 4);
        push(bytes.length, cci);
        for (const b of bytes) push(b, 8);
        for (let i = 0; i < 4 && bits.length < capacityBits; i++) bits.push(0);   // 终止符
        while (bits.length % 8 !== 0) bits.push(0);
        const codewords = [];
        for (let i = 0; i < bits.length; i += 8) {
            let v = 0;
            for (let j = 0; j < 8; j++) v = (v << 1) | bits[i + j];
            codewords.push(v);
        }
        for (let pad = 0xEC; codewords.length < dataCapacity; pad ^= 0xEC ^ 0x11) {
            codewords.push(pad);
        }
        return addEccAndInterleave(ver, codewords);
    }

    // ── 矩阵绘制 ──
    function Grid(size) {
        this.size = size;
        this.modules = [];
        this.fn = [];
        for (let y = 0; y < size; y++) {
            this.modules.push(new Array(size).fill(false));
            this.fn.push(new Array(size).fill(false));
        }
    }
    Grid.prototype.set = function (x, y, dark) {
        this.modules[y][x] = dark;
        this.fn[y][x] = true;
    };
    Grid.prototype.get = function (x, y) { return this.modules[y][x]; };

    function drawFunctionPatterns(g, ver) {
        const size = g.size;
        for (let i = 0; i < size; i++) {
            g.set(6, i, i % 2 === 0);
            g.set(i, 6, i % 2 === 0);
        }
        finder(g, 3, 3);
        finder(g, size - 4, 3);
        finder(g, 3, size - 4);
        const pos = alignPositions(ver);
        for (let i = 0; i < pos.length; i++) {
            for (let j = 0; j < pos.length; j++) {
                const corner = (i === 0 && j === 0) ||
                    (i === 0 && j === pos.length - 1) ||
                    (i === pos.length - 1 && j === 0);
                if (!corner) align(g, pos[i], pos[j]);
            }
        }
        drawFormat(g, 0, 0);                 // 占位：先把格式信息区标为功能模块
        if (ver >= 7) drawVersion(g, ver);
    }

    function finder(g, cx, cy) {
        for (let dy = -4; dy <= 4; dy++) {
            for (let dx = -4; dx <= 4; dx++) {
                const x = cx + dx, y = cy + dy;
                if (x < 0 || x >= g.size || y < 0 || y >= g.size) continue;
                const d = Math.max(Math.abs(dx), Math.abs(dy));
                g.set(x, y, d !== 2 && d !== 4);
            }
        }
    }

    function align(g, cx, cy) {
        for (let dy = -2; dy <= 2; dy++) {
            for (let dx = -2; dx <= 2; dx++) {
                g.set(cx + dx, cy + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
            }
        }
    }

    function bch(value, poly, bits) {
        // 生成多项式 poly 首位必为 1，其次数 d = floor(log2(poly))；
        // 长除法每步看 bit d-1（Nayuki 口径：format 用 >>>9，version 用 >>>11）。
        // 之前误写成 >>>d，余数恒为 value<<bits，格式信息 ECC 全错。
        let rem = value;
        const deg = Math.floor(Math.log2(poly));
        for (let i = 0; i < bits; i++) rem = (rem << 1) ^ ((rem >>> (deg - 1)) * poly);
        return rem;
    }

    function drawFormat(g, mask, ecl) {
        const data = (ecl << 3) | mask;
        const bits = ((data << 10) | bch(data, 0x537, 10)) ^ 0x5412;
        const bit = (i) => ((bits >>> i) & 1) !== 0;
        const size = g.size;
        for (let i = 0; i <= 5; i++) g.set(8, i, bit(i));
        g.set(8, 7, bit(6));
        g.set(8, 8, bit(7));
        g.set(7, 8, bit(8));
        for (let i = 9; i < 15; i++) g.set(14 - i, 8, bit(i));
        for (let i = 0; i < 8; i++) g.set(size - 1 - i, 8, bit(i));
        for (let i = 8; i < 15; i++) g.set(8, size - 15 + i, bit(i));
        g.set(8, size - 8, true);          // 恒黑模块
    }

    function drawVersion(g, ver) {
        const bits = (ver << 12) | bch(ver, 0x1F25, 12);
        const size = g.size;
        for (let i = 0; i < 18; i++) {
            const dark = ((bits >>> i) & 1) !== 0;
            const a = size - 11 + (i % 3);
            const b = Math.floor(i / 3);
            g.set(a, b, dark);
            g.set(b, a, dark);
        }
    }

    function drawCodewords(g, codewords) {
        const size = g.size;
        let i = 0;
        for (let right = size - 1; right >= 1; right -= 2) {
            if (right === 6) right = 5;               // 跳过竖向 timing 列
            for (let vert = 0; vert < size; vert++) {
                for (let j = 0; j < 2; j++) {
                    const x = right - j;
                    const upward = ((right + 1) & 2) === 0;
                    const y = upward ? size - 1 - vert : vert;
                    if (g.fn[y][x] || i >= codewords.length * 8) continue;
                    g.modules[y][x] = ((codewords[i >>> 3] >>> (7 - (i & 7))) & 1) !== 0;
                    i++;
                }
            }
        }
    }

    const MASKS = [
        (x, y) => (x + y) % 2 === 0,
        (x, y) => y % 2 === 0,
        (x, y) => x % 3 === 0,
        (x, y) => (x + y) % 3 === 0,
        (x, y) => (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0,
        (x, y) => (x * y) % 2 + (x * y) % 3 === 0,
        (x, y) => ((x * y) % 2 + (x * y) % 3) % 2 === 0,
        (x, y) => ((x + y) % 2 + (x * y) % 3) % 2 === 0,
    ];

    function applyMask(g, mask) {
        const f = MASKS[mask];
        for (let y = 0; y < g.size; y++) {
            for (let x = 0; x < g.size; x++) {
                if (!g.fn[y][x] && f(x, y)) g.modules[y][x] = !g.modules[y][x];
            }
        }
    }

    function penalty(g) {
        const size = g.size;
        let result = 0;
        const m = g.modules;
        const finderPenalty = (hist) => {
            const n = hist[1];
            const core = n > 0 && hist[2] === n && hist[3] === n * 3 && hist[4] === n && hist[5] === n;
            return (core && hist[0] >= n * 4 && hist[6] >= n ? 1 : 0) +
                (core && hist[6] >= n * 4 && hist[0] >= n ? 1 : 0);
        };
        const addHist = (len, hist) => {
            if (hist[0] === 0) len += size;
            hist.pop();
            hist.unshift(len);
        };
        for (let axis = 0; axis < 2; axis++) {
            for (let a = 0; a < size; a++) {
                let runColor = false, runLen = 0;
                const hist = [0, 0, 0, 0, 0, 0, 0];
                for (let b = 0; b < size; b++) {
                    const c = axis === 0 ? m[a][b] : m[b][a];
                    if (c === runColor) {
                        runLen++;
                        if (runLen === 5) result += PENALTY.N1;
                        else if (runLen > 5) result++;
                    } else {
                        addHist(runLen, hist);
                        if (!runColor) result += finderPenalty(hist) * PENALTY.N3;
                        runColor = c;
                        runLen = 1;
                    }
                }
                if (runColor) { addHist(runLen, hist); runLen = 0; }
                runLen += size;
                addHist(runLen, hist);
                result += finderPenalty(hist) * PENALTY.N3;
            }
        }
        for (let y = 0; y < size - 1; y++) {
            for (let x = 0; x < size - 1; x++) {
                const c = m[y][x];
                if (c === m[y][x + 1] && c === m[y + 1][x] && c === m[y + 1][x + 1]) {
                    result += PENALTY.N2;
                }
            }
        }
        let dark = 0;
        for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (m[y][x]) dark++;
        const total = size * size;
        const k = Math.ceil(Math.abs(dark * 20 - total * 10) / total) - 1;
        return result + k * PENALTY.N4;
    }

    // ── 对外：编码 ──
    // 返回 { version, size, get(x, y) }；容量不够时抛错（不会静默出坏码）。
    function encode(text) {
        const bytes = [];
        for (let i = 0; i < text.length; i++) {
            const c = text.charCodeAt(i);
            if (c > 0xFF) throw new Error('二维码只支持单字节文本');
            bytes.push(c);
        }
        let ver = 0, codewords = null;
        for (let v = 1; v <= MAX_VERSION; v++) {
            const cw = buildCodewords(bytes, v);
            if (cw) { ver = v; codewords = cw; break; }
        }
        if (!ver) throw new Error('内容过长（上限约 ' + 213 + ' 字节）');

        const size = ver * 4 + 17;
        let best = null, bestScore = Infinity, bestMask = 0;
        for (let mask = 0; mask < 8; mask++) {
            const g = new Grid(size);
            drawFunctionPatterns(g, ver);
            drawCodewords(g, codewords);
            applyMask(g, mask);
            drawFormat(g, mask, FORMAT_BITS_M);
            const score = penalty(g);
            if (score < bestScore) { bestScore = score; best = g; bestMask = mask; }
        }
        return {
            version: ver, size: size, mask: bestMask,
            get: (x, y) => best.get(x, y),
        };
    }

    // 画到 canvas。quiet zone 固定 4 模块（规范要求 ≥4）。
    // scale 自动取「装得进 CSS 尺寸的最大整数倍」，保证像素对齐不糊。
    function draw(canvas, text, opts) {
        opts = opts || {};
        const qr = encode(text);
        const quiet = 4;
        const total = qr.size + quiet * 2;
        const cssSize = opts.size || canvas.clientWidth || 160;
        const dpr = Math.min(window.devicePixelRatio || 1, 3);
        const scale = Math.max(1, Math.floor(cssSize / total));
        const px = total * scale;
        canvas.width = Math.round(px * dpr);
        canvas.height = Math.round(px * dpr);
        canvas.style.width = px + 'px';
        canvas.style.height = px + 'px';
        const ctx = canvas.getContext('2d');
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.fillStyle = opts.light || '#ffffff';
        ctx.fillRect(0, 0, px, px);
        ctx.fillStyle = opts.dark || '#000000';
        for (let y = 0; y < qr.size; y++) {
            for (let x = 0; x < qr.size; x++) {
                if (qr.get(x, y)) ctx.fillRect((x + quiet) * scale, (y + quiet) * scale, scale, scale);
            }
        }
        return qr;
    }

    PV.qr = { encode, draw };
})(window.PV = window.PV || {});
