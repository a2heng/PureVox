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

// 调试面板：只渲染 Rust 快照（debug_snapshot），不自行计算任何指标（AGENTS.md 1.3）。
const { invoke } = window.__TAURI__.core

const POLL_MS = 500 // 2 Hz
const $ = (id) => document.getElementById(id)

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
  if (!p) return ['（无数据）', 'na']
  if (p.state === 'ok') return [fmt(p.value), '']
  if (p.state === 'pending') return ['采集中…', 'pending']
  return ['不可用：' + p.reason, 'na']
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
// 父级 Probe 不可用时，各子项显示同一原因
function putFrom(el, parent, pick) {
  if (parent.state !== 'ok') return putProbe(el, parent, () => '')
  pick(el, parent.value)
}

// ---------- 各区块 ----------
function renderApp(s) {
  put($('app-version'), s.app.version)
  put($('app-pid'), String(s.app.pid))
  put($('app-uptime'), duration(s.uptime_ms))
  put($('app-ts'), clock(s.ts))
  putProbe($('app-http'), s.app.debug_http, (v) => v)
}

function renderSystem(sys) {
  put($('sys-sampled'), sys.sampled_at ? '采样于 ' + clock(sys.sampled_at) : '')
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
  const tbody = $('gpu-table').tBodies[0]
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

// ---------- 音频流 ----------
const db = (v) => v.toFixed(1).padStart(6) + ' dBFS'
const hz = (v) => v.toFixed(1).padStart(8) + ' Hz'
const ms = (v) => v.toFixed(2).padStart(6) + ' ms'

const ppm = (v) => (v >= 0 ? '+' : '') + v.toFixed(0).padStart(6) + ' ppm'
const isOut = (s) => s.direction === 'output'
// measured_input_rate / measured_output_rate 是「进 / 出本流重采样器」的速率，按方向换算成设备侧 / 引擎侧
const devRate = (s) => (isOut(s) ? s.measured_output_rate : s.measured_input_rate)
const engRate = (s) => (isOut(s) ? s.measured_input_rate : s.measured_output_rate)

// 每路流卡片里的文本项：[标签, 取值函数]（两列一行，保持偶数项）
const STREAM_FIELDS = [
  ['状态', (s) => probeText(s.state, (v) => v)],
  ['信号源', (s) => [s.source ?? '—（输入流）', '']],
  ['降噪', (s) => probeText(s.denoise, (v) => v)],
  ['设备格式', (s) => [`${s.sample_rate} Hz ${s.channels} ch ${s.sample_format}`, '']],
  ['重采样', (s) => [s.resampler, '']],
  ['设备侧速率', (s) => probeText(devRate(s), hz)],
  ['引擎侧速率（48k）', (s) => probeText(engRate(s), hz)],
  ['时钟伺服修正', (s) => probeText(s.asrc_adjust_ppm, ppm)],
  ['重采样延迟', (s) => [ms(s.resampler_delay_ms), '']],
  ['回调块（帧）', (s) => probeText(s.callback_frames, (v) => `最近 ${v.last}  最小 ${v.min}  最大 ${v.max}`)],
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
  ['欠载（补静音样本）', (s) => {
    const [t, c] = probeText(s.underruns, String)
    return [t, c || (s.underruns.state === 'ok' && s.underruns.value > 0 ? 'na' : '')]
  }],
  ['重同步次数', (s) => {
    const [t, c] = probeText(s.resyncs, String)
    return [t, c || (s.resyncs.state === 'ok' && s.resyncs.value > 0 ? 'na' : '')]
  }],
  ['丢弃样本 / 流错误', (s) => [`${s.overruns} / ${s.stream_errors}${s.last_error ? '  ' + s.last_error : ''}`, s.overruns || s.stream_errors ? 'na' : '']],
  ['端到端延迟', (s) => probeText(s.latency_ms, ms)],
  ['推理耗时 均值 / 最大', (s) => {
    const [a, c] = probeText(s.inference_ms_avg, ms)
    const [b] = probeText(s.inference_ms_max, ms)
    return [`${a}  /  ${b}`, c]
  }],
  ['', () => ['', '']],
]
// 标签随方向变化的项：渲染时按流方向取
function isOutLabel(inLabel, outLabel) {
  return { in: inLabel, out: outLabel }
}
const labelOf = (l, s) => (typeof l === 'string' ? l : isOut(s) ? l.out : l.in)

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
  const [g, w, h] = fitCanvas(cv)
  g.clearRect(0, 0, w, h)
  g.strokeStyle = cssVar('--line'); g.beginPath(); g.moveTo(0, h / 2); g.lineTo(w, h / 2); g.stroke()
  if (!wave.length) return
  g.strokeStyle = cssVar('--ok'); g.beginPath()
  // 每个像素列画该列样本的最小 / 最大值
  const per = wave.length / w
  for (let x = 0; x < w; x++) {
    let lo = 1, hi = -1
    const a = Math.floor(x * per), b = Math.max(a + 1, Math.floor((x + 1) * per))
    for (let i = a; i < b && i < wave.length; i++) { lo = Math.min(lo, wave[i]); hi = Math.max(hi, wave[i]) }
    g.moveTo(x + 0.5, h / 2 - hi * h / 2); g.lineTo(x + 0.5, h / 2 - lo * h / 2 + 1)
  }
  g.stroke()
}

