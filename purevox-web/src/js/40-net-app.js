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

// Net 版装配：对应 lite_net（网络输入 → 降噪 → 本地输出，最简档）。
// 浏览器不能监听 TCP 端口，故「网络输入」这半边走 WebRTC：
//   接收端（默认，本页的主角色）= lite_net 本体
//     远端麦克风 --WebRTC--> 本页 --> ONNX 降噪 --> 本地扬声器
//   发送端 = 喂给接收端的另一端（本机麦克风直传，不做降噪）
// 与桌面端一致：降噪只在接收侧做，发送端原样出声。
// 信令：手工交换连接码（deflate + base64），无服务端、无 STUN/TURN，
// 只走 host candidate —— 与 Lite Net 的局域网定位一致。
(function (PV) {
    'use strict';

    const A = PV.ASSETS;
    const app = {
        cfg: null, pipeline: null, pc: null, stream: null,
        role: 'rx', running: false, engineReady: false,
        rtt: 0, lost: 0, jitter: 0, rxBytes: 0,
    };
    PV.app = app;

    // 信令信封（我们自己的线上格式，压缩后 base64）：
    //   { v, t, sdp } —— 注意 t 只是信封里的缩写；
    //   交给 WebRTC API 前必须还原成 { type, sdp }，否则
    //   setRemoteDescription 收不到 type，会报「Failed to parse SessionDescription」。
    async function encodeSignal(desc) {
        const json = JSON.stringify({ v: 1, t: desc.type, sdp: desc.sdp });
        if (typeof CompressionStream === 'function') {
            const cs = new CompressionStream('deflate-raw');
            const stream = new Blob([json]).stream().pipeThrough(cs);
            const buf = new Uint8Array(await new Response(stream).arrayBuffer());
            let s = '';
            for (let i = 0; i < buf.length; i += 0x8000) {
                s += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
            }
            return 'Z' + btoa(s);
        }
        return 'R' + btoa(unescape(encodeURIComponent(json)));
    }

    async function decodeSignal(text) {
        const raw = (text || '').trim().replace(/\s+/g, '');
        if (!raw) throw new Error('连接码为空');
        const tag = raw[0];
        const body = raw.slice(1);
        if (tag === 'Z') {
            if (typeof DecompressionStream !== 'function') {
                throw new Error('当前浏览器不支持解压连接码（缺 DecompressionStream）');
            }
            const bin = atob(body);
            const buf = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
            const ds = new DecompressionStream('deflate-raw');
            const stream = new Blob([buf]).stream().pipeThrough(ds);
            const json = await new Response(stream).text();
            return JSON.parse(json);
        }
        if (tag === 'R') return JSON.parse(decodeURIComponent(escape(atob(body))));
        throw new Error('连接码格式不认识（应以 Z 或 R 开头）');
    }

    // 无 STUN → 只需 host candidate，gather 很快；仍留超时兜底
    function iceComplete(pc, timeoutMs) {
        return new Promise((res) => {
            if (pc.iceGatheringState === 'complete') return res();
            let done = false;
            const finish = () => {
                if (done) return;
                done = true;
                pc.removeEventListener('icegatheringstatechange', onChange);
                clearTimeout(timer);
                res();
            };
            const onChange = () => { if (pc.iceGatheringState === 'complete') finish(); };
            const timer = setTimeout(finish, timeoutMs || 3000);
            pc.addEventListener('icegatheringstatechange', onChange);
        });
    }

    function newPeer() {
        closePeer();
        // iceServers 留空：局域网直连（与 Lite Net 同——不引第三方信令/STUN）
        const pc = new RTCPeerConnection({ iceServers: [] });
        pc.addEventListener('connectionstatechange', () => {
            const s = pc.connectionState;
            PV.ui.log('连接状态：' + s);
            if (s === 'connected') {
                PV.ui.state('已连接', 'good');
                PV.ui.status('已连接 · 48kHz 单声道 · 降噪常驻');
            } else if (s === 'connecting') {
                PV.ui.state('协商中', 'busy');
            } else if (s === 'disconnected' || s === 'failed') {
                PV.ui.state(s === 'failed' ? '连接失败' : '已断开', 'bad');
                if (s === 'failed') teardown();
            }
        });
        app.pc = pc;
        return pc;
    }

    function closePeer() {
        if (app.pc) {
            try { app.pc.close(); } catch (e) { /* 已关闭 */ }
            app.pc = null;
        }
    }

    // ── 接收端：生成邀请码 ──
    async function makeInvite() {
        const pc = newPeer();
        pc.addEventListener('track', (e) => {
            const stream = e.streams[0] || new MediaStream([e.track]);
            onRemoteStream(stream);
        });
        const offer = await pc.createOffer({ offerToReceiveAudio: true });
        await pc.setLocalDescription(offer);
        await iceComplete(pc);
        const code = await encodeSignal(pc.localDescription);
        PV.ui.el('rx-invite').value = code;
        // 新邀请码 = 旧应答码作废，清掉免得误粘（重复应答会撞 stable 状态）
        PV.ui.el('rx-answer').value = '';
        PV.ui.state('待应答', 'busy');
        PV.ui.log('邀请码已生成（' + code.length + ' 字符）——复制发给对方，再把对方的应答码粘到下面');
    }

    // 信封 → RTCSessionDescriptionInit：t → type。
    // 少了这一步，setRemoteDescription 收不到 type，Chrome 报
    // 「Failed to parse SessionDescription」。
    function toSessionDesc(desc) {
        return { type: desc.t, sdp: desc.sdp };
    }

    async function acceptAnswer() {
        const text = (PV.ui.el('rx-answer').value || '').trim();
        if (!app.pc) {
            PV.ui.log('请先生成邀请码，再把对方的应答码粘进来');
            return;
        }
        if (!text) {
            PV.ui.log('请先粘贴对方的应答码');
            return;
        }
        // 接受应答的前置条件是 have-local-offer。已经 stable 说明这份应答码
        // 用过了（重复点「连接」或粘了旧码）——直接报浏览器原始错误没用，
        // 给一句能照做的提示。
        const st = app.pc.signalingState;
        if (st !== 'have-local-offer') {
            if (st === 'stable') {
                PV.ui.state(app.pc.connectionState === 'connected' ? '已连接' : '已应答', 'good');
                PV.ui.log('这份应答码已经用过了。要换一台设备，请重新点「生成邀请码」。');
            } else {
                PV.ui.state('需重连', 'busy');
                PV.ui.log('当前协商状态 ' + st + '，收不了新的应答码；请重新点「生成邀请码」。');
            }
            return;
        }
        try {
            const desc = await decodeSignal(text);
            if (desc.t !== 'answer') throw new Error('这不是应答码（是邀请码）');
            await app.pc.setRemoteDescription(toSessionDesc(desc));
            // 趁这次点击的用户激活还在（ICE 要好几秒），先把 AudioContext 起好，
            // 否则音轨到达时新建的 context 会被自动播放策略挂起 → 连上却没声音
            if (!app.pipeline) app.pipeline = new PV.Pipeline();
            app.pipeline._onLevel = (peak) => PV.ui.level(PV.util.linearToDb(peak));
            await app.pipeline.prepare();
            PV.ui.state('协商中', 'busy');
        } catch (e) {
            PV.ui.state('应答码无效', 'bad');
            PV.ui.log('应答码解析失败：' + (e && e.message ? e.message : e));
        }
    }

    // ── 接收端：远端音轨接入降噪播放链路 ──
    async function onRemoteStream(stream) {
        if (app.running) return;
        PV.ui.log('收到远端音轨，接入降噪链路…');
        if (!app.engineReady) {
            const ok = await PV.session.bootEngine();
            if (!ok) return;
            app.engineReady = true;
        }
        refreshState();
        app.stream = stream;
        if (!app.pipeline) app.pipeline = new PV.Pipeline();
        app.pipeline._onLevel = (peak) => PV.ui.level(PV.util.linearToDb(peak));
        try {
            await app.pipeline.start(stream, {
                preGainDb: app.cfg.pre_gain_db,
                postGainDb: app.cfg.post_gain_db,
            });
        } catch (e) {
            PV.ui.log('音频图启动失败：' + (e && e.message ? e.message : e));
            return;
        }
        const outSel = PV.ui.el('out-select');
        if (outSel && outSel.value && await app.pipeline.setSinkId(outSel.value)) {
            app.cfg.output_device_id = outSel.value;
            app.cfg.output_device = outSel.options[outSel.selectedIndex].textContent;
            PV.util.saveCfg('net', app.cfg);
            PV.ui.log('输出设备：' + app.cfg.output_device);
        }
        app.running = true;
        PV.ui.running(true, '挂断');
        PV.ui.status('已连接 · 48kHz 单声道 · 降噪常驻');
    }

    // ── 发送端：粘邀请码 → 授权麦克风 → 生成应答码 ──
    async function makeAnswer() {
        const text = PV.ui.el('tx-invite').value;
        let desc;
        try {
            desc = await decodeSignal(text);
            if (desc.t !== 'offer') throw new Error('这不是邀请码');
        } catch (e) {
            PV.ui.state('邀请码无效', 'bad');
            PV.ui.log('邀请码解析失败：' + (e && e.message ? e.message : e));
            return;
        }
        app.stream = null;

        const micSel = PV.ui.el('in-select');
        const audio = {
            echoCancellation: false, noiseSuppression: false,
            autoGainControl: false, channelCount: 1,
        };
        if (micSel && micSel.value) audio.deviceId = { ideal: micSel.value };
        let stream;
        try {
            stream = await navigator.mediaDevices.getUserMedia({ audio });
        } catch (e) {
            PV.ui.state('无权限', 'bad');
            PV.ui.log('麦克风不可用：' + (e && e.name ? e.name + ' ' + e.message : e));
            return;
        }
        app.stream = stream;
        PV.ui.log('麦克风已开：' + (stream.getAudioTracks()[0] || {}).label);

        const pc = newPeer();
        try {
            // 顺序按 WebRTC 惯例：先吃下远端 offer，再挂本地轨道，最后出 answer
            await pc.setRemoteDescription(toSessionDesc(desc));
            stream.getTracks().forEach((t) => pc.addTrack(t, stream));
            const answer = await pc.createAnswer();
            await pc.setLocalDescription(answer);
            await iceComplete(pc);
            const code = await encodeSignal(pc.localDescription);
            PV.ui.el('tx-answer').value = code;
            PV.ui.state('待连接', 'busy');
            // 必须置位：主按钮靠它区分「生成应答码」与「挂断」。漏置会让第二次
            // 点击又走一遍 makeAnswer → newPeer 把已建好的连接拆掉。
            app.running = true;
            PV.ui.running(true, '挂断');
            PV.ui.status('已应答 · 等待对方连接');
            PV.ui.log('应答码已生成（' + code.length + ' 字符）——复制发回给对方，连接建立后自动开声');
        } catch (e) {
            PV.ui.state('应答失败', 'bad');
            PV.ui.log('生成应答码失败：' + (e && e.message ? e.message : e));
            stream.getTracks().forEach((t) => t.stop());   // 失败别把麦克风挂着
            if (app.stream === stream) app.stream = null;
            return;
        }
        startMeter(stream);
        // 麦克风权限已到手：设备 label 与输出列表此刻解锁
        await app.inRow.refresh();
        await app.outRow.refresh();
    }

    // 发送端电平表：发送端不跑降噪链路，若不给它电平反馈，看起来就像没在工作。
    // 单独一个只连 AnalyserNode、不连 destination 的 AudioContext（不回放自己）。
    async function startMeter(stream) {
        stopMeter();
        try {
            const Ctor = window.AudioContext || window.webkitAudioContext;
            app._meterCtx = new Ctor();
            if (app._meterCtx.state === 'suspended') await app._meterCtx.resume();
            const src = app._meterCtx.createMediaStreamSource(stream);
            const an = app._meterCtx.createAnalyser();
            an.fftSize = 1024;
            src.connect(an);                       // 故意不接 destination
            const buf = new Uint8Array(an.fftSize);
            app._meterTimer = setInterval(() => {
                an.getByteTimeDomainData(buf);
                let sum = 0;
                for (let i = 0; i < buf.length; i++) {
                    const v = (buf[i] - 128) / 128;
                    sum += v * v;
                }
                PV.ui.level(PV.util.linearToDb(Math.sqrt(sum / buf.length)));
            }, 100);
        } catch (e) {
            PV.ui.log('电平表不可用：' + (e && e.message ? e.message : e));
        }
    }
    function stopMeter() {
        if (app._meterTimer) {
            clearInterval(app._meterTimer);
            app._meterTimer = null;
        }
        if (app._meterCtx) {
            try { app._meterCtx.close(); } catch (e) { /* 已关闭 */ }
            app._meterCtx = null;
        }
    }

    async function teardown() {
        app.running = false;
        stopMeter();
        if (app.pipeline) {
            await app.pipeline.stop();
            app.pipeline = null;
        }
        if (app.stream) {
            app.stream.getTracks().forEach((t) => t.stop());
            app.stream = null;
        }
        closePeer();
        // 连接已断，旧应答码随之作废：清空免得下次误粘（会撞 stable 状态）
        PV.ui.el('rx-answer').value = '';
        PV.ui.running(false, app.role === 'rx' ? '连接' : '开始发送');
        PV.ui.state('未连接', 'idle');
        PV.ui.status('未连接');
        PV.ui.resetLevel();
        PV.ort.flush();
    }

    // 设备行（与 Mic 版共用实现，见 35-session.js）。接收端自己不用麦克风，
    // 所以输出设备列表默认是锁着的——由输出行上的「授权」按钮解锁。
    function setupDevices() {
        app.inRow = PV.session.setupInputRow('net', app.cfg);
        app.outRow = PV.session.setupOutputRow('net', app.cfg,
            (id) => (app.pipeline ? app.pipeline.setSinkId(id) : null));
    }

    // ── 角色切换 ──
    // 接收端 = lite_net 本体：远端音频进、降噪、出扬声器 → 输出 + 前/后增益归它。
    // 发送端 = 喂音频的那一端：一个麦克风输入、原样直传，不输出也不降噪，
    // 所以输出行与增益行整组隐藏（露着只会让人以为该在发送端调增益）。
    function setRole(role) {
        if (app.role === role && PV.ui.el('tab-' + role).classList.contains('active')) return;
        app.role = role;
        app.cfg.role = role;
        PV.util.saveCfg('net', app.cfg);
        for (const r of ['rx', 'tx']) {
            const tab = PV.ui.el('tab-' + r);
            const pane = PV.ui.el('pane-' + r);
            if (tab) tab.classList.toggle('active', r === role);
            if (pane) pane.style.display = r === role ? 'block' : 'none';
        }
        const controls = PV.ui.el('rx-controls');
        if (controls) controls.style.display = role === 'rx' ? 'flex' : 'none';
        // 调试面板也分角色：接收端看 RTT/丢包，发送端看协商状态/已发送/麦克风
        for (const cls of ['dg-rx', 'dg-tx']) {
            const want = cls === 'dg-rx' ? role === 'rx' : role === 'tx';
            document.querySelectorAll('.' + cls).forEach((n) => {
                n.style.display = want ? '' : 'none';
            });
        }
        if (app.running) teardown();
        else {
            PV.ui.running(false, role === 'rx' ? '连接' : '开始发送');
            PV.ui.state('未连接', 'idle');
            PV.ui.status('未连接');
        }
    }

    // ── 传输统计（RTT / 丢包 / 抖动，对齐 html/ 客户端调试面板口径）──
    // 接收端看 RTT 与丢包；发送端看协商状态与已发送字节——两端都有反馈，
    // 否则发送端没有任何数字，看起来就像没在跑。
    async function pollStats() {
        if (!app.pc) return;
        if (app.role === 'tx') {
            PV.ui.debug({
                signaling: app.pc.connectionState + '/' + app.pc.iceConnectionState,
                sent: app.sentBytes ? (app.sentBytes / 1024).toFixed(0) + ' KB' : '0 KB',
                mic: app.stream ? (app.stream.getAudioTracks()[0] || {}).label || '已开' : '--',
            });
        }
        if (app.pc.connectionState !== 'connected') return;
        let report;
        try {
            report = await app.pc.getStats();
        } catch (e) {
            return;
        }
        report.forEach((r) => {
            if (r.type === 'candidate-pair' && r.state === 'succeeded' && r.nominated) {
                app.rtt = Math.round((r.currentRoundTripTime || 0) * 1000);
            } else if (r.type === 'inbound-rtp' && r.kind === 'audio') {
                app.lost = r.packetsLost || 0;
                app.jitter = Math.round((r.jitter || 0) * 1000);
                app.rxBytes = r.bytesReceived || 0;
            } else if (r.type === 'outbound-rtp' && r.kind === 'audio') {
                app.sentBytes = r.bytesSent || 0;
            }
        });
    }

    function bind() {
        PV.ui.el('tab-rx').addEventListener('click', () => setRole('rx'));
        PV.ui.el('tab-tx').addEventListener('click', () => setRole('tx'));

        PV.ui.el('btn-run').addEventListener('click', () => {
            if (app.running) teardown();
            else if (app.role === 'rx') {
                if (!app.pc) PV.ui.log('先生成邀请码');
            } else makeAnswer();
        });

        PV.ui.el('rx-make').addEventListener('click', () => makeInvite().catch(reportErr));
        PV.ui.el('rx-accept').addEventListener('click', () => acceptAnswer());
        PV.ui.el('tx-make').addEventListener('click', () => makeAnswer());

        PV.ui.el('rx-copy').addEventListener('click', () => copyCode('rx-invite'));
        PV.ui.el('tx-copy').addEventListener('click', () => copyCode('tx-answer'));

        window.addEventListener('beforeunload', () => {
            closePeer();
            if (app.stream) app.stream.getTracks().forEach((t) => t.stop());
            PV.ort.dispose();
        });
    }

    // 状态徽标只有一个写者：引擎引导会分阶段改它（加载中→就绪），
    // 若引导比连接先结束就会把「已连接」盖掉，故连接态由此函数按真实连接状态复写。
    function refreshState() {
        if (!app.pc || app.pc.connectionState !== 'connected') return;
        PV.ui.state('已连接', 'good');
        PV.ui.status('已连接 · 48kHz 单声道 · 降噪常驻');
    }

    function reportErr(e) {
        PV.ui.state('失败', 'bad');
        PV.ui.log('错误：' + (e && e.message ? e.message : e));
    }

    async function copyCode(id) {
        const text = PV.ui.el(id).value;
        if (!text) return;
        try {
            await navigator.clipboard.writeText(text);
            PV.ui.log('连接码已复制到剪贴板');
        } catch (e) {
            PV.ui.el(id).select();
            PV.ui.log('剪贴板不可用，请手动全选复制');
        }
    }

    async function main() {
        app.cfg = PV.session.init('net');
        PV.ui.el('origin-note').textContent = location.origin;
        PV.ui.debug({ model: A.meta.modelLabel, hop: PV.util.HOP + ' / 10ms' });
        PV.ui.log('PureVox Web（Lite Net）· ' + A.meta.modelLabel +
            ' · ONNX Runtime Web ' + A.meta.ortVersion);
        // 两条路径要分清：程序从哪下 ≠ 音频往哪走
        PV.ui.log('程序资源（页面 / ONNX 运行时 / 降噪模型）从 ' + location.origin + ' 加载');
        PV.ui.log('音频走局域网 WebRTC 直连：host candidate，不用 STUN/TURN、不经服务器、不出局域网');
        PV.ui.log('两端需同一网段（跨网段需自备 TURN，本版未内置）；同机双开收发会声反馈，验证请戴耳机');
        PV.ui.log('发送端只做一件事：一个麦克风输入 → 原样送上网（不输出、不降噪，增益在接收端调）');
        if (!PV.Pipeline.supported) {
            PV.ui.state('不支持', 'bad');
            PV.ui.log('当前浏览器不支持 AudioWorklet，无法运行');
            return;
        }
        setRole(app.cfg.role === 'tx' ? 'tx' : 'rx');
        setupDevices();
        bind();
        PV.session.startStats(() => ({
            rtt: app.rtt ? app.rtt + ' ms' : '--',
            loss: app.role === 'rx' ? (app.lost + ' / ' + app.jitter + 'ms') : '--',
        }));        setInterval(pollStats, 1000);
        await app.inRow.refresh();
        await app.outRow.refresh();
        // 接收端是主角色：进页面先把引擎热起来，连上即出声
        if (app.role === 'rx') {
            PV.session.bootEngine().then((ok) => {
                app.engineReady = ok;
                refreshState();
            });
        }
    }

    window.addEventListener('DOMContentLoaded', main);
})(window.PV = window.PV || {});
