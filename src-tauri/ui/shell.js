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

// 应用外壳（UI/UX 重做）：侧栏导航、会话列表、状态栏、调试抽屉、Toast、键盘。
//
// 纪律：
// - **不碰数据**：引擎/列/行状态一律来自 columns.js 转发的同一份快照（AGENTS §1.3），
//   这里只做「把同一份状态放到侧栏/状态栏」的呈现。
// - **只增强，不改语义**：原有按钮 ID 全部保留（设置/更新/驱动/启动…由各自模块绑定），
//   这个文件只加外壳交互（抽屉、焦点、Esc、空格、Toast、会话快捷跳转）。
;(() => {
const $ = (id) => document.getElementById(id)
const { invoke } = window.__TAURI__.core
const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
const body = document.body

// ---------- Toast ----------
let toastSeq = 0
/**
 * 一条会自己消失的提示。`level`: info | ok | warn | error。
 * 失败类必须看得见（AGENTS §1.2「不静默失败」），但仍以调试接口/面板为唯一事实源。
 */
function toast(text, level = 'info', ms = 4200) {
  const host = $('toasts')
  if (!host || !text) return
  const el = document.createElement('div')
  el.className = `toast ${level}`
  el.dataset.id = String(++toastSeq)
  const icon = document.createElement('span')
  icon.className = 'toast-icon'
  icon.textContent = level === 'error' ? '!' : level === 'warn' ? '!' : level === 'ok' ? '✓' : 'i'
  const msg = document.createElement('span')
  msg.className = 'toast-msg'
  msg.textContent = String(text)
  const close = document.createElement('button')
  close.type = 'button'
  close.className = 'toast-x'
  close.textContent = '✕'
  close.title = T('关闭')
  close.addEventListener('click', () => remove())
  el.append(icon, msg, close)
  host.appendChild(el)
  requestAnimationFrame(() => el.classList.add('in'))
  let timer = setTimeout(remove, ms)
  el.addEventListener('mouseenter', () => clearTimeout(timer))
  el.addEventListener('mouseleave', () => { timer = setTimeout(remove, 1600) })
  function remove() {
    clearTimeout(timer)
    el.classList.remove('in')
    setTimeout(() => el.remove(), 180)
  }
}
window.__pvToast = toast

// ---------- 运行状态 / 状态栏 ----------
const setDot = (el, level) => {
  if (!el) return
  el.className = 'dot ' + level
}
function paintEngine(on, level) {
  const cls = level === 'warn' ? 'warn' : on ? 'ok' : 'idle'
  const text = on ? (level === 'warn' ? T('运行中（需注意）') : T('运行中')) : T('已停止')
  for (const id of ['side-dot', 'sb-dot']) setDot($(id), cls)
  for (const id of ['side-state-text', 'sb-run', 'sb-engine']) {
    const el = $(id)
    if (el) el.textContent = text
  }
}

let lastPlan = null
function onSnapshot(s) {
  if (!s) return
  const audio = s.audio || {}
  const verdict = audio.verdict
  const warn = verdict && verdict.state === 'ok' && verdict.value && verdict.value.level === 'warn'
  const running = !!s.running
  paintEngine(running, warn ? 'warn' : 'ok')

  // 版本 / 调试地址（与调试面板同源）
  const ver = s.app && s.app.version ? String(s.app.version) : ''
  for (const id of ['side-version', 'sb-version']) {
    const el = $(id)
    if (el) el.textContent = ver ? 'v' + ver.replace(/^v/, '') : ''
  }
  const http = s.app && s.app.debug_http && s.app.debug_http.state === 'ok' ? s.app.debug_http.value : ''
  if ($('sb-http')) $('sb-http').textContent = http || ''

  // 设备数
  const d = s.devices
  if (d && d.state === 'ok' && d.value && Array.isArray(d.value.devices)) {
    const ins = d.value.devices.filter((x) => x.direction === 'input').length
    const outs = d.value.devices.filter((x) => x.direction === 'output').length
    if ($('sb-devices')) $('sb-devices').textContent = T('设备 {a} 入 / {b} 出', { a: ins, b: outs })
  } else if ($('sb-devices')) {
    $('sb-devices').textContent = T('设备 —')
  }

  const info = audio.engine_info
  const cols = info && Array.isArray(info.columns) ? info.columns : []
  if ($('sb-columns')) {
    $('sb-columns').textContent = cols.length
      ? T('{n} 列', { n: cols.length })
      : lastPlan && lastPlan.columns
        ? T('{n} 列', { n: lastPlan.columns.length })
        : ''
  }
  if ($('sb-hop')) {
    const hop = cols.length && cols[0].hop_rate ? Number(cols[0].hop_rate) : 0
    $('sb-hop').textContent = running && hop > 0 ? hop.toFixed(1) + ' hop/s' : ''
    $('sb-hop').className = 'sb-item mono' + (running && hop > 0 && Math.abs(hop - 100) > 2 ? ' warn' : '')
  }
  const health = verdict && verdict.state === 'ok' ? verdict.value : null
  const h = $('sb-health')
  if (h) {
    h.textContent = health ? health.text : ''
    h.className = 'sb-item' + (health && health.level === 'warn' ? ' warn' : health ? ' ok' : '')
    h.title = h.textContent
  }
  const gen = $('side-generation')
  if (gen) gen.textContent = info && info.generation ? 'gen ' + info.generation : ''
  paintRowDots(s)
}

/** columns.js 每次重绘计划后调用；侧栏不再重复列列表，只更新状态栏计数。 */
function onPlan(plan) {
  lastPlan = plan
  const n = $('sb-columns')
  if (n) n.textContent = T('{n} 列', { n: (plan && plan.columns ? plan.columns.length : 0) })
}

let lastSnap = null

/** 行健康点：`engine_info.columns[].rows[].health`，前端只上色。 */
function paintRowDots(s) {
  const info = s && s.audio && s.audio.engine_info
  const cols = info && Array.isArray(info.columns) ? info.columns : []
  document.querySelectorAll('.row[data-ci][data-ri]').forEach((node) => {
    const el = /** @type {HTMLElement} */ (node)
    const ci = Number(el.dataset.ci)
    const ri = Number(el.dataset.ri)
    const c = cols.find((x) => Number(x.index) === ci)
    const r = c && Array.isArray(c.rows) ? c.rows.find((x) => Number(x.index) === ri) : null
    const dot = /** @type {HTMLElement|null} */ (el.querySelector('.row-dot'))
    if (!dot) return
    const h = r && r.health
    if (h && h.state === 'ok') {
      const level = h.value && h.value.level === 'warn' ? 'warn' : 'ok'
      dot.className = 'row-dot ' + level
      dot.title = h.value ? h.value.text : ''
    } else if (h && h.state === 'unavailable') {
      dot.className = 'row-dot err'
      dot.title = h.reason || ''
    } else {
      dot.className = 'row-dot pending'
      dot.title = ''
    }
  })
}

// ---------- 应用问题弹框 ----------
/** 顶部「N 个问题」→ 完整错误（右上关闭、右下复制）。 */
function showProblems(list) {
  const box = $('problems')
  const text = $('prob-text')
  if (!box || !text) return
  text.textContent = (list || []).join('\n')
  box.classList.remove('hidden')
  $('prob-close')?.focus()
}

$('prob-close')?.addEventListener('click', () => $('problems')?.classList.add('hidden'))
$('prob-copy')?.addEventListener('click', async () => {
  const txt = $('prob-text')?.textContent || ''
  try {
    await navigator.clipboard.writeText(txt)
    toast(T('已复制'), 'ok', 2500)
  } catch (e) {
    toast(T('复制失败：') + e, 'error', 5000)
  }
})

// ---------- 内置截屏（调试抽屉标题栏） ----------
$('btn-shot')?.addEventListener('click', async () => {
  const b = /** @type {HTMLButtonElement|null} */ ($('btn-shot'))
  if (b) b.disabled = true
  try {
    const path = await invoke('capture_ui')
    toast(T('截图已保存：') + path, 'ok', 6500)
  } catch (e) {
    toast(T('截图失败：') + e, 'error', 7000)
  } finally {
    if (b) b.disabled = false
  }
})

// ---------- 面板：点遮罩 / Esc 关闭 ----------
function visiblePanel() {
  return Array.from(document.querySelectorAll('.panel')).find((p) => !p.classList.contains('hidden'))
}
function closePanel(panel) {
  if (!panel) return
  const id = panel.id
  const btn =
    id === 'settings' ? $('set-close')
    : id === 'update' ? $('up-close')
    : id === 'drivers' ? $('drv-close')
    : id === 'problems' ? $('prob-close')
    : null
  btn?.click()
}
document.addEventListener('click', (e) => {
  const panel = e.target instanceof Element ? e.target.closest('.panel') : null
  if (panel && e.target === panel) closePanel(panel)
})

// ---------- 键盘 ----------
document.addEventListener('keydown', (e) => {
  const t = e.target
  const typing =
    t instanceof HTMLElement &&
    (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)
  if (e.key === 'Escape') {
    // 面板在最上层，先关面板；没有面板才收抽屉
    const p = visiblePanel()
    if (p) {
      closePanel(p)
      return
    }
    if (body.classList.contains('debug-on')) {
      $('btn-debug')?.click()
      return
    }
    return
  }
  if (typing) return
  if (e.code === 'Space' && !(t instanceof HTMLButtonElement)) {
    e.preventDefault()
    $('btn-run')?.click()
    return
  }
  if ((e.key === 'd' || e.key === 'D') && !e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault()
    $('btn-debug')?.click()
    return
  }
  if (e.key === '?') {
    e.preventDefault()
    toast(T('快捷键：空格 启动/停止 · D 调试面板 · Esc 关闭'), 'info', 6000)
  }
})

// 语言切换：状态栏里的动态串按新语言重绘
document.addEventListener('pv-langchange', () => {
  if (lastSnap) onSnapshot(lastSnap)
  if (lastPlan) onPlan(lastPlan)
})

window.__pvShell = { onSnapshot, onPlan, toast, showProblems }

// 首屏：没有任何数据时也把状态栏/侧栏摆好（不伪造 0）
paintEngine(false)
})()
