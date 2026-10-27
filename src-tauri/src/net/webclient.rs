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

//! 网页版客户端（DESIGN.md §4.1）：网络行卡片里的二维码指向这里。
//!
//! **为什么要有**：二维码扫的是**网页地址**（`http://<ip>:59123/`），不是 `ws://`
//! ——扫 `ws://` 手机只会把它当一串文本，什么也不会发生。网页版的功能与 APK 对等：
//! 麦克风上行、扬声器下行、打字、全尺寸键盘，所以「手机客户端」不该是个独立概念，
//! 浏览器只是其中一种客户端。
//!
//! **为什么不引编解码库**：Opus 用 `libopus-wasm`（MIT，预编译 wasm，随包分发），
//! 与 Rust 侧 `net/opus_sys.rs` 运行时加载 libopus 是同一个思路：不新增 C 编译链。
//! **不用浏览器内置 WebCodecs**：`AudioEncoder`/`AudioDecoder` 只有 Chrome 94+ /
//! Safari 16.4+ 才有，实测有浏览器根本没有，于是麦克风与扬声器一起不可用。
//!
//! 服务端**什么都不解析**：这页是纯静态资源，协议仍只有 `/ws` 那一套
//! （`proto.rs`），所以网页版与 APK 走的是同一条链路、同一种节拍。
//!
//! **下行播放必须是环形缓冲，不是「一包一个 AudioBufferSource」**（2026-10-27 实测）：
//! Windows 的 10 ms 定时器实际按 15.6 ms 触发，包成串到达；20 ms 的按包起播会在 1 kHz
//! 测试音上拼出 2~6 ms 空隙（8 s 内 8 次，能量 99.9% → 93.8%）。现在解码后的样本进
//! AudioWorklet 的环形队列，深度固定 20 ms（超了丢一格 / 空了淡出），欠载数经 `spkUnderruns` 可见。

/// 网页版客户端页面。整页自包含（无外链、无构建），直接内联进二进制。
pub const PAGE: &str = r##"<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#101418">
<title>PureVox</title>
<style>
:root{--bg:#101418;--fg:#e8edf2;--mut:#93a1b0;--ok:#3ddc84;--warn:#ffb454;--err:#ff5f56;--line:#26313d}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif;
  padding:max(12px,env(safe-area-inset-top)) 12px calc(20px + env(safe-area-inset-bottom))}
h1{font-size:17px;margin:0 0 2px}
.sub{color:var(--mut);font-size:12px;margin-bottom:12px}
.bar{display:flex;align-items:center;gap:8px;background:#18202a;border:1px solid var(--line);
  border-radius:10px;padding:9px 11px;margin-bottom:10px}
.dot{width:9px;height:9px;border-radius:50%;background:var(--mut);flex:0 0 auto}
.dot.on{background:var(--ok)} .dot.err{background:var(--err)} .dot.warn{background:var(--warn)}
#st{font-size:12px;color:var(--mut);min-width:0;overflow-wrap:anywhere}
.rtt{margin-left:auto;color:var(--mut);font-size:11px;flex:0 0 auto}
button{font:inherit;color:var(--fg);background:#202a36;border:1px solid var(--line);
  border-radius:9px;padding:10px 12px;cursor:pointer;flex:1 1 0;min-width:0}
