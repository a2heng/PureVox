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

// 调试面板：只渲染 Rust 快照，不自行计算指标（AGENTS.md 1.3）。
// 页面由本机只读调试接口提供（GET /debug/ui），浏览器独立进程打开；快照由 snapshot.js 轮询，
// 这里只订阅 `pv-snapshot` 事件渲染。**不再有跨进程的东西**（截图 / 关闭窗口 / 开发者工具按钮
// 都去掉了：面板截不到主窗口的图，关标签页用浏览器自己）。
// 独立作用域（IIFE）：与其它模块共享全局，避免顶层 const 重名。
;(() => {

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

// 结论：Rust 侧判定好的 {level, text}，这里只映射颜色（AGENTS 1.3：前端不自己算）
function healthCell(p) {
  if (!p) return [T('（无数据）'), 'na']
  if (p.state !== 'ok') return probeText(p, () => '')
  return [p.value.text, p.value.level === 'warn' ? 'warn' : 'ok']
}
function putHealth(el, p) {
  const [t, c] = healthCell(p)
  put(el, t, c)
}

// 实测速率还没采够一个窗口时不编数字（后缀留空，计数照显）
function rateSuffix(r) {
  return r && r.state === 'ok' ? r.value : null
}
// 累计计数 + 实测速率：累计值只会越涨越大，速率才看得出「现在还在不在发生」
function countRate(p, r) {
  const [t, c] = probeText(p, String)
  const v = rateSuffix(r)
  if (v === null || p.state !== 'ok') return [t, c]
  return [`${t}  (${v.toFixed(1)}/s)`, c || (v > 0.5 ? 'warn' : '')]
}
function rawRate(n, r) {
  const v = rateSuffix(r)
  if (v === null) return [String(n), '']
  return [`${n}  (${v.toFixed(1)}/s)`, v > 0.5 ? 'warn' : '']
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
  ['结论', (s) => healthCell(s.health)],
  ['状态', (s) => probeText(s.state, (v) => v)],
  ['设备', (s) => [s.device_name, '']],
  ['格式', (s) => [`${s.sample_rate} Hz ${s.channels} ch ${s.sample_format}`, '']],
  ['信号源', (s) => [s.source || '—', '']],
  ['重采样', (s) => [s.resampler, '']],
  ['设备侧速率', (s) => probeText(devRate(s), hz)],
  ['引擎侧速率', (s) => probeText(engRate(s), hz)],
  ['伺服修正', (s) => probeText(s.asrc_adjust_ppm, ppm)],
  ['重采样延迟', (s) => [ms(s.resampler_delay_ms), '']],
  ['回调块', (s) => probeText(s.callback_frames, (v) => T('最近 {last}  最小 {min}  最大 {max}', { last: v.last, min: v.min, max: v.max }))],
  ['回调间隔', (s) => probeText(s.callback_frames, (v) => `${v.last_gap_ms.toFixed(1)} / ${v.max_gap_ms.toFixed(1)} ms`)],
  ['回调次数', (s) => [String(s.callbacks), '']],
  [isOutLabel('输入/输出帧', '消耗/设备帧'), (s) => [`${s.frames_in} / ${s.frames_processed}`, '']],
  ['hop/剩余', (s) => [`${s.hops} / ${s.pending_frames}`, s.pending_frames < 480 ? '' : 'na']],
  ['峰值 / RMS', (s) => {
    const [p, pc] = probeText(s.peak_dbfs, db)
    const [r] = probeText(s.rms_dbfs, db)
    return [`${p}  /  ${r}`, pc]
  }],
  [isOutLabel('环形缓冲水位', '缓冲/设备'), (s) => {
    if (!isOut(s)) return [ms(s.buffer_level_ms), '']
    const [d] = probeText(s.device_buffer_ms, ms)
    return [`${ms(s.buffer_level_ms)}  /  ${d}`, '']
  }],
  ['欠载', (s) => countRate(s.underruns, s.underrun_per_s)],
  ['重同步', (s) => countRate(s.resyncs, s.resync_per_s)],
  ['丢弃', (s) => rawRate(s.overruns, s.overrun_per_s)],
  ['错误', (s) => {
    const [t, c] = rawRate(s.stream_errors, s.stream_error_per_s)
    return [s.last_error ? `${t}  ${s.last_error}` : t, c]
  }],
  ['端到端延迟', (s) => probeText(s.latency_ms, ms)],
  ['推理 均/最大', (s) => {
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
    '<h4></h4><div class="stream-body">' +
    // 四个直接子项 = 四列（比例 2:2:3:3）：两张参数表（各一套标签+数值）在左，波形、频谱在右。
    // 参数表不要再套一层容器：套了之后 .stream-body 只有 3 个子项，第二列会整列空着。
    '<table class="kv"><tbody></tbody></table>' +
    '<table class="kv"><tbody></tbody></table>' +
    `<figure><figcaption>${T(isOut(s) ? '波形（48 kHz，最近 50 ms，送入重采样前）' : '波形（48 kHz，最近 50 ms）')}</figcaption><canvas class="wave"></canvas></figure>` +
    `<figure><figcaption>${T('平均频谱（0 ~ 24 kHz，-140 ~ 0 dBFS）')}</figcaption><canvas class="spec"></canvas></figure>` +
    '</div>'
  const half = Math.ceil(STREAM_FIELDS.length / 2)
  card.querySelectorAll('tbody').forEach((tb, col) => {
    for (let i = col * half; i < Math.min(STREAM_FIELDS.length, (col + 1) * half); i++) {
      const tr = document.createElement('tr')
      const th = document.createElement('th'); const td = document.createElement('td')
      th.textContent = labelOf(STREAM_FIELDS[i][0], s)
      tr.append(th, td)
      tb.appendChild(tr)
    }
  })
  return card
}

function renderAudio(audio) {
  putProbe($('audio-engine'), audio.engine, (v) => v)
  putHealth($('audio-verdict'), audio.verdict)
  const box = $('audio-streams')
  const ids = audio.streams.map((s) => s.id).join('|')
  if (box.dataset.ids !== ids) {
    box.dataset.ids = ids
    box.replaceChildren(...audio.streams.map(makeStreamCard))
  }
  audio.streams.forEach((s, i) => {
    const card = box.children[i]
    put(card.querySelector('h4'), isOut(s) ? T('输出：{name}', { name: s.device_name }) : T('输入：{name}', { name: s.device_name }))
    const tds = card.querySelectorAll('td')   // 两张参数表按列顺序依次排列，顺序与 makeStreamCard 一致
    STREAM_FIELDS.forEach(([, f], j) => { const [t, c] = f(s); put(tds[j], t, c) })
    drawWave(card.querySelector('.wave'), s.waveform)
    drawSpectrum(card.querySelector('.spec'), s.spectrum_db, s.spectrum_bin_hz, isOut(s) ? Infinity : s.sample_rate)
  })
}

// ---------- 网络（只读状态；开关 / 二维码 / 远程输入都在网络行卡片里，见 netrow.js）----------
// 速率没采够一个窗口时写「采集中…」，不编数字（与音频流同一套约定）。
function rateNum(p) {
  const v = rateSuffix(p)
  return v === null ? T('采集中…') : v.toFixed(1)
}

function renderNet(net) {
  const svc = net.running ? T('已启动') : T('服务未启动')
  const ri = net.remote_input ? T('已开启') : T('已关闭')
  const rtt = net.rtt_ms == null ? T('（未测）') : net.rtt_ms.toFixed(1) + ' ms'
  put(
    $('net-status'),
    `${T('服务')} ${svc} ｜ ${T('客户端 {n}，订阅 {m}', { n: net.clients, m: net.subs })} ｜ ` +
      `${T('出站 {n} 包/s', { n: rateNum(net.tx_packets_per_s) })} ｜ ${T('进站 {n} 包/s', { n: rateNum(net.rx_packets_per_s) })} ｜ ` +
      `${T('往返时延 {n}', { n: rtt })} ｜ ${T('远程输入')} ${ri}`,
  )
  // 逐路订阅：出站**没有缓冲**（推来即发），所以这里看的是「有没有积压 / 丢没丢」；
  // 真正的水位在客户端那一侧，由客户端 2 Hz 上报（`net.client`）—— 一处缓冲，一处报数。
  const subs = net.tx_subs
    .map(
      (x, i) =>
        `${i + 1}# ` +
        T('待发 {n} hop', { n: x.pending_hops }) +
        (x.dropped ? T('（丢 {n}）', { n: x.dropped }) : ''),
    )
    .join('  ')
  const [, dc] = rawRate(net.tx_dropped, net.tx_dropped_per_s)
  // 上行（手机 → 电脑）：单向延迟 + 接收环水位/目标 + jitter
  const upLat =
    net.rx_latency_ms == null
      ? T('（测时延中）')
      : net.rx_latency_ms.toFixed(1) + ' ms'
  const up = `${T('上行')} ${upLat} ｜ ` +
    T('水位 {a}/{b} ms', {
      a: net.rx_level_ms.toFixed(0),
      b: net.rx_target_ms.toFixed(0),
    }) +
    // jitter / 最大间隔 / 裁剪 / 欠载 / PLC / 丢包：**全量**摆出来（测量不花钱）
    ` ｜ jitter ${net.rx_jitter_ms.toFixed(1)}/${net.rx_worst_jitter_ms.toFixed(1)} ms` +
    ` ｜ ${T('最大间隔 {n} ms', { n: net.rx_max_gap_ms.toFixed(0) })}` +
    (net.rx_trims ? ' ｜ ' + T('裁 {n} 格', { n: net.rx_trims }) : '') +
    (net.rx_underruns ? ' ｜ ' + T('欠载 {n}', { n: net.rx_underruns }) : '') +
    (net.rx_plc ? ' ｜ ' + T('PLC {n}', { n: net.rx_plc }) : '') +
    (net.rx_lost ? ' ｜ ' + T('丢包 {n}', { n: net.rx_lost }) : '') +
    (net.rx_late ? ' ｜ ' + T('空环到达 {n}', { n: net.rx_late }) : '')
  // 下行（电脑 → 手机）：客户端自己那一环 —— 延迟真正的落点
  const c = net.client
  const down = c
    ? `${T('手机环')} ${c.level_ms.toFixed(0)}/${c.target_ms.toFixed(0)} ms` +
      (c.one_way_ms == null ? '' : ' ｜ ' + T('下行 {n} ms', { n: c.one_way_ms.toFixed(1) })) +
      (c.underruns ? ' ｜ ' + T('欠载 {n}', { n: c.underruns }) : '') +
      (c.concealed ? ' ｜ ' + T('掩盖 {n}', { n: c.concealed }) : '') +
      (c.clock_ppm == null
        ? ' ｜ ' + T('时钟偏差测时中')
        : ' ｜ ' + T('时钟 {n} ppm', { n: c.clock_ppm.toFixed(0) }))
    : T('（客户端未上报）')
  put($('net-subs'), `${subs || T('没有订阅')} ｜ ${up} ｜ ${down}`, dc)
  putHealth($('net-verdict'), { state: 'ok', value: net.verdict })
  putProbe($('net-codec'), net.codec, (v) => v)
}

// ---------- 轮询 ----------
function renderRecorder(s) {
  putProbe($('audio-recorder'), s.recorder, (v) => v)
  putProbe($('audio-calib'), s.calib, (v) => v)
}

let reported = false
let last = /** @type {PvSnapshot | null} */ (null)

function renderSnapshot(s) {
  last = s
  renderApp(s)
  renderSystem(s.system)
  renderGpu(s.system.gpu)
  renderAudio(s.audio)
  renderRecorder(s)
  renderNet(s.net)
  if (window.__pvOnSnapshot) window.__pvOnSnapshot(s)
  put($('dbg-status'), '', '')
  if (!reported) {
    reported = true
    window.__pvDebug?.(`调试面板已渲染快照（流 ${s.audio.streams.length} 路）`)
  }
}

// 快照轮询由 `snapshot.js` 统一负责（主窗口与调试窗口同一份实现、同一频率）；
// 这里只订阅事件并渲染，不再自己 invoke / setTimeout（否则就是两条轮询路径）。
document.addEventListener('pv-snapshot', (e) => {
  try {
    renderSnapshot(/** @type {CustomEvent} */ (e).detail)
  } catch (err) {
    put($('dbg-status'), T('渲染快照失败：') + err, 'na')
    window.__pvDebug?.('调试面板渲染快照失败：' + err)
  }
})
document.addEventListener('pv-snapshot-error', (e) => {
  put($('dbg-status'), T('取快照失败：') + /** @type {CustomEvent} */ (e).detail, 'na')
})

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
  renderNet(last.net)
})

})()
