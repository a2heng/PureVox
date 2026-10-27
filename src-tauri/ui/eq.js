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

// 31 段均衡（处理行 `eq31`）的卡片：**频响曲线 + 可拖频段 + 低切/高切 + 6+6 预设**。
//
// DSP 全在 Rust（`biquad` crate，见 components/eq31.rs）；本文件只做交互与绘制：
// - 拖动频段 → 改本地 settings → 节流 `set_row_param(settings)`（热参数，不重建会话）；
// - 曲线来自 `eq_curve` 命令（与音频路径同一套系数，界面不自己算频响）；
// - 预设：上面 6 个内置、下面 6 个自定义槽，**两行六列、左右对齐**；
//   单击应用；自定义槽**长按 0.6 s** 保存当前设置（不弹输入框、不做删除）；
// - 低切 / 高切是独立开关，频响会画进同一条曲线。
;(function () {
  'use strict'

  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  const call = (cmd, args) => window.__TAURI__?.core?.invoke(cmd, args)

  const BANDS = 31
  /** ISO 1/3 倍频程中心频率（与 Rust `CENTER_HZ` 一致） */
  const CENTERS = [
    20, 25, 31.5, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000,
    1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000,
  ]
  const DB_RANGE = 24
  const CUSTOM_SLOTS = 6
  const STORE = 'pv.eq31.presets'

  /** 6 个内置预设（31 个 dB，20 Hz → 20 kHz） */
  const BUILTIN = {
    [T('平坦')]: new Array(BANDS).fill(0),
    [T('人声')]: [0, 0, 0, 0, 0, -1, -2, -2, -1, 0, 1, 2, 3, 3, 2, 1, 1, 1, 2, 2, 1, 0, -1, -1, 0, 0, 0, 0, 0, 0, 0],
    [T('音乐')]: [3, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 0],
    [T('低音')]: [6, 6, 5, 4, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [T('高音')]: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6],
    [T('播客')]: [-6, -5, -4, -2, 0, 1, 2, 3, 3, 3, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  }

  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v))

  function defaults() {
    return { bands: new Array(BANDS).fill(0), low_cut_hz: 80, high_cut_hz: 16000, low_cut_on: false, high_cut_on: false }
  }

  function readSettings(row) {
    let s = null
    try {
      s = JSON.parse(row.params?.settings || '{}')
    } catch (_) {
      s = null
    }
    const d = defaults()
    if (!s || typeof s !== 'object') return d
    const bands = Array.isArray(s.bands) ? s.bands.slice(0, BANDS) : []
    while (bands.length < BANDS) bands.push(0)
    return {
      bands: bands.map((v) => clamp(Number(v) || 0, -DB_RANGE, DB_RANGE)),
      low_cut_hz: clamp(Number(s.low_cut_hz) || d.low_cut_hz, 20, 500),
      high_cut_hz: clamp(Number(s.high_cut_hz) || d.high_cut_hz, 1000, 24000),
      low_cut_on: !!s.low_cut_on,
      high_cut_on: !!s.high_cut_on,
    }
  }

  const json = (s) => JSON.stringify(s)
  function customSlots() {
    try {
      const a = JSON.parse(localStorage.getItem(STORE) || '[]')
      return Array.isArray(a) ? a.slice(0, CUSTOM_SLOTS) : []
    } catch (_) {
      return []
    }
  }
  function saveSlots(a) {
    try {
      localStorage.setItem(STORE, JSON.stringify(a.slice(0, CUSTOM_SLOTS)))
    } catch (_) {}
  }

  const xOf = (f, w) => (Math.log(clamp(f, 20, 20000) / 20) / Math.log(20000 / 20)) * w

  /**
   * @param {HTMLElement} elRow @param {number} ci @param {number} ri @param {any} row
   * @param {{apply?: () => void, persist?: () => void}} [hooks]
   */
  function attach(elRow, ci, ri, row, hooks) {
    void hooks
    const box = document.createElement('div')
    box.className = 'eqcard'

    const curve = /** @type {HTMLCanvasElement} */ (document.createElement('canvas'))
    curve.className = 'eq-canvas'
    curve.title = T('在曲线上拖动：左右选频段，上下改增益')
    box.appendChild(curve)

    // 低切 / 高切（独立开关；频响画进曲线）
    const cuts = document.createElement('div')
    cuts.className = 'eq-cuts'
    const lowOn = document.createElement('input'); lowOn.type = 'checkbox'
    const lowLab = document.createElement('label'); lowLab.className = 'en'
    lowLab.append(lowOn, document.createTextNode(T('低切')))
    const lowHz = document.createElement('input'); lowHz.type = 'range'; lowHz.min = '20'; lowHz.max = '500'; lowHz.step = '10'
    const lowVal = document.createElement('span'); lowVal.className = 'mono eq-cut-val'
    const highOn = document.createElement('input'); highOn.type = 'checkbox'
    const highLab = document.createElement('label'); highLab.className = 'en'
    highLab.append(highOn, document.createTextNode(T('高切')))
    const highHz = document.createElement('input'); highHz.type = 'range'; highHz.min = '1000'; highHz.max = '24000'; highHz.step = '100'
    const highVal = document.createElement('span'); highVal.className = 'mono eq-cut-val'
    // **两行三列**（开关 / 滑块 / 数值）上下对齐；以前四个控件挤一行，16k 会被挤到下一行
    const lowRow = document.createElement('div'); lowRow.className = 'eq-cut-row'
    lowRow.append(lowLab, lowHz, lowVal)
    const highRow = document.createElement('div'); highRow.className = 'eq-cut-row'
    highRow.append(highLab, highHz, highVal)
    cuts.append(lowRow, highRow)
    box.appendChild(cuts)

    const presets = document.createElement('div')
    presets.className = 'eq-presets'
    box.appendChild(presets)

    let settings = readSettings(row)
    let selSlot = -1
    let curvePending = false
    let sendTimer = 0
    /** @type {{hz:number[], db:number[]}|null} */
    let response = null

    const applyMode = () => {
      lowOn.checked = settings.low_cut_on
      lowHz.value = String(settings.low_cut_hz)
      lowVal.textContent = settings.low_cut_hz + ' Hz'
      highOn.checked = settings.high_cut_on
      highHz.value = String(settings.high_cut_hz)
      highVal.textContent = settings.high_cut_hz >= 1000 ? settings.high_cut_hz / 1000 + 'k' : settings.high_cut_hz + ' Hz'
      lowHz.disabled = !settings.low_cut_on
      highHz.disabled = !settings.high_cut_on
    }

    /** 节流下发（热参数，不重建会话）；抬手时立即补一次 */
    const push = (immediate) => {
      window.clearTimeout(sendTimer)
      const send = () => {
        row.params.settings = json(settings)
        call('set_row_param', { col: ci, row: ri, key: 'settings', value: row.params.settings }).catch((e) => {
          window.__pvToast?.(T('EQ 下发失败：') + e, 'error', 6000)
        })
      }
      if (immediate) send()
      else sendTimer = window.setTimeout(send, 60)
    }

    /** 曲线：`eq_curve` 用当前 settings 算（界面不自己算频响） */
    const refreshCurve = (immediate) => {
      if (curvePending) return
      curvePending = true
      window.setTimeout(async () => {
        curvePending = false
        try {
          const r = await call('eq_curve', { settings: json(settings), points: 320 })
          response = /** @type {any} */ (r)
          draw()
        } catch (e) {
          window.__pvDebug?.('EQ 曲线失败：' + e)
        }
      }, immediate ? 0 : 50)
    }

    function draw() {
      const dpr = window.devicePixelRatio || 1
      const w = Math.max(60, curve.clientWidth || 360)
      const h = Math.max(60, curve.clientHeight || 150)
      const W = Math.round(w * dpr)
      const H = Math.round(h * dpr)
      if (curve.width !== W || curve.height !== H) {
        curve.width = W
        curve.height = H
      }
      const g = curve.getContext('2d')
      g.setTransform(dpr, 0, 0, dpr, 0, 0)
      g.clearRect(0, 0, w, h)
      const css = (k, d) => getComputedStyle(document.documentElement).getPropertyValue(k).trim() || d
      const line = css('--line', '#232b36')
      const muted = css('--muted', '#8a94a6')
      const accent = css('--accent', '#4f8cff')
      const padB = 14
      const ph = h - padB
      const yOf = (db) => ph / 2 - (db / DB_RANGE) * (ph / 2)

      // 网格 + 频率刻度（对数轴 20 Hz ~ 20 kHz）
      g.font = '9px Consolas, monospace'
      g.textBaseline = 'top'
      g.strokeStyle = line
      g.fillStyle = muted
      for (const db of [24, 12, 0, -12, -24]) {
        const y = yOf(db)
        g.globalAlpha = db === 0 ? 0.9 : 0.5
        g.beginPath(); g.moveTo(0, y); g.lineTo(w, y); g.stroke()
        g.globalAlpha = 1
      }
      for (const f of [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]) {
        const x = xOf(f, w)
        g.globalAlpha = 0.18
        g.beginPath(); g.moveTo(x, 0); g.lineTo(x, ph); g.stroke()
        g.globalAlpha = 1
        g.textAlign = x < 12 ? 'left' : x > w - 12 ? 'right' : 'center'
        g.fillText(f >= 1000 ? f / 1000 + 'k' : String(f), x, ph + 3)
      }

      // 31 根柱子（用户喜欢的那版）：从 0 dB 线长到当前增益
      const step = w / BANDS
      for (let i = 0; i < BANDS; i++) {
        const x = i * step
        const bw = Math.max(1, step - 2)
        const db = settings.bands[i]
        const y = yOf(db)
        // 柱子按方向上色：**正增益红、负增益绿**（曲线仍是主题蓝，两件事能分开看）
        g.fillStyle = db >= 0 ? '#e5484d' : '#30a46c'
        g.globalAlpha = 0.75
        g.fillRect(x + 1, Math.min(y, ph / 2), bw, Math.abs(ph / 2 - y))
        g.globalAlpha = 1
      }

      // 曲线（含低切/高切）
      if (response && response.hz && response.db) {
        g.strokeStyle = accent
        g.lineWidth = 2
        g.lineJoin = 'round'
        g.beginPath()
        response.hz.forEach((f, i) => {
          const x = xOf(f, w)
          const y = yOf(clamp(response.db[i], -DB_RANGE, DB_RANGE))
          i ? g.lineTo(x, y) : g.moveTo(x, y)
        })
        g.stroke()
        g.lineWidth = 1
      }
    }

    // 拖动：左右选最近的频段，上下改增益
    let dragging = -1
    const at = (ev) => {
      const r = curve.getBoundingClientRect()
      const x = ev.clientX - r.left
      const y = ev.clientY - r.top
      const padB = 14
      const ph = r.height - padB
      const db = clamp(((ph / 2 - y) / (ph / 2)) * DB_RANGE, -DB_RANGE, DB_RANGE)
      const idx = clamp(Math.round((x / r.width) * (BANDS - 1)), 0, BANDS - 1)
      return { idx, db }
    }
    curve.addEventListener('pointerdown', (ev) => {
      curve.setPointerCapture(ev.pointerId)
      const { idx, db } = at(ev)
      dragging = idx
      settings.bands[idx] = Math.round(db * 2) / 2
      draw()
      refreshCurve(false)
      push(false)
    })
    curve.addEventListener('pointermove', (ev) => {
      if (dragging < 0) return
      const { db } = at(ev)
      settings.bands[dragging] = Math.round(db * 2) / 2
      draw()
      refreshCurve(false)
      push(false)
    })
    curve.addEventListener('pointerup', (ev) => {
      if (dragging >= 0) {
        curve.releasePointerCapture?.(ev.pointerId)
        dragging = -1
        push(true)
        refreshCurve(true)
      }
    })

    lowOn.addEventListener('change', () => {
      settings.low_cut_on = lowOn.checked
      applyMode()
      push(true)
      refreshCurve(true)
    })
    highOn.addEventListener('change', () => {
      settings.high_cut_on = highOn.checked
      applyMode()
      push(true)
      refreshCurve(true)
    })
    lowHz.addEventListener('input', () => {
      settings.low_cut_hz = Number(lowHz.value)
      applyMode()
      refreshCurve(false)
      push(false)
    })
    highHz.addEventListener('input', () => {
      settings.high_cut_hz = Number(highHz.value)
      applyMode()
      refreshCurve(false)
      push(false)
    })

    const applySettings = (s2) => {
      settings = {
        bands: s2.bands.slice(0, BANDS),
        low_cut_hz: s2.low_cut_hz ?? settings.low_cut_hz,
        high_cut_hz: s2.high_cut_hz ?? settings.high_cut_hz,
        low_cut_on: s2.low_cut_on ?? settings.low_cut_on,
        high_cut_on: s2.high_cut_on ?? settings.high_cut_on,
      }
      while (settings.bands.length < BANDS) settings.bands.push(0)
      applyMode()
      push(true)
      refreshCurve(true)
      draw()
    }

    /** 预设：上排 6 个内置、下排 6 个自定义槽；网格两行六列，左右对齐 */
    const paintPresets = () => {
      presets.replaceChildren()
      const add = (label, apply, cls) => {
        const b = document.createElement('button')
        b.type = 'button'
        b.className = 'eq-preset' + (cls || '')
        b.textContent = label
        b.addEventListener('click', apply)
        presets.appendChild(b)
        return b
      }
      for (const [name, bands] of Object.entries(BUILTIN)) {
        add(name, () => {
          selSlot = -1
          applySettings({ bands })
          paintPresets()
        })
      }
      const slots = customSlots()
      for (let i = 0; i < CUSTOM_SLOTS; i++) {
        const slot = slots[i] || null
        const b = add(T('预设 {n}', { n: i + 1 }), () => {
          if (!slot) {
            window.__pvToast?.(T('长按这个按钮保存当前设置'), 'info', 4000)
            return
          }
          selSlot = i
          applySettings(slot)
          paintPresets()
        }, slot ? (selSlot === i ? ' sel' : '') : ' empty')
        b.title = slot ? T('单击应用；长按覆盖保存') : T('长按保存当前设置')
        let timer = 0
        let long = false
        b.addEventListener('pointerdown', () => {
          long = false
          timer = window.setTimeout(() => {
            long = true
            const a = customSlots()
            while (a.length < CUSTOM_SLOTS) a.push(null)
            a[i] = JSON.parse(json(settings))
            saveSlots(a)
            selSlot = i
            paintPresets()
            window.__pvToast?.(T('已保存到自定义 {n}', { n: i + 1 }), 'ok', 3000)
          }, 600)
        })
        const cancel = () => window.clearTimeout(timer)
        b.addEventListener('pointerup', cancel)
        b.addEventListener('pointerleave', cancel)
        b.addEventListener('click', (ev) => {
          if (long) {
            ev.preventDefault()
            ev.stopPropagation()
            long = false
          }
        }, true)
      }
    }
    paintPresets()

    applyMode()
    elRow.appendChild(box)
    window.requestAnimationFrame(() => {
      draw()
      refreshCurve(true)
    })
    return true
  }

  window.__pvEq = { attach }
})()