button:active{transform:translateY(1px)}
button.pri{background:#1d5c3a;border-color:#2b7d51}
button[aria-pressed=true]{background:#1d5c3a;border-color:#2b7d51}
button:disabled{opacity:.45}
.row{display:flex;gap:8px;margin-bottom:10px}
.note{color:var(--mut);font-size:12px;margin:10px 2px 4px}
.warnbox{border:1px solid #5a4a20;background:#241f12;color:#f0d9a8;border-radius:10px;
  padding:10px 11px;font-size:12px;margin-bottom:10px}
input[type=text]{font:inherit;width:100%;color:var(--fg);background:#18202a;border:1px solid var(--line);
  border-radius:9px;padding:11px 12px}
.keys{display:none;position:fixed;inset:0;background:#0c1014;z-index:9;overflow:auto;
  padding:max(10px,env(safe-area-inset-top)) 10px calc(10px + env(safe-area-inset-bottom))}
.keys.on{display:block}
.keys .kb{margin:0 auto;max-width:520px}
.krow{display:flex;gap:5px;margin-bottom:5px}
.krow .k{flex:1;min-width:0;padding:12px 2px;text-align:center;font-size:12px;border-radius:7px;
  background:#1b242e;border:1px solid var(--line);user-select:none;touch-action:none}
.krow .k.w2{flex:2} .krow .k.w3{flex:3}
.krow .k.act{background:#1d5c3a}
</style>

<h1>PureVox</h1>
<div class="sub" id="pc"></div>

<div class="bar"><span class="dot" id="dot"></span><span id="st">未连接</span><span class="rtt" id="rtt"></span></div>

<div class="warnbox" id="warn">
  同网段直连且<strong>无鉴权</strong>：开启远程输入后，同一 Wi-Fi 下的任何设备都能向这台电脑打字和发按键。
</div>

<div class="row">
  <button id="bMic" aria-pressed="false">麦克风</button>
  <button id="bSpk" aria-pressed="false">扬声器</button>
</div>
<div class="row">
  <button id="bKb" style="flex:1">键盘 / 打字</button>
</div>
<div class="note" id="hint"></div>
<div class="note" id="caps"></div>

<input type="text" id="text" placeholder="在此打字，发到电脑…" autocomplete="off" enterkeyhint="send">
<div class="note">用系统的「全键盘」发送按键可能比这排虚拟键好用；两者可以并用。</div>

<div class="keys" id="kb"><div class="kb">
  <div class="krow" id="r0"></div><div class="krow" id="r1"></div>
  <div class="krow" id="r2"></div><div class="krow" id="r3"></div>
  <div class="krow" id="r4"></div>
  <button id="bClose" style="width:100%;margin-top:8px">收起</button>
</div></div>

<script>
"use strict";
// ---- 与 Rust 侧 proto.rs 保持一致的协议（只有 /ws 这一套） ----
const SR = 48000, HOP = 480, HDR = 8;
/** 单调时钟（µs，u32 回绕）：与 Rust 的 `rt::now_us32()` 同一口径。 */
function nowUs() { return Math.floor(performance.now() * 1000) & 0xFFFFFFFF; }
/** 带符号 u32 微秒差（跨回绕有效），与 Rust `rt::us_diff` 等价。 */
function usDiff(now, then) { return (now - then) | 0; }
/** 与电脑的时钟偏移（µs，网页 − 电脑）；没同步过是 null。 */
let clockOffsetUs = null;
const st = document.getElementById('st'), dot = document.getElementById('dot');
const hint = document.getElementById('hint');
document.getElementById('pc').textContent = location.host;

let ws = null, micOn = false, spkOn = false;
/** 自动申请麦克风只试一次：用户拒了之后不该在每次重连时反复弹框骚扰。 */
let micAsked = false;
let enc = null, dec = null, actx = null, micNode = null, spkGain = null, spkNode = null;
/** wasm 编解码模块（`createEncoder`/`createDecoder` 的工厂）；关掉扬声器再打开时要重建解码器。 */
let opusMod = null;
/** 扬声器是否已经向服务端发过 `sub`（重连/关开时不能重复订阅）。 */
let spkSub = false;
/** 下行缓冲水位与欠载数（worklet 上报，只用于状态行 / 调试）。 */
let spkBuffered = 0, spkUnderruns = 0;
/** 下行单向延迟（ms，电脑发出 → 网页收到）；NaN = 还没同步。 */
let spkOneWay = NaN;
/** worklet 上报的环状态（深度 / 欠载 / 晚到 / 裁剪 / 抖动 / 最大间隔）。 */
let spkTarget = 0, spkLate = 0, spkTrims = 0, spkJitter = 0, spkMaxGap = 0;

// ---- Opus 编解码器：wasm（`libopus-wasm`，MIT），不依赖浏览器内置的 WebCodecs ----
// 帧长按引擎的 10 ms hop 配（480 样本 @ 48 kHz 单声道），和服务端队列同格式，
// 采集 / 解码出来的都是 Float32，一个转换都不用做。
// `signal` 必须传 `Signal.Voice` 那个**常量**（3001），传字符串 'voice' 会被
// libopus-wasm 判为非法值而抛 `RangeError`——编解码器初始化失败，表现为「点了没反应」。
let codecReady = false;
async function loadCodecs() {
  if (codecReady) return;
  opusMod = await import('/libopus-wasm.js');
  enc = await opusMod.createEncoder({
    channels: 1, frameSize: HOP, bitrate: 32000, signal: opusMod.Signal.Voice,
  });
  dec = await opusMod.createDecoder({ channels: 1 });
  codecReady = true;
}

/** 解码器按需重建：关掉扬声器时**不再销毁**它，但浏览器 / 内存异常后也能恢复。 */
async function ensureDec() {
  if (!codecReady) await loadCodecs();
  if (!dec) dec = await opusMod.createDecoder({ channels: 1 });
  return dec;
}
let frameMs = 10, ready = false;
/** 上行包序号（与 Rust 侧同口径：每包 +1，只用于判丢包）。 */
let micSeq = 0;

function say(t, cls) { st.textContent = t; dot.className = 'dot ' + (cls || ''); }
function send(o) { if (ws && ws.readyState === 1) ws.send(JSON.stringify(o)); }
function sendBin(b) { if (ws && ws.readyState === 1 && b) ws.send(b); }

// ---- 连接 ----
function connect() {
  const u = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws';
  say('连接中…', 'warn');
  ready = false;
  ws = new WebSocket(u);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => {
    // 握手是**必须**的（proto.rs：`proto` 不等于 PROTO 服务端直接回 err 断开）。
    send({ t: 'hello', proto: 3 });
  };
  ws.onclose = () => {
    // 连接断了，服务端的订阅随连接销毁；本地播放节点也收掉，重连后按 spkOn 重新订阅。
    stopSpkNode(false);
    say('已断开，3 秒后重连', 'err');
    setTimeout(connect, 3000);
  };
  ws.onerror = () => { /* onclose 会接手 */ };
  ws.onmessage = (ev) => {
    if (typeof ev.data === 'string') {
      let m; try { m = JSON.parse(ev.data); } catch { return; }
      if (m.t === 'ready') {
        ready = true; frameMs = m.frame_ms || 10;
        say('已连接 · ' + m.rate + ' Hz · ' + m.frame_ms + ' ms/帧', 'on');
        // **连上就自动申请麦克风**（用户扫码进来的意图就是要用麦克风，不该再点一次按钮）。
        // 浏览器会弹权限框；被拒也不影响打字 / 键盘 / 扬声器，那些照常可用。
        if (!micOn && !micAsked) {
          micAsked = true;
          autoMic().catch(() => {});
        }
        if (micOn && !enc) startMic();
        if (spkOn) startSpk();
      } else if (m.t === 'sync') {
        // NTP 四时戳：t1 = 收到、t2 = 回（都用网页自己的单调时钟，µs）
        const t1 = nowUs();
        send({ t: 'syncreply', id: m.id, t0: m.t0, t1: t1, t2: nowUs() });
      } else if (m.t === 'offset') {
        // 电脑算出的时钟偏移（网页 − 电脑，µs）：下行单向延迟靠它
        clockOffsetUs = m.us;
      } else if (m.t === 'pong') {
        const t0 = pingAt[m.id];
        if (t0) { const rtt = Math.round(performance.now() - t0);
                  pingAt[m.id] = 0; send({ t: 'rtt', ms: rtt }); rttEl.textContent = rtt + ' ms'; }
      } else if (m.t === 'err') {
        say('服务端拒绝：' + m.msg, 'err');
      }
      return;
    }
    // 二进制 = 8 字节帧头（seq + 发送方 µs）+ 一个 Opus 包
    const raw = new Uint8Array(ev.data);
    if (raw.length <= HDR) return;
    const pkt = raw.subarray(HDR);
    if (spkOn && dec && ready) playPacket(pkt, new DataView(raw.buffer, raw.byteOffset).getUint32(4));
  };
  // RTT：每 5 秒 ping 一次，服务端原样回 pong；时差自己算好再用 rtt 上报
  // （proto.rs 的约定：客户端测、服务端不计时，这样 /debug 里能直接看到）
  clearInterval(pingTimer);
  pingTimer = setInterval(() => {
    if (ws && ws.readyState === 1) { const id = ++pingSeq; pingAt[id] = performance.now(); send({ t: 'ping', id }); }
  }, 5000);
}
let pingTimer = null, pingSeq = 0;
const pingAt = {};
const rttEl = document.getElementById('rtt');

// ---- 网页自己的音频时钟偏差：AudioContext 的播放时钟 vs performance.now ----
// 实测（DESIGN.md §4.1.1）设备晶振差只有几十 ppm，这里让浏览器把自己的偏差报上来，
// /debug 的 `net.client.clock_ppm` 直接可读 —— 与安卓用 getPlaybackHeadPosition 是同一件事。
let webClockPpm = NaN;
let lastClockSample = null;
setInterval(() => {
  if (!actx || actx.state !== 'running') return;
  const s = { audio: actx.currentTime, wall: performance.now() };
  if (lastClockSample) {
    const dAudio = s.audio - lastClockSample.audio;
    const dWall = (s.wall - lastClockSample.wall) / 1000;
    if (dWall > 1.5 && dAudio > 0) webClockPpm = (dAudio / dWall - 1) * 1e6;
  }
  lastClockSample = s;
}, 1000);

// ---- 客户端状态上报（2 Hz）：电脑那边看不到网页的播放环，只能靠这里报 ----
setInterval(() => {
  if (!ws || ws.readyState !== 1 || !spkOn) return;
  send({
    t: 'st',
    level_ms: spkBuffered * 1000 / SR,
    target_ms: spkTarget,
    underruns: spkUnderruns,
    concealed: 0,
    late: spkLate,
    one_way_ms: Number.isFinite(spkOneWay) ? spkOneWay : undefined,
    clock_ppm: Number.isFinite(webClockPpm) ? webClockPpm : undefined,
  });
}, 500);

// ---- 麦克风上行：getUserMedia → AudioWorklet 取 480 样本的 hop → wasm Opus ----
let workletUrl = null;
function worklet() {
  if (workletUrl) return workletUrl;
  // 内联的 worklet：只把收到的 Float32 拼成 hop 交回主线程，编解码在主线程做。
  const src = `class H extends AudioWorkletProcessor{
    constructor(){super();this.buf=new Float32Array(480);this.n=0;}
    process(inputs){const c=inputs[0]&&inputs[0][0];if(c){
      for(let i=0;i<c.length;i++){this.buf[this.n++]=c[i];
        if(this.n===480){const o=this.buf.slice(0);this.port.postMessage(o);this.n=0;}}}
    return true;}}
  // 下行播放：**环形队列 + 固定 20 ms 深度**（DESIGN.md §4.1.4）。不能每包起一个 AudioBufferSource：
  // Windows 的 10 ms 定时器实际按 15.6 ms 触发，包成串到达，20 ms 的「按包起播」
  // 会周期性断 2~6 ms（1 kHz 测试音实测 8 秒内 8 次，能量从 99.9% 掉到 93.8%）。
  class S extends AudioWorkletProcessor{
    // 下行播放环：**唯一的缓冲**，与 `audio/Rt.kt` / `net/rt.rs` 逐字同一套规则：
    // 深度固定 20 ms，运行期只有「超了丢一格」和「空了补一格」，**不重采样、不伺服**。
    constructor(){
      super();
      this.b=new Float32Array(8192);this.head=0;this.count=0;
      this.lastHop=new Float32Array(480);
      this.depth=Math.round(sampleRate*0.020);this.trimAt=Math.round(sampleRate*0.030);
      this.primed=false;this.streak=0;
      this.lastArr=0;this.jitter=0;this.worst=0;this.maxGap=0;
      this.packets=0;this.hops=0;this.underruns=0;this.trims=0;this.late=0;this.dropped=0;
      this.port.onmessage=(e)=>{const d=e.data;if(d&&d.pcm)this.push(d.pcm);};
    }
    push(p){
      this.packets++;
      if(this.primed && this.count===0)this.late++;
      const now=currentTime*1000;
      if(this.lastArr){
        const dt=now-this.lastArr;
        this.maxGap=Math.max(this.maxGap,dt);
        if(dt>0&&dt<500){const dev=Math.abs(dt-10);this.jitter+=0.02*(dev-this.jitter);if(dev>this.worst)this.worst=dev;}
      }
      this.lastArr=now;
      for(let i=0;i<p.length;i++){this.b[(this.head+this.count)%8192]=p[i];this.count++;}
      if(this.count>this.trimAt){const drop=this.count-this.depth;this.head=(this.head+drop)%8192;this.count-=drop;this.trims++;this.dropped+=drop;}
    }
    process(inputs,outputs){
      const out=outputs[0][0],n=out.length;
      if(!this.primed){
        // 起播 = 攒满深度（20 ms）
        if(this.count<this.depth){out.fill(0);return true;}
        this.primed=true;
      }
      if(this.count>=n){
        for(let i=0;i<n;i++){out[i]=this.b[(this.head+i)%8192];}
        this.head=(this.head+n)%8192;this.count-=n;this.hops++;this.streak=0;
        if(n===480)this.lastHop.set(out);
      }else{
        // 欠载：淡出重复上一格（绝不硬切静音）
        this.underruns++;this.streak++;
        for(let i=0;i<n;i++){out[i]=this.lastHop[i%480]*0.5*(1-i/n);}
        if(this.streak>100){this.primed=false;this.streak=0;this.lastArr=0;}
      }
      if(this.packets%5===0)this.port.postMessage({
        level:this.count,target:this.depth,jitter:this.jitter,worst:this.worst,maxGap:this.maxGap,
        underruns:this.underruns,trims:this.trims,late:this.late,dropped:this.dropped,hops:this.hops,
      });
      return true;
    }
  }
  registerProcessor('hop',H);
  registerProcessor('spk',S);`;
  workletUrl = URL.createObjectURL(new Blob([src], { type: 'application/javascript' }));
  return workletUrl;
}

/** 麦克风在**拿到权限之前**就注定拿不到的原因（空串 = 可以试）。 */
function micBlockReason() {
  // `getUserMedia` 只在安全上下文里可用。`http://192.168.x.x` 不是安全上下文，浏览器
  // 直接拒绝，**没有任何绕过办法** —— 所以二维码必须指向 https（DESIGN.md §4.1.11）。
  if (!window.isSecureContext)
    return '当前页面不是安全上下文，浏览器禁止使用麦克风。必须用 https:// 打开' +
           '（用 http:// 时无论怎么点都没用；电脑上启动服务后扫码即可）';
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia)
    return '这个浏览器不支持麦克风采集（需要 https 与较新的浏览器）';
  return '';
}

async function startMic() {
  // **先说清为什么拿不到**，不然只看到「麦克风打不开」根本不知道该做什么。这几种原因
  // 对应的动作完全不同：非安全上下文要换 https 打开；权限被拒要去地址栏改网站设置；
  // 没有麦克风是设备问题。
  const blocked = micBlockReason();
  if (blocked) { say(blocked, 'err'); throw new Error(blocked); }
  if (!codecReady) throw new Error('Opus 编解码器还没准备好');
  say('申请麦克风权限…', 'warn');
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, sampleRate: SR, echoCancellation: false, noiseSuppression: false, autoGainControl: false } });
  } catch (e) {
    let msg;
    if (e && e.name === 'NotAllowedError') {
      msg = window.isSecureContext
        ? '麦克风权限被拒绝：点地址栏左边的图标 → 网站设置 → 麦克风改成「允许」'
        : '麦克风被拒绝：页面不是安全上下文，必须用 https:// 打开';
    } else if (e && e.name === 'NotFoundError') {
      msg = '这台设备上没有找到麦克风';
    } else {
      msg = '麦克风打不开：' + (e && e.message ? e.message : String(e));
    }
    say(msg, 'err');
    throw e;
  }
  if (!actx) actx = new AudioContext({ sampleRate: SR });
  if (actx.state === 'suspended') await actx.resume();
  await actx.audioWorklet.addModule(worklet());
  micNode = new AudioWorkletNode(actx, 'hop', { numberOfInputs: 1, numberOfOutputs: 0 });
  micNode.port.onmessage = (e) => {
    if (!enc) return;
    // 每 hop 一帧 Opus（10 ms），前面加 8 字节帧头：seq + **采集时刻**（电脑用它算单向延迟）
    const f = new Float32Array(e.data);
    try {
      // 直接编码成裸 Opus 包：**不需要浏览器内置的编解码器**（WebCodecs 只有 Chrome 94+ /
      // Safari 16.4+ 才有，实测有浏览器根本没有，麦克风与扬声器一起不可用）。
      const enc8 = enc.encodeFloat(f);
      const payload = new Uint8Array(enc8.buffer, enc8.byteOffset, enc8.byteLength);
      const msg = new Uint8Array(HDR + payload.length);
      const ts = nowUs();
      const seq = micSeq++ & 0xFFFFFFFF;
      const dv = new DataView(msg.buffer);
      dv.setUint32(0, seq); dv.setUint32(4, ts);
      msg.set(payload, HDR);
      sendBin(msg.buffer);
    } catch (e) { say('Opus 编码失败：' + (e && e.message ? e.message : e), 'err'); }
  };
  const src = actx.createMediaStreamSource(stream);
  src.connect(micNode);
}

