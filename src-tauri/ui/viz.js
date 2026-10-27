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

// 可视化（处理行 `viz`）与调试面板的**共用绘制层**。
//
// 设计来自 legacy `uitk/viz.py`（沿用 legacy 的观感）：
// - **电平**：分段 LED 条，绿 -60..-20 / 黄 -20..-9 / 红 -9..0；快攻慢放
//   （上升瞬间到位、下降 90 dB/s），峰值保持 10 s 后按 20 dB/s 回落；
//   没有外置刻度，峰值数字就在条右端。
// - **频谱**：整列底槽 + 三色柱（-45 dB 以下绿、-45..-30 黄、-30 以上红），
//   纯频谱无网格；分段方式是**对数频率**（20 Hz ~ 20 kHz），与 legacy 的 Mel 64 段
//   同一个视觉意图。
//
// 与 legacy 的差别（有意为之）：数据来自我们的 960 点 FFT，64 段映射在 **Rust**
// （`dsp::bands`）里算好（插件、`/debug/audio`、`/debug/viz` 同一份），本文件只画；
// 弹道与 EMA 平滑属于「怎么画」，放在这里。
//
// 两个入口：
// - `window.__pvViz.attach(...)`：主窗口里处理行 `viz` 的卡片（10 Hz 轮询 `viz_snapshot`）；
// - `window.__pvViz.vu(canvas, db)` / `.spectrum(canvas, bands)`：调试面板逐流复用同一份绘制。
;(function () {
  'use strict'

  /** @param {string} s @param {Record<string, string|number>} [vars] */
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  const invoke = (cmd, args) => window.__TAURI__?.core?.invoke(cmd, args)

  // ---- 电平（VU）参数：与 legacy `uitk/viz.py` 同一套 ----
  const VU_MIN = -60
  const VU_MAX = 0
  const VU_RANGE = VU_MAX - VU_MIN
  const VU_Z1 = -20 // 绿区上界
  const VU_Z2 = -9 // 黄区上界
  const VU_FALL_DB_S = 90
  const VU_PEAK_HOLD_MS = 10000
  const VU_PEAK_FALL_DB_S = 20

  // ---- 频谱参数 ----
  const SPEC_BANDS = 61
  const SPEC_LO = 20
  const SPEC_HI = 20480
  const SPEC_PER_OCT = 6
  const SPEC_MIN = -90
  const SPEC_MAX = 0
  const SPEC_TICKS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]
  const SPEC_Z1 = -45
  const SPEC_Z2 = -30
  /** 平滑时间常数（ms）：10 Hz 与 2 Hz 下观感一致，不靠固定 α 猜帧率。 */
  const SPEC_TAU_MS = 90

  const C_GREEN = '#4caf50'
  const C_YELLOW = '#ffb300'
  const C_RED = '#e53935'
  const C_PEAK = '#d7b98c'

  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v))
  const dbR = (db) => clamp((db - VU_MIN) / VU_RANGE, 0, 1)
  const specR = (db) => clamp((db - SPEC_MIN) / (SPEC_MAX - SPEC_MIN), 0, 1)

  /** 按 DPR 调整画布并返回 CSS 像素坐标系。 */
  function fit(cv) {
    const dpr = window.devicePixelRatio || 1
    const w = Math.max(1, cv.clientWidth || 300)
    const h = Math.max(1, cv.clientHeight || 22)
    const W = Math.round(w * dpr)
    const H = Math.round(h * dpr)
    if (cv.width !== W || cv.height !== H) {
      cv.width = W
      cv.height = H
    }
    const g = cv.getContext('2d')
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
    g.clearRect(0, 0, w, h)
    return { g, w, h }
  }

  /** 从 CSS 变量取色（主题换了、调试面板与主窗口也是同一套）。 */
  function cssVar(name, fallback) {
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
    return v || fallback
  }

  const tint = (hex, a) => {
    const n = parseInt(hex.slice(1), 16)
    const r = (n >> 16) & 255
    const g = (n >> 8) & 255
    const b = n & 255
    return `rgba(${r},${g},${b},${a})`
  }

  // ---------------- 电平条 ----------------

  /** 每个画布一份弹道状态（同一个 canvas 可能被重建，WeakMap 自然回收）。 */
  const vuStates = new WeakMap()

  function vuState(cv) {
    let s = vuStates.get(cv)
    if (!s) {
      s = {
        db: VU_MIN,
        peak: VU_MIN,
        peakAt: 0,
        target: null,
        lastUpdate: 0,
        lastTs: 0,
        raf: 0,
      }
      vuStates.set(cv, s)
    }
    return s
  }

  function paintVu(cv, s) {
    const { g, w, h } = fit(cv)
    const pad = 2
    const mono = 'Consolas, "Cascadia Mono", monospace'
    const textW = w >= 210 ? 46 : 0
    const barW = Math.max(20, w - pad * 2 - textW)
    const scale = clamp(h / 22, 0.8, 1.6)
    const segW = Math.max(4, Math.round(6 * scale))
    const gap = Math.max(1, Math.round(2 * scale))
    const step = segW + gap
    const n = Math.max(8, Math.floor((barW + gap) / step))
    const realStep = (barW + gap) / n
    const lit = Math.round(dbR(s.db) * n)
    const peakI = Math.round(dbR(s.peak) * n) - 1
    const top = pad + 1
    const bot = h - pad - 1

    for (let i = 0; i < n; i++) {
      const x0 = pad + i * realStep
      const r = (i + 1) / n
      const zone = r < dbR(VU_Z1) ? C_GREEN : r < dbR(VU_Z2) ? C_YELLOW : C_RED
      let fill = tint(zone, 0.14)
      if (i < lit) fill = zone
      else if (i === peakI && s.peak > VU_MIN + 0.5) fill = C_PEAK
      g.fillStyle = fill
      g.fillRect(x0, top, Math.max(1, realStep - gap), Math.max(1, bot - top))
    }

    if (textW) {
      g.font = `11px ${mono}`
      g.textAlign = 'right'
      g.textBaseline = 'middle'
      g.fillStyle = cssVar('--muted', '#8a94a6')
      const text = s.peak <= VU_MIN + 0.5 ? '-∞' : s.peak.toFixed(1)
      g.fillText(text, w - pad, h / 2)
    }
  }

  function vuTick(cv) {
    const s = vuState(cv)
    if (!cv.isConnected) {
      s.raf = 0
      return
    }
    const now = performance.now()
    const dt = s.lastTs ? clamp((now - s.lastTs) / 1000, 0, 0.25) : 0
    s.lastTs = now
    // 500 ms 没有新数据就当没信号（否则会一直挂在最后一个值上，像卡住了）
    if (now - s.lastUpdate > 500) s.target = null
    if (s.target == null) {
      s.db = Math.max(VU_MIN, s.db - VU_FALL_DB_S * dt)
    } else if (s.target >= s.db) {
      s.db = s.target // 快攻：上升瞬间到位
    } else {
      s.db = Math.max(s.target, s.db - VU_FALL_DB_S * dt)
    }
    if (now - s.peakAt > VU_PEAK_HOLD_MS) {
      s.peak = Math.max(VU_MIN, s.peak - VU_PEAK_FALL_DB_S * dt)
    }
    paintVu(cv, s)
    const moving =
      s.db > VU_MIN + 0.05 ||
      s.peak > VU_MIN + 0.05 ||
      s.target != null
    s.raf = moving ? window.requestAnimationFrame(() => vuTick(cv)) : 0
  }

  /**
   * 送一帧峰值电平（dBFS）。传 `null` = 没数据（按弹道落到 -60）。
   * @param {HTMLCanvasElement} cv @param {number|null} peakDb
   */
  function vu(cv, peakDb) {
    if (!cv) return
    const s = vuState(cv)
    if (typeof peakDb === 'number' && Number.isFinite(peakDb)) {
      s.target = clamp(peakDb, VU_MIN, VU_MAX)
      s.lastUpdate = performance.now()
      if (s.target > s.peak) {
        s.peak = s.target
        s.peakAt = s.lastUpdate
      }
    } else {
      s.lastUpdate = 0
    }
    if (!s.raf) s.raf = window.requestAnimationFrame(() => vuTick(cv))
  }

  // ---------------- 频谱 ----------------

  /** 每个画布一份 EMA 状态。 */
  const specStates = new WeakMap()

  function specState(cv) {
    let s = specStates.get(cv)
    if (!s) {
      s = { smooth: new Float32Array(SPEC_BANDS), inited: false, lastTs: 0 }
      specStates.set(cv, s)
    }
    return s
  }

  /**
   * 画 64 段对数频谱（dBFS）。`bands` 来自 Rust（`spectrum_bands_db` / `viz.frames[].bands_db`）。
   * @param {HTMLCanvasElement} cv @param {number[]} bands
   */
  function spectrum(cv, bands) {
    if (!cv || !Array.isArray(bands) || bands.length !== SPEC_BANDS) return
    const s = specState(cv)
    const now = performance.now()
    const dt = s.lastTs ? clamp(now - s.lastTs, 1, 2000) : 100
    s.lastTs = now
    const a = clamp(1 - Math.exp(-dt / SPEC_TAU_MS), 0.2, 0.9)
    for (let i = 0; i < SPEC_BANDS; i++) {
      const v = Number.isFinite(bands[i]) ? bands[i] : SPEC_MIN
      s.smooth[i] = s.inited ? s.smooth[i] + a * (v - s.smooth[i]) : v
    }
    s.inited = true

    const { g, w, h } = fit(cv)
    // 对数频率轴：20 Hz ~ 20.5 kHz（ISO 1/6 倍频程 61 段）。
    // **对数轴不能从 0 开始**（log 0 = -∞），左边缘就是 20 Hz；底部标出 20/50/100…20k，
    // 让「低频挤、高频疏」这件事一眼可见，而不是一排等距柱子。
    const labelH = h >= 70 ? 13 : 0
    const barH = h - labelH
    const xOf = (f) => (Math.log2(clamp(f, SPEC_LO, SPEC_HI) / SPEC_LO) / Math.log2(SPEC_HI / SPEC_LO)) * w

    // 只留一条底线，不铺满高底槽（深色主题下满高槽位会像一排等高柱子）
    g.fillStyle = cssVar('--line', '#232b36')
    g.fillRect(0, barH - 1, w, 1)

    const step = w / SPEC_BANDS
    const gap = step >= 5 ? 1 : 0
    const zones = /** @type {Array<[number, number, string]>} */ ([
      [SPEC_MIN, SPEC_Z1, C_GREEN],
      [SPEC_Z1, SPEC_Z2, C_YELLOW],
      [SPEC_Z2, SPEC_MAX, C_RED],
    ])
    for (let i = 0; i < SPEC_BANDS; i++) {
      const x0 = (i / SPEC_BANDS) * w
      const colW = Math.max(1, step - gap)
      const db = s.smooth[i]
      if (!(db > SPEC_MIN + 0.5)) continue
      for (const [lo, hi, color] of zones) {
        const a0 = Math.max(SPEC_MIN, lo)
        const a1 = Math.min(db, hi)
        if (a1 <= a0) continue
        const y0 = barH - specR(a1) * barH
        const y1 = barH - specR(a0) * barH
        g.fillStyle = color
        g.fillRect(x0, y0, colW, Math.max(1, y1 - y0))
      }
    }

    if (labelH) {
      g.font = '9px Consolas, "Cascadia Mono", monospace'
      g.textBaseline = 'top'
      g.fillStyle = cssVar('--muted', '#8a94a6')
      for (const f of SPEC_TICKS) {
        const x = xOf(f)
        if (x < 8 || x > w - 2) continue
        g.textAlign = x < 14 ? 'left' : x > w - 14 ? 'right' : 'center'
        const label = f >= 1000 ? (f / 1000) + 'k' : String(f)
        g.fillText(label, x, barH + 3)
        g.globalAlpha = 0.18
        g.fillRect(Math.round(x), 0, 1, barH)
        g.globalAlpha = 1
      }
    }
  }

  // ---------------- 主窗口：处理行 `viz` 卡片 ----------------

  /** @type {Set<{el: HTMLElement, ci: number, ri: number, row: any, paint: (s:any)=>void}>} */
  const cards = new Set()
  /** @type {number|null} */
  let timer = null
  let lastSnap = null
  let lastError = ''

  async function poll() {
    if (!cards.size) {
      if (timer !== null) {
        window.clearInterval(timer)
        timer = null
      }
      return
    }
    try {
      lastSnap = await invoke('viz_snapshot')
      lastError = ''
    } catch (e) {
      lastError = String(e)
    }
    for (const c of cards) {
      if (!c.el.isConnected) {
        cards.delete(c)
        continue
      }
      c.paint(lastSnap)
    }
  }

  function modeOf(row) {
    const m = row.params && row.params.mode
    return m === 'vu' || m === 'spectrum' ? m : 'both'
  }

  const mk = (tag, cls, text) => {
    const e = document.createElement(tag)
    if (cls) e.className = cls
    if (text != null) e.textContent = text
    return e
  }

  /**
   * @param {HTMLElement} elRow @param {number} ci @param {number} ri @param {any} row
   * @param {{apply?: () => void, persist?: () => void}} [hooks]
   */
  function attach(elRow, ci, ri, row, hooks) {
    const box = mk('div', 'vizcard')
    const head = mk('div', 'viz-head')
    const label = mk('span', 'viz-label', T('显示内容'))
    const sel = document.createElement('select')
    sel.className = 'viz-mode'
    sel.setAttribute('aria-label', T('显示内容'))
    for (const [v, l] of /** @type {Array<[string,string]>} */ ([
      ['both', T('电平 + 频谱')],
      ['vu', T('电平（VU）')],
      ['spectrum', T('频谱')],
    ])) {
      const o = document.createElement('option')
      o.value = v
      o.textContent = l
      sel.appendChild(o)
    }
    sel.value = modeOf(row)
    head.append(label, sel)

    const body = mk('div', 'viz-body')
    const vuCv = /** @type {HTMLCanvasElement} */ (mk('canvas', 'viz-vu'))
    const specCv = /** @type {HTMLCanvasElement} */ (mk('canvas', 'viz-spec'))
    vuCv.setAttribute('aria-label', T('电平'))
    specCv.setAttribute('aria-label', T('频谱'))
    const note = mk('div', 'viz-note')
    body.append(vuCv, specCv, note)
    box.append(head, body)
    elRow.appendChild(box)

    const applyMode = () => {
      const m = modeOf(row)
      vuCv.style.display = m === 'spectrum' ? 'none' : ''
      specCv.style.display = m === 'vu' ? 'none' : ''
    }
    applyMode()

    sel.addEventListener('change', () => {
      row.params.mode = sel.value
      applyMode()
      // 显示内容只是「怎么画」，引擎两侧都算：落盘即可，**不重建会话**（DEBUG.md §1.3）
      if (hooks && typeof hooks.persist === 'function') hooks.persist()
      if (lastSnap) paint(lastSnap)
    })

    /** 按快照重画这一张卡（10 Hz；`snap` 可能为 null = 取失败） */
    const paint = (snap) => {
      const frame = snap && snap.frames ? snap.frames.find((f) => f.col === ci && f.row === ri) : null
      const enabled = row.enabled !== false
      const has = !!frame && enabled
      note.textContent = has
        ? ''
        : !enabled
          ? T('已停用')
          : lastError
            ? T('状态不可用：') + lastError
            : T('会话未运行')
      note.classList.toggle('on', !has)
      vu(vuCv, has && typeof frame.peak_dbfs === 'number' ? frame.peak_dbfs : null)
      if (has) spectrum(specCv, frame.bands_db)
      else {
        // 没数据时画一张空槽（不伪造 0 dB 的柱子）
        const st = specState(specCv)
        st.inited = false
        spectrum(specCv, new Array(SPEC_BANDS).fill(SPEC_MIN))
      }
    }
    const card = { el: elRow, ci, ri, row, paint }
    cards.add(card)
    if (timer === null) timer = window.setInterval(poll, 100)
    poll()
    return true
  }

  function reset() {
    cards.clear()
    lastSnap = null
    if (timer !== null) {
      window.clearInterval(timer)
      timer = null
    }
  }

  window.addEventListener('DOMContentLoaded', () => {
    document.addEventListener('pv-langchange', () => {
      for (const c of cards) c.paint(lastSnap)
    })
    window.addEventListener('beforeunload', reset)
  })

  window.__pvViz = { attach, reset, vu, spectrum }
})()
