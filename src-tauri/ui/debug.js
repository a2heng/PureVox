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

function renderAudio(audio) {
  putProbe($('audio-engine'), audio.engine, (v) => v)
  put($('audio-streams'), audio.streams.length ? `${audio.streams.length} 路` : '无')
}

let devStamp = null
function renderDevices(dev) {
  const tbody = $('dev-table').tBodies[0]
  if (dev.state !== 'ok') {
    putProbe($('dev-state'), dev, () => '')
    return
  }
  put($('dev-state'), '')
  const v = dev.value
  if (v.enumerated_at === devStamp) return // 列表只在重新枚举后重建
  devStamp = v.enumerated_at
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
    renderDevices(s.devices)
    put($('dbg-status'), '', '')
  } catch (e) {
    put($('dbg-status'), '取快照失败：' + e, 'na')
  }
  setTimeout(tick, POLL_MS)
}

$('dev-refresh').addEventListener('click', () => invoke('refresh_devices'))
tick()