// ---- 扬声器下行：wasm 解码 → AudioWorklet 播放环（固定 20 ms） ----
async function startSpk() {
  await ensureDec();
  if (!actx) actx = new AudioContext({ sampleRate: SR });
  if (actx.state === 'suspended') await actx.resume();
  if (!spkGain) {
    spkGain = actx.createGain();
    spkGain.gain.value = 1;
    spkGain.connect(actx.destination);
  }
  if (!spkNode) {
    await actx.audioWorklet.addModule(worklet());
    spkNode = new AudioWorkletNode(actx, 'spk', {
      numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [1],
    });
    spkNode.port.onmessage = (e) => {
      const d = e.data;
      if (d && typeof d.level === 'number') {
        spkBuffered = d.level;
        spkTarget = d.target || 0;
        spkUnderruns = d.underruns || 0;
        spkLate = d.late || 0;
        spkTrims = d.trims || 0;
        spkJitter = d.jitter || 0;
        spkMaxGap = d.maxGap || 0;
      }
    };
    spkNode.connect(spkGain);
  }
  // `Sub` 是**无负载**的单位变体，只能发 {"t":"sub"}；带额外字段服务端解析不出来。
  if (!spkSub) {
    send({ t: 'sub' });
    spkSub = true;
  }
}

/** 收掉本地下行节点；`unsub` = 还要告诉服务端退订（连接断开时服务端自己会清）。 */
function stopSpkNode(unsub) {
  if (unsub) send({ t: 'unsub' });
  if (spkNode) {
    try { spkNode.disconnect(); } catch (e) { /* 已断开 */ }
    spkNode = null;
  }
  spkSub = false;
  spkBuffered = 0;
}