const SPEC_MIN = -140, SPEC_MAX = 0
function drawSpectrum(cv, spec, binHz, nativeRate) {
  const [g, w, h, dpr] = fitCanvas(cv)
  g.clearRect(0, 0, w, h)
  const maxHz = binHz * (spec.length - 1 || 480)
  g.font = `${10 * dpr}px Consolas, monospace`
  g.fillStyle = cssVar('--muted'); g.strokeStyle = cssVar('--line')
  for (let f = 4000; f < maxHz; f += 4000) {
    const x = (f / maxHz) * w
    g.beginPath(); g.moveTo(x, 0); g.lineTo(x, h); g.stroke()
    g.fillText(`${f / 1000}k`, x + 2 * dpr, h - 3 * dpr)
  }
  // 原生奈奎斯特频率：重采样后此线以上应无信号
  const nyq = nativeRate / 2
  if (nyq < maxHz) {
    const x = (nyq / maxHz) * w
    g.strokeStyle = cssVar('--warn'); g.setLineDash([4 * dpr, 3 * dpr])
    g.beginPath(); g.moveTo(x, 0); g.lineTo(x, h); g.stroke(); g.setLineDash([])
    g.fillStyle = cssVar('--warn'); g.fillText(`原生奈奎斯特 ${(nyq / 1000).toFixed(2)}k`, x + 3 * dpr, 12 * dpr)
  }
  if (!spec.length) return
  g.strokeStyle = cssVar('--ok'); g.beginPath()
  spec.forEach((v, k) => {
    const x = (k / (spec.length - 1)) * w
    const y = h - ((Math.max(SPEC_MIN, v) - SPEC_MIN) / (SPEC_MAX - SPEC_MIN)) * h
    k ? g.lineTo(x, y) : g.moveTo(x, y)
  })
  g.stroke()
}

function makeStreamCard(s) {
  const card = document.createElement('div')
  card.className = 'stream'
  card.innerHTML =
    '<h3></h3><table class="kv"><tbody></tbody></table>' +
    '<div class="plots">' +
    `<figure><figcaption>波形（48 kHz，最近 50 ms${isOut(s) ? '，送入重采样前' : ''}）</figcaption><canvas class="wave"></canvas></figure>` +
    '<figure><figcaption>平均频谱（0 ~ 24 kHz，-140 ~ 0 dBFS）</figcaption><canvas class="spec"></canvas></figure>' +
    '</div>'
  const tbody = card.querySelector('tbody')
  // 两列一行
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
    put(card.querySelector('h3'), isOut(s) ? `输出：${s.device_name}  ←  ${s.source}` : `输入：${s.device_name}`)
    const tds = card.querySelectorAll('td')
    STREAM_FIELDS.forEach(([, f], j) => { const [t, c] = f(s); put(tds[j], t, c) })
    drawWave(card.querySelector('.wave'), s.waveform)
    // 输出流的频谱测的是送入重采样前的 48k 信号，不标设备奈奎斯特线
    drawSpectrum(card.querySelector('.spec'), s.spectrum_db, s.spectrum_bin_hz, isOut(s) ? Infinity : s.sample_rate)
  })
}

