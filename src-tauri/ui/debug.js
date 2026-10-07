/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
 * PureVox is licensed under the GNU General Public License v3.0 or
 * later (GPL-3.0-or-later).  See LICENSE for details.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * The built-in AI models are NOT covered by the GPL; they are the
 * property of a2heng and may only be used with PureVox under
 * authorization.  See MODEL-LICENSE.md for details.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

// 调试列：只渲染 Rust 快照（debug_snapshot），不自行计算指标（AGENTS.md 1.3）。
// 独立作用域（IIFE）：与 columns.js 共享全局，避免顶层 const 重名。
;(() => {
const { invoke } = window.__TAURI__.core

const POLL_MS = 500 // 2 Hz
const $ = (id) => document.getElementById(id)
const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)

// ---------- 格式化：固定宽度 ----------
const pct = (v) => v.toFixed(1).padStart(5) + ' %'
const bytes = (b) =>
  b >= 2 ** 30 ? (b / 2 ** 30).toFixed(2).padStart(6) + ' GiB' : (b / 2 ** 20).toFixed(1).padStart(6) + ' MiB'
const two = (n) => String(n).padStart(2, '0')
const duration = (ms) => {
  const s = Math.floor(ms / 1000)
  return `${two(Math.floor(s / 3600))}:${two(Math.floor(s / 60) % 60)}:${two(s % 60)}`
}
const clock = (ts) => {
  const d = new Date(ts)
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}.${String(d.getMilliseconds()).padStart(3, '0')}`
}

// Probe：{state:'ok', value} | {state:'pending'} | {state:'unavailable', reason}
function probeText(p, fmt) {
  if (!p) return [T('（无数据）'), 'na']
  if (p.state === 'ok') return [fmt(p.value), '']
  if (p.state === 'pending') return [T('采集中…'), 'pending']
  return [T('不可用：') + p.reason, 'na']
}
function put(el, text, cls = '') {
  if (el.textContent !== text) el.textContent = text
  el.className = cls
  el.title = text
}
function putProbe(el, p, fmt) {
  const [t, c] = probeText(p, fmt)
  put(el, t, c)
}
function putFrom(el, parent, pick) {
  if (parent.state !== 'ok') return putProbe(el, parent, () => '')
  pick(el, parent.value)
}

// ---------- 调试列：应用 / 系统 / GPU ----------
function renderApp(s) {
  put($('app-version'), s.app.version)
  put($('app-pid'), String(s.app.pid))
  put($('app-uptime'), duration(s.uptime_ms))
  put($('app-ts'), clock(s.ts))
  putProbe($('app-http'), s.app.debug_http, (v) => v)
}

function renderSystem(sys) {
  put($('sys-sampled'), sys.sampled_at ? T('采样于 {time}', { time: clock(sys.sampled_at) }) : '')
  putFrom($('cpu-system'), sys.cpu, (el, v) => put(el, pct(v.system_pct)))
  putFrom($('cpu-process'), sys.cpu, (el, v) => put(el, pct(v.process_pct)))
  putFrom($('cpu-cores'), sys.cpu, (el, v) => put(el, String(v.logical_cores)))
  putFrom($('mem-used'), sys.memory, (el, v) =>
    put(el, `${bytes(v.system_total - v.system_available)} / ${bytes(v.system_total)}`))
  putFrom($('mem-avail'), sys.memory, (el, v) => put(el, bytes(v.system_available)))
  putFrom($('mem-ws'), sys.memory, (el, v) => put(el, bytes(v.process_working_set)))
  putFrom($('mem-private'), sys.memory, (el, v) => putProbe(el, v.process_private, bytes))
}

function renderGpu(gpu) {
  const tbody = /** @type {HTMLTableElement} */ ($('gpu-table')).tBodies[0]
  if (gpu.state !== 'ok') {
    putProbe($('gpu-state'), gpu, () => '')
    tbody.replaceChildren()
    return
  }
  put($('gpu-state'), '')
  const list = gpu.value
  if (tbody.rows.length !== list.length) {
    tbody.replaceChildren(...list.map(() => {
      const tr = document.createElement('tr')
      for (let i = 0; i < 5; i++) tr.appendChild(document.createElement('td'))
      return tr
    }))
  }
  list.forEach((a, i) => {
    const c = tbody.rows[i].cells
    put(c[0], `${a.name}  (${a.vendor_id})`)
    putProbe(c[1], a.utilization_pct, pct)
    putProbe(c[2], a.process_utilization_pct, pct)
    putProbe(c[3], a.dedicated_used, (u) => `${bytes(u)} / ${bytes(a.dedicated_total)}`)
    putProbe(c[4], a.process_dedicated_used, bytes)
  })
}

// ---------- 调试列：音频流 ----------
const db = (v) => v.toFixed(1).padStart(6) + ' dBFS'
const hz = (v) => v.toFixed(1).padStart(8) + ' Hz'
const ms = (v) => v.toFixed(2).padStart(6) + ' ms'
const ppm = (v) => (v >= 0 ? '+' : '') + v.toFixed(0).padStart(6) + ' ppm'
const isOut = (s) => s.direction === 'output'
const devRate = (s) => (isOut(s) ? s.measured_output_rate : s.measured_input_rate)
const engRate = (s) => (isOut(s) ? s.measured_input_rate : s.measured_output_rate)

function isOutLabel(inLabel, outLabel) {
  return { in: inLabel, out: outLabel }
}
/** @param {string | {in: string, out: string}} l @param {any} s */
const labelOf = (l, s) => {
  const t = typeof l === 'string' ? l : isOut(s) ? l.out : l.in
  return T(t)
}

/** @type {Array<[string | {in: string, out: string}, (s: any) => string[]]>} */
const STREAM_FIELDS = [
  ['状态', (s) => probeText(s.state, (v) => v)],
  ['设备', (s) => [s.device_name, '']],
  ['设备格式', (s) => [`${s.sample_rate} Hz ${s.channels} ch ${s.sample_format}`, '']],
  ['重采样', (s) => [s.resampler, '']],
  ['设备侧速率', (s) => probeText(devRate(s), hz)],
  ['引擎侧速率（48k）', (s) => probeText(engRate(s), hz)],
  ['时钟伺服修正', (s) => probeText(s.asrc_adjust_ppm, ppm)],
  ['重采样延迟', (s) => [ms(s.resampler_delay_ms), '']],
  ['回调块（帧）', (s) => probeText(s.callback_frames, (v) => T('最近 {last}  最小 {min}  最大 {max}', { last: v.last, min: v.min, max: v.max }))],
  ['回调次数', (s) => [String(s.callbacks), '']],
  [isOutLabel('输入帧 / 输出帧', '48k 消耗帧 / 设备帧'), (s) => [`${s.frames_in} / ${s.frames_processed}`, '']],
  ['hop 数 / 剩余帧', (s) => [`${s.hops} / ${s.pending_frames}`, s.pending_frames < 480 ? '' : 'na']],
  ['峰值 / RMS', (s) => {
    const [p, pc] = probeText(s.peak_dbfs, db)
    const [r] = probeText(s.rms_dbfs, db)
    return [`${p}  /  ${r}`, pc]
  }],
  [isOutLabel('环形缓冲水位', '48k 缓冲 / 设备缓冲'), (s) => {
    if (!isOut(s)) return [ms(s.buffer_level_ms), '']
    const [d] = probeText(s.device_buffer_ms, ms)
    return [`${ms(s.buffer_level_ms)}  /  ${d}`, '']
  }],
  ['欠载（补静音样本）', (s) => probeText(s.underruns, String)],
  ['重同步次数', (s) => probeText(s.resyncs, String)],
  ['丢弃样本 / 流错误', (s) => [`${s.overruns} / ${s.stream_errors}${s.last_error ? '  ' + s.last_error : ''}`, s.overruns || s.stream_errors ? 'na' : '']],
  ['端到端延迟', (s) => probeText(s.latency_ms, ms)],
  ['推理耗时 均值 / 最大', (s) => {
    const [a, c] = probeText(s.inference_ms_avg, ms)
    const [b] = probeText(s.inference_ms_max, ms)
    return [`${a}  /  ${b}`, c]
  }],
  ['', () => ['', '']],
]

function cssVar(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

function fitCanvas(cv) {
  const r = cv.getBoundingClientRect()
  const dpr = window.devicePixelRatio || 1
  const w = Math.max(1, Math.round(r.width * dpr))
  const h = Math.max(1, Math.round(r.height * dpr))
  if (cv.width !== w || cv.height !== h) { cv.width = w; cv.height = h }
  return [cv.getContext('2d'), w, h, dpr]
}

function drawWave(cv, wave) {
  const [g, w, h, dpr] = fitCanvas(cv)
  g.clearRect(0, 0, w, h)
  const left = 32 * dpr
  const pw = Math.max(1, w - left)
  // 上下限标尺（幅度 ±1.0）
  g.font = `${9 * dpr}px Consolas, monospace`
  g.strokeStyle = cssVar('--line')
  g.fillStyle = cssVar('--muted')
  for (const [t, v] of /** @type {Array<[string, number]>} */ ([['+1.0', 1], ['0', 0], ['-1.0', -1]])) {
    const y = h / 2 - (v * h) / 2
    g.beginPath()
    g.moveTo(left, y)
    g.lineTo(w, y)
    g.stroke()
    g.fillText(t, 2 * dpr, Math.min(h - 1, Math.max(8 * dpr, y + 3 * dpr)))
  }
  if (!wave.length) return
  g.strokeStyle = cssVar('--ok')
  g.beginPath()
  const per = wave.length / pw
  for (let x = 0; x < pw; x++) {
    let lo = 1
    let hi = -1
    const a = Math.floor(x * per)
    const b = Math.max(a + 1, Math.floor((x + 1) * per))
    for (let i = a; i < b && i < wave.length; i++) {
      lo = Math.min(lo, wave[i])
      hi = Math.max(hi, wave[i])
    }
    const px = left + x + 0.5
    g.moveTo(px, h / 2 - (hi * h) / 2)
    g.lineTo(px, h / 2 - (lo * h) / 2 + 1)
  }
  g.stroke()
}

const SPEC_MIN = -140, SPEC_MAX = 0
function drawSpectrum(cv, spec, binHz, nativeRate) {
  const [g, w, h, dpr] = fitCanvas(cv)
  g.clearRect(0, 0, w, h)
  const left = 36 * dpr
  const pw = Math.max(1, w - left)
  const maxHz = binHz * (spec.length - 1 || 480)
  g.font = `${9 * dpr}px Consolas, monospace`
  g.fillStyle = cssVar('--muted')
  g.strokeStyle = cssVar('--line')
  // 上下限标尺（dBFS 0 ~ -140，每 20 dB 一格）
  for (let v = 0; v >= SPEC_MIN; v -= 20) {
    const y = h - ((v - SPEC_MIN) / (SPEC_MAX - SPEC_MIN)) * h
    g.beginPath()
    g.moveTo(left, y)
    g.lineTo(w, y)
    g.stroke()
    g.fillText(String(v), 2 * dpr, Math.min(h - 1, Math.max(8 * dpr, y + 3 * dpr)))
  }
  // 横轴（频率，每 4 kHz）
  for (let f = 4000; f < maxHz; f += 4000) {
    const x = left + (f / maxHz) * pw
    g.beginPath()
    g.moveTo(x, 0)
    g.lineTo(x, h)
    g.stroke()
    g.fillText(`${f / 1000}k`, x + 2 * dpr, h - 3 * dpr)
  }
  const nyq = nativeRate / 2
  if (nyq < maxHz) {
    const x = left + (nyq / maxHz) * pw
    g.strokeStyle = cssVar('--warn')
    g.setLineDash([4 * dpr, 3 * dpr])
    g.beginPath()
    g.moveTo(x, 0)
    g.lineTo(x, h)
    g.stroke()
    g.setLineDash([])
    g.fillStyle = cssVar('--warn')
    g.fillText(T('原生奈奎斯特 {f}k', { f: (nyq / 1000).toFixed(2) }), x + 3 * dpr, 12 * dpr)
  }
  if (!spec.length) return
  g.strokeStyle = cssVar('--ok')
  g.beginPath()
  spec.forEach((v, k) => {
    const x = left + (k / (spec.length - 1)) * pw
    const y = h - ((Math.max(SPEC_MIN, v) - SPEC_MIN) / (SPEC_MAX - SPEC_MIN)) * h
    k ? g.lineTo(x, y) : g.moveTo(x, y)
  })
  g.stroke()
}

function makeStreamCard(s) {
  const card = document.createElement('div')
  card.className = 'stream'
  card.innerHTML =
    '<h4></h4><table class="kv"><tbody></tbody></table>' +
    '<div class="plots">' +
    `<figure><figcaption>${T(isOut(s) ? '波形（48 kHz，最近 50 ms，送入重采样前）' : '波形（48 kHz，最近 50 ms）')}</figcaption><canvas class="wave"></canvas></figure>` +
    `<figure><figcaption>${T('平均频谱（0 ~ 24 kHz，-140 ~ 0 dBFS）')}</figcaption><canvas class="spec"></canvas></figure>` +
    '</div>'
  const tbody = card.querySelector('tbody')
  for (let i = 0; i < STREAM_FIELDS.length; i += 2) {
    const tr = document.createElement('tr')
    for (let j = i; j < i + 2; j++) {
      const th = document.createElement('th'); const td = document.createElement('td')
      th.textContent = STREAM_FIELDS[j] ? labelOf(STREAM_FIELDS[j][0], s) : ''
      tr.append(th, td)
    }
    tbody.appendChild(tr)
  }
  return card
}

function renderAudio(audio) {
  putProbe($('audio-engine'), audio.engine, (v) => v)
  const box = $('audio-streams')
  const ids = audio.streams.map((s) => s.id).join('|')
  if (box.dataset.ids !== ids) {
    box.dataset.ids = ids
    box.replaceChildren(...audio.streams.map(makeStreamCard))
  }
  audio.streams.forEach((s, i) => {
    const card = box.children[i]
    put(card.querySelector('h4'), isOut(s) ? T('输出：{name}', { name: s.device_name }) : T('输入：{name}', { name: s.device_name }))
    const tds = card.querySelectorAll('td')
    STREAM_FIELDS.forEach(([, f], j) => { const [t, c] = f(s); put(tds[j], t, c) })
    drawWave(card.querySelector('.wave'), s.waveform)
    drawSpectrum(card.querySelector('.spec'), s.spectrum_db, s.spectrum_bin_hz, isOut(s) ? Infinity : s.sample_rate)
  })
}

// ---------- 轮询 ----------
function renderRecorder(s) {
  putProbe($('audio-recorder'), s.recorder, (v) => v)
  putProbe($('audio-calib'), s.calib, (v) => v)
}

let reported = false
let last = /** @type {PvSnapshot | null} */ (null)
async function tick() {
  try {
    const s = /** @type {PvSnapshot} */ (await invoke('debug_snapshot'))
    last = s
    renderApp(s)
    renderSystem(s.system)
    renderGpu(s.system.gpu)
    renderAudio(s.audio)
    renderRecorder(s)
    if (window.__pvOnSnapshot) window.__pvOnSnapshot(s)
    put($('dbg-status'), '', '')
    if (!reported) {
      reported = true
      window.__pvDebug?.(`调试面板已渲染快照（流 ${s.audio.streams.length} 路）`)
    }
  } catch (e) {
    put($('dbg-status'), T('取快照失败：') + e, 'na')
    window.__pvDebug?.('调试面板取快照失败：' + e)
  }
  setTimeout(tick, POLL_MS)
}

// 语言切换：立即按新语言重绘（流卡片的标签 / 图注在建卡时定型，需强制重建卡片）
document.addEventListener('pv-langchange', () => {
  if (!last) return
  const box = $('audio-streams')
  if (box) box.dataset.ids = ''
  renderApp(last)
  renderSystem(last.system)
  renderGpu(last.system.gpu)
  renderAudio(last.audio)
  renderRecorder(last)
})

tick()
})()