/** 一个收到的 Opus 包 → 解码 → 塞进 worklet 的环形缓冲（缓冲只在接收方）。 */
function playPacket(bytes, tsUs) {
  if (!spkNode || !dec) return;
  // 单向延迟：收到时刻（换算到电脑时钟）− 电脑发出时刻。需要 offset，没有就只算抖动。
  if (clockOffsetUs != null && tsUs) {
    const us = usDiff(nowUs(), tsUs) + clockOffsetUs;
    if (us >= 0 && us < 5000000) spkOneWay = us / 1000;
  }
  let pcm;
  try { pcm = dec.decodeFloat(bytes, { frameSize: HOP }); }
  catch (e) { say('Opus 解码失败：' + (e && e.message ? e.message : e), 'err'); return; }
  if (!pcm || !pcm.length) return;
  // 复制一份再转移：wasm 侧的内存不能保证在 postMessage 之后还归我们管
  const copy = new Float32Array(pcm);
  spkNode.port.postMessage({ pcm: copy }, [copy.buffer]);
}

// ---- UI ----
function bind(id, fn) {
  const b = document.getElementById(id);
  b.addEventListener('click', async () => { b.disabled = true; try { await fn(b); } catch (e) { say(String(e), 'err'); } finally { b.disabled = false; } });
}
bind('bMic', async (b) => {
  micOn = !micOn; b.setAttribute('aria-pressed', String(micOn));
  if (micOn) { await startMic(); say('麦克风已开', 'on'); }
  else if (micNode) { micNode.disconnect(); micNode = null; say('麦克风已关'); }
});