let devStamp = null
function renderDevices(dev, streams) {
  const tbody = $('dev-table').tBodies[0]
  if (dev.state !== 'ok') {
    putProbe($('dev-state'), dev, () => '')
    return
  }
  put($('dev-state'), '')
  const v = dev.value
  // 输出可选的信号源：测试音 + 正在采集的输入
  const sources = [['tone', '测试音 1 kHz']].concat(
    streams.filter((s) => s.direction === 'input').map((s) => [s.device_id, '输入：' + s.device_name]))
  // 每路输入的降噪状态从对应音频流推出（开启 = denoise 为 ok）
  const dnOf = (id) => {
    const s = streams.find((x) => x.direction === 'input' && x.device_id === id)
    return s ? s.denoise.state : '-'
  }
  // 列表只在重新枚举、打开状态、信号源或降噪状态变化后重建
  const stamp = v.enumerated_at + ':' + v.devices.map((d) => (d.opened ? 1 : 0)).join('') + ':' +
    sources.map((x) => x[0]).join('|') + ':' + v.devices.filter((d) => d.direction === 'input').map((d) => dnOf(d.id)).join(',')
  if (stamp === devStamp) return
  devStamp = stamp
  put($('dev-meta'), `枚举于 ${clock(v.enumerated_at)}，耗时 ${v.duration_ms} ms，接口 ${v.hosts.join(' / ') || '无'}，共 ${v.devices.length} 项`)
  const sorted = [...v.devices].sort((a, b) => a.direction.localeCompare(b.direction) || Number(b.is_default) - Number(a.is_default))
  tbody.replaceChildren(...sorted.map((d) => {
    const tr = document.createElement('tr')
    tr.title = d.id
    const cells = [
      [d.direction === 'input' ? '输入' : '输出', ''],
      [d.is_default ? '是' : '', d.is_default ? 'ok' : ''],
      [d.name, ''],
      [d.host, ''],
      probeText(d.native, (n) => `${n.sample_rate} Hz ${n.channels} ch ${n.sample_format}`),
      [`${d.device_type} / ${d.interface_type}`, ''],
    ]
    for (const [t, c] of cells) {
      const td = document.createElement('td')
      put(td, t, c)
      tr.appendChild(td)
    }
    const act = document.createElement('td')
    if (d.direction === 'input') {
      const btn = document.createElement('button')
      btn.type = 'button'
      btn.textContent = d.opened ? '停止' : '采集'
      btn.addEventListener('click', async () => {
        btn.disabled = true
        try {
          await invoke(d.opened ? 'stop_capture' : 'start_capture', { deviceId: d.id })
          put($('dev-action'), '')
        } catch (e) {
          put($('dev-action'), `${d.opened ? '停止' : '采集'} ${d.name} 失败：${e}`, 'na')
          btn.disabled = false
        }
      })
      act.appendChild(btn)
      // 降噪开关（仅采集运行时可用）
      const dnOn = dnOf(d.id) === 'ok'
      const dn = document.createElement('button')
      dn.type = 'button'
      dn.textContent = dnOn ? '降噪：开' : '降噪：关'
      dn.disabled = !d.opened
      dn.addEventListener('click', async () => {
        dn.disabled = true
        try {
          await invoke('set_denoise', { deviceId: d.id, on: !dnOn })
          put($('dev-action'), '')
        } catch (e) {
          put($('dev-action'), `切换降噪失败：${e}`, 'na')
          dn.disabled = false
        }
      })
      act.appendChild(dn)
    } else {
      const sel = document.createElement('select')
      sel.setAttribute('aria-label', '信号源')
      for (const [id, label] of sources) sel.add(new Option(label, id))
      sel.disabled = d.opened
      const btn = document.createElement('button')
      btn.type = 'button'
      btn.textContent = d.opened ? '停止' : '播放'
      btn.addEventListener('click', async () => {
        btn.disabled = true
        try {
          if (d.opened) await invoke('stop_playback', { deviceId: d.id })
          else await invoke('start_playback', { deviceId: d.id, source: sel.value })
          put($('dev-action'), '')
        } catch (e) {
          put($('dev-action'), `${d.opened ? '停止' : '播放'} ${d.name} 失败：${e}`, 'na')
          btn.disabled = false
        }
      })
      act.append(sel, btn)
    }
    tr.appendChild(act)
    return tr
  }))
  $('dev-errors').replaceChildren(...v.errors.map((e) => {
    const li = document.createElement('li')
    li.textContent = e
    return li
  }))
}

// ---------- 轮询 ----------
async function tick() {
  try {
    const s = await invoke('debug_snapshot')
    renderApp(s)
    renderSystem(s.system)
    renderGpu(s.system.gpu)
    renderAudio(s.audio)
    renderDevices(s.devices, s.audio.streams)
    put($('dbg-status'), '', '')
  } catch (e) {
    put($('dbg-status'), '取快照失败：' + e, 'na')
  }
  setTimeout(tick, POLL_MS)
}

$('dev-refresh').addEventListener('click', () => invoke('refresh_devices'))
tick()