/**
 * 自动开麦：把按钮状态、开关变量一起设好，免得用户看按钮还是「关」而实际已经在推流。
 * 失败时 `startMic` 已经在状态行说明了原因，这里只负责把界面恢复原状。
 */
async function autoMic() {
  micOn = true;
  document.getElementById('bMic').setAttribute('aria-pressed', 'true');
  try { await startMic(); say('麦克风已开（连上就自动申请了权限）', 'on'); }
  catch (e) { micOn = false; document.getElementById('bMic').setAttribute('aria-pressed', 'false'); }
}
bind('bSpk', async (b) => {
  spkOn = !spkOn; b.setAttribute('aria-pressed', String(spkOn));
  if (spkOn) { await startSpk(); say('扬声器已开（自适应缓冲）', 'on'); }
  else { stopSpkNode(true); say('扬声器已关'); }
});
// 远程输入**没有客户端消息**：它是电脑端的总开关（proto.rs 里没有 remote_input），
// 客户端发过去服务端解析不出来、连接会被断开。只能提示用户在电脑上勾。
bind('bKb', () => {
  document.getElementById('kb').classList.toggle('on');
  hint.textContent = '打字与按键要电脑端也勾上「远程输入」才生效（默认关闭，同网段无鉴权）。';
});
bind('bClose', () => document.getElementById('kb').classList.remove('on'));

const text = document.getElementById('text');
function pushText() {
  const s = text.value;
  if (s) { send({ t: 'text', s }); text.value = ''; }
}
text.addEventListener('change', pushText);
text.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); pushText(); } });

// ---- 虚拟键盘：Android 键码常量（与 net/keymap.rs 对齐的子集） ----
const K = {
  '1':{w:'w2',c:'1',a:7},'2':{w:'w2',c:'2',a:8},'3':{w:'w2',c:'3',a:9},'4':{w:'w2',c:'4',a:10},
  '5':{w:'w2',c:'5',a:11},'6':{w:'w2',c:'6',a:12},'7':{w:'w2',c:'7',a:13},'8':{w:'w2',c:'8',a:14},
  '9':{w:'w2',c:'9',a:15},'0':{w:'w2',c:'0',a:7},
};
// 布局：[标签, Android keycode, 宽度倍数]
const LAYOUT = [
  [['Esc',111],['1',8],['2',9],['3',10],['4',11],['5',12],['6',13],['7',14],['8',15],['9',16],['0',7],['⌫',67]],
  [['Tab',61],['Q',24],['W',25],['E',26],['R',27],['T',28],['Y',29],['U',30],['I',31],['O',32],['P',33],['[',71],[']',72]],
  [['Caps',113],['A',29],['S',30],['D',31],['F',32],['G',33],['H',34],['J',35],['K',36],['L',37],[';',74],["'",75],['↵',66]],
  [['Shift',59],['Z',54],['X',55],['C',56],['V',57],['B',58],['N',59],['M',60],[',',55],['.',56],['/',57],['Shift',59]],
  [['Ctrl',113],['Alt',57],['',0],['空格',62],['←',21],['↑',19],['↓',20],['→',22]],
];
const WD = { 'Tab':'w2','Caps':'w2','↵':'w2','Shift':'w2','空格':'w3','Ctrl':'w3','Alt':'w3' };
LAYOUT.forEach((row, ri) => {
  const host = document.getElementById('r' + ri);
  row.forEach(([label, code]) => {
    const d = document.createElement('div');
    d.className = 'k ' + (WD[label] || '');
    d.textContent = label;
    if (code) {
      d.addEventListener('pointerdown', (e) => {
        e.preventDefault(); d.classList.add('act');
        send({ t: 'key', code, down: true });
      });
      const up = () => { d.classList.remove('act'); send({ t: 'key', code, down: false }); };
      d.addEventListener('pointerup', up);
      d.addEventListener('pointerleave', up);
      d.addEventListener('pointercancel', up);
    } else { d.style.visibility = 'hidden'; }
    host.appendChild(d);
  });
});

// ---- 能力实测：真的把 Opus 编解码器**加载起来**跑一帧，而不是只看 API 在不在 ----
// 之前只看 `typeof AudioEncoder`，那既会误判（API 存在 ≠ 支持 opus 配置），也漏掉了真正
// 的限制：WebCodecs 只有 Chrome 94+ / Safari 16.4+ 才有，实测有浏览器根本没有，于是麦克风
// 与扬声器一起不可用，而从外面看一律只是「没声音」。现在编解码在 wasm 里，只要浏览器有
// `WebAssembly` 就能跑——所以这里直接加载并编/解一帧验真，比探测任何 API 都可靠。
const caps = { secure: false, mic: false, opus: false, why: '' };

async function probeCaps() {
  caps.secure = !!window.isSecureContext;
  caps.mic = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  if (!caps.secure) { caps.why = '非安全上下文（必须 https:// 打开）'; return caps; }
  if (!caps.mic) { caps.why = '没有麦克风采集接口'; return caps; }
  try {
    await loadCodecs();
    // 真编一帧、真解一帧：能闭环才叫「支持」
    const probe = new Float32Array(HOP);           // 全零的静音帧
    const packet = enc.encodeFloat(probe);
    const back = dec.decodeFloat(packet, { frameSize: HOP });
    caps.opus = !!packet.length && back.length === HOP;
    if (!caps.opus) caps.why = 'Opus 编解码器初始化后编解码结果不对';
  } catch (e) {
    caps.opus = false;
    caps.why = 'Opus 编解码器（wasm）加载失败：' + (e && e.message ? e.message : e);
  }
  return caps;
}

function capsText() {
  const yn = (b) => b ? '有' : '无';
  return '本浏览器：' +
    (caps.secure ? '安全上下文' : '非安全上下文（麦克风一定用不了）') + ' / ' +
    '麦克风接口' + yn(caps.mic) + ' / ' +
    'Opus 编解码（wasm）' + yn(caps.opus) +
    (caps.why ? '　→　' + caps.why : '');
}

probeCaps().then((c) => {
  document.getElementById('caps').textContent = capsText();
  // 上报给电脑端：这样「为什么用不了」能在 `/debug` 里直接读到，不必靠用户描述
  try {
    const q = new URLSearchParams({
      ua: navigator.userAgent.slice(0, 60),
      secure: yn(c.secure), mic: yn(c.mic), enc: yn(c.opus),
      dec: yn(c.opus), opus: yn(c.opus), why: c.why || '无',
    });
    fetch('/report-caps?' + q.toString()).catch(() => {});
  } catch (e) { /* 上报失败不影响使用 */ }

  if (!c.opus) {
    say('这个浏览器用不了 Opus 编解码器：' + c.why +
        '。打字与键盘照常可用。', 'err');
    document.getElementById('bMic').disabled = true;
    document.getElementById('bSpk').disabled = true;
    return;   // 编解码不可用就别连了，连上也传不了音频
  }
  connect();
});
</script>
</html>
"##;
