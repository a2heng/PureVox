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

// AEC 校准结果面板（AGENTS.md §1.4）：**只渲染 Rust 的 `calib_report`**。
//
// 唯一数据源是快照里的 `calib_report`（= `GET /debug/calib` = `~/.purevox/calib_last.json`）；
// 本文件不重算峰值、不算延时、不改增益。曲线是报告里的平滑带内能量包络（不是原始波形），
// 峰值时刻直接用报告里的 `mic_peak_ms` / `far_peak_ms`。
;(() => {
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  /** @type {(id: string) => HTMLElement | null} */
  const $ = (id) => document.getElementById(id)

  /** 最近一份报告（复制 JSON / 重绘用） @type {any} */
  let last = null
  let shownMs = 0

  /** 数字格式化：null / 非有限值一律显示破折号，不伪造 0（§1.1）。 */
  const num = (v, digits = 1) =>
    typeof v === 'number' && Number.isFinite(v) ? v.toFixed(digits) : '—'

  /** 稳定判定码 → 本地化结论（Rust 的 verdict_text 只作为诊断行原样显示）。 */
  function verdictText(r) {
    switch (r.verdict_code) {
      case 'ok':
        return T('成功：峰值匹配完成，延时与增益已应用。')
      case 'mic_not_heard':
        return T('失败：麦克风没有听到探针。')
      case 'far_unavailable':
        return T('失败：远端参考没有数据。')
      case 'too_short':
        return T('失败：采集数据太短。')
      default:
        return r.ok ? T('成功') : T('失败')
    }
  }

  function setText(id, text, cls) {
    const el = $(id)
    if (!el) return
    el.textContent = text
    if (cls !== undefined) el.className = cls
  }

  /** 画曲线：两条带内能量包络 + 两个峰值 + 延时标注。 */
  function draw(r) {
    const cv = /** @type {HTMLCanvasElement | null} */ ($('calib-canvas'))
    if (!cv) return
    const cssW = cv.clientWidth || 720
    const cssH = 260
    const dpr = window.devicePixelRatio || 1
    cv.width = Math.max(1, Math.round(cssW * dpr))
    cv.height = Math.round(cssH * dpr)
    const ctx = cv.getContext('2d')
    if (!ctx) return
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, cssW, cssH)

    const pad = { l: 48, r: 14, t: 16, b: 28 }
    const w = Math.max(10, cssW - pad.l - pad.r)
    const h = Math.max(10, cssH - pad.t - pad.b)
    const s = r && r.series
    if (!s || !Array.isArray(s.t_ms) || s.t_ms.length < 2) {
      ctx.fillStyle = '#8a94a6'
      ctx.font = '12px system-ui, sans-serif'
      ctx.fillText(T('没有曲线数据（采集超时或数据太短）'), pad.l, pad.t + 24)
      return
    }

    const t0 = s.t_ms[0]
    const t1 = s.t_ms[s.t_ms.length - 1]
    // 画**原始**平滑包络（不叠加校准增益、不做任何换算）：用户要看的是「这次听到了什么」，
    // 不是把结果再算一遍。目标线只表示两端最终要归一化到的同一条电平。
    const micEnv = Array.isArray(s.mic_env_db) ? s.mic_env_db : []
    const farEnv = Array.isArray(s.far_env_db) ? s.far_env_db : []
    let lo = Infinity
    let hi = -Infinity
    for (const arr of [micEnv, farEnv]) {
      for (const v of arr) {
        if (typeof v === 'number' && Number.isFinite(v)) {
          lo = Math.min(lo, v)
          hi = Math.max(hi, v)
        }
      }
    }
    if (!Number.isFinite(lo) || !Number.isFinite(hi)) {
      lo = -120
      hi = 0
    }
    // 只保留峰值附近 90 dB：远端的数字静音是 -120，不夹一下会把两条曲线压成一条直线。
    lo = Math.max(lo, hi - 90)
    lo = Math.floor((lo - 4) / 10) * 10
    hi = Math.ceil((hi + 4) / 10) * 10
    if (hi - lo < 20) hi = lo + 20

    const x = (t) => pad.l + ((t - t0) / Math.max(1e-9, t1 - t0)) * w
    const y = (db) => pad.t + (1 - (db - lo) / Math.max(1e-9, hi - lo)) * h

    // 网格 + 刻度（左 dBFS、下秒）
    ctx.lineWidth = 1
    ctx.strokeStyle = 'rgba(138, 148, 166, 0.22)'
    ctx.fillStyle = '#8a94a6'
    ctx.font = '11px ui-monospace, monospace'
    for (let i = 0; i <= 5; i++) {
      const db = lo + ((hi - lo) * i) / 5
      const yy = y(db)
      ctx.beginPath()
      ctx.moveTo(pad.l, yy)
      ctx.lineTo(pad.l + w, yy)
      ctx.stroke()
      ctx.fillText(db.toFixed(0), 8, yy + 4)
    }
    for (let i = 0; i <= 6; i++) {
      const tt = t0 + ((t1 - t0) * i) / 6
      const xx = x(tt)
      ctx.beginPath()
      ctx.moveTo(xx, pad.t)
      ctx.lineTo(xx, pad.t + h)
      ctx.stroke()
      ctx.fillText((tt / 1000).toFixed(2) + 's', Math.min(xx, pad.l + w - 30) - 10, cssH - 8)
    }

    // 两条横线：far 峰值电平 / mic 峰值电平（原始带内峰值，不再做任何换算）
    const hline = (db, color, label) => {
      if (typeof db !== 'number' || !Number.isFinite(db)) return
      const yy = Math.min(Math.max(y(db), pad.t), pad.t + h)
      ctx.save()
      ctx.setLineDash([6, 4])
      ctx.strokeStyle = color
      ctx.lineWidth = 1.2
      ctx.beginPath()
      ctx.moveTo(pad.l, yy)
      ctx.lineTo(pad.l + w, yy)
      ctx.stroke()
      ctx.restore()
      ctx.fillStyle = color
      ctx.font = '11px ui-monospace, monospace'
      const tw = ctx.measureText(label).width
      ctx.fillText(label, pad.l + w - tw - 4, Math.max(yy - 4, pad.t + 10))
    }
    hline(
      r.far_band_rms_db,
      '#f0a13c',
      T('远端峰值 {db} dBFS', { db: num(r.far_band_rms_db) }),
    )
    hline(
      r.mic_band_rms_db,
      '#4aa3ff',
      T('麦克风峰值 {db} dBFS', { db: num(r.mic_band_rms_db) }),
    )

    // 曲线（far 在下面先画，mic 盖上去）
    const path = (arr, color) => {
      if (!Array.isArray(arr)) return
      ctx.beginPath()
      let started = false
      for (let i = 0; i < s.t_ms.length; i++) {
        const v = arr[i]
        if (typeof v !== 'number' || !Number.isFinite(v)) continue
        const xx = x(s.t_ms[i])
        const yy = y(v)
        if (!started) {
          ctx.moveTo(xx, yy)
          started = true
        } else ctx.lineTo(xx, yy)
      }
      ctx.strokeStyle = color
      ctx.lineWidth = 2
      ctx.stroke()
    }
    path(farEnv, '#f0a13c')
    path(micEnv, '#4aa3ff')

    // 峰值竖线 + 标签（位置来自报告，不从曲线上再找一遍）
    const peak = (ms, color, labelLines, labelY) => {
      if (typeof ms !== 'number' || !Number.isFinite(ms)) return
      const xx = Math.min(Math.max(x(ms), pad.l), pad.l + w)
      ctx.save()
      ctx.setLineDash([4, 4])
      ctx.strokeStyle = color
      ctx.beginPath()
      ctx.moveTo(xx, pad.t)
      ctx.lineTo(xx, pad.t + h)
      ctx.stroke()
      ctx.restore()
      ctx.fillStyle = color
      ctx.font = '11px ui-monospace, monospace'
      labelLines.forEach((line, i) => {
        const tw = ctx.measureText(line).width
        const tx = Math.min(Math.max(xx + 4, pad.l), pad.l + w - tw)
        ctx.fillText(line, tx, labelY + i * 13)
      })
    }
    const micPeak = typeof r.mic_peak_ms === 'number' ? T('麦克风峰值 {t} ms', { t: num(r.mic_peak_ms) }) : ''
    const farPeak = typeof r.far_peak_ms === 'number' ? T('远端峰值 {t} ms', { t: num(r.far_peak_ms) }) : ''
    peak(r.far_peak_ms, '#f0a13c', [farPeak], pad.t + 12)
    peak(r.mic_peak_ms, '#4aa3ff', [micPeak], pad.t + 28)

    // 延时标注：两峰值之间画双箭头
    if (
      typeof r.far_peak_ms === 'number' &&
      typeof r.mic_peak_ms === 'number' &&
      typeof r.delay_raw_ms === 'number'
    ) {
      const x1 = x(r.far_peak_ms)
      const x2 = x(r.mic_peak_ms)
      const yy = pad.t + h - 14
      ctx.strokeStyle = '#e5e7eb'
      ctx.fillStyle = '#e5e7eb'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.moveTo(x1, yy)
      ctx.lineTo(x2, yy)
      ctx.stroke()
      const label = T('延时差 {d} ms', { d: num(r.delay_raw_ms) })
      const tw = ctx.measureText(label).width
      const tx = Math.min(Math.max((x1 + x2) / 2 - tw / 2, pad.l), pad.l + w - tw)
      // 垫一层背景：标签落在曲线尾部时仍然读得清
      ctx.fillStyle = 'rgba(8, 13, 24, 0.82)'
      ctx.fillRect(tx - 4, yy - 18, tw + 8, 15)
      ctx.fillStyle = '#e5e7eb'
      ctx.fillText(label, tx, yy - 6)
    }
  }

  function render(r) {
    last = r
    const ok = !!r.ok
    setText('calib-verdict', ok ? T('成功') : T('失败'), 'cal-verdict ' + (ok ? 'ok' : 'na'))

    setText(
      'calib-delay',
      r.delay_ms == null
        ? '—'
        : `${num(r.delay_ms)} ms ｜ MIC ${num(r.mic_peak_ms)} / FAR ${num(r.far_peak_ms)} ms ｜ ` +
            T('采集时两路峰值差 {raw} ms', { raw: num(r.delay_raw_ms) }),
    )
    setText(
      'calib-echo',
      r.echo_path_db == null ? '—' : `${num(r.echo_path_db, 0)} dB（${T('远端 → 麦克风')}）`,
    )
    setText(
      'calib-mic',
      r.mic_band_rms_db == null
        ? '—'
        : `${num(r.mic_band_rms_db)} dBFS ｜ ` +
            T('比底噪高 {snr} dB', { snr: num(r.mic_snr_db, 0) }) +
            (r.mic_gain_db == null ? '' : ' → ' + T('已应用 {g} dB', { g: num(r.mic_gain_db, 0) })),
    )
    setText(
      'calib-far',
      r.far_band_rms_db == null
        ? '—'
        : `${num(r.far_band_rms_db)} dBFS` +
            (r.far_gain_db == null ? '' : ' → ' + T('已应用 {g} dB', { g: num(r.far_gain_db, 0) })),
    )
    setText('calib-conclusion', verdictText(r))

    const diag = $('calib-diag')
    if (diag) {
      diag.textContent = r.verdict_text || ''
      diag.classList.toggle('hidden', !r.verdict_text)
    }
    const nextRow = $('calib-next-row')
    const next = $('calib-next')
    if (nextRow && next) {
      next.textContent = r.next_step || ''
      nextRow.classList.toggle('hidden', !r.next_step)
    }

    const meta = []
    meta.push('schema v' + (r.schema_version ?? '?'))
    meta.push(T('第 {c} 列第 {r} 行', { c: (r.col ?? 0) + 1, r: (r.row ?? 0) + 1 }))
    meta.push(
      T('麦克风 {mic} ｜ 远端 {far}', {
        mic: r.mic_device ?? '—',
        far: r.far_device ?? '—',
      }),
    )
    meta.push(T('探针带 {lo}–{hi} Hz；曲线 = 21 ms 帧能量的 13 帧（约 35 ms）平滑（不是原始波形）', {
      lo: r.band_hz?.[0] ?? '—',
      hi: r.band_hz?.[1] ?? '—',
    }))
    if (r.dumps?.json) meta.push(T('文件：{path}', { path: r.dumps.json }))
    setText('calib-meta', meta.join(' ｜ '), 'calib-meta mono')
  }

  function visible() {
    const p = $('calib')
    return !!p && !p.classList.contains('hidden')
  }

  function open() {
    $('calib')?.classList.remove('hidden')
    if (last) render(last)
    // 面板从 hidden 变可见后 clientWidth 才有值：下一帧再画
    requestAnimationFrame(() => {
      if (last) draw(last)
    })
  }

  function close() {
    $('calib')?.classList.add('hidden')
  }

  async function copy() {
    if (!last) return
    const text = JSON.stringify(last, null, 2)
    try {
      await navigator.clipboard.writeText(text)
      window.__pvToast?.(T('已复制 JSON'), 'ok')
    } catch (e) {
      // WebView 剪贴板权限偶发不给：用临时 textarea 兜底
      const ta = document.createElement('textarea')
      ta.value = text
      document.body.appendChild(ta)
      ta.select()
      let ok = false
      try {
        ok = document.execCommand('copy')
      } catch (_) {
        /* 忽略 */
      }
      ta.remove()
      window.__pvToast?.(ok ? T('已复制 JSON') : T('复制失败：') + e, ok ? 'ok' : 'error')
    }
  }

  /**
   * 快照回调：只有**新报告**（`created_ms` 变化）才自动弹面板；
   * 同一份报告重复到达（2 Hz 轮询）不重复打扰。
   */
  function onSnapshot(probe) {
    if (!probe || probe.state !== 'ok' || !probe.value) return
    const r = probe.value
    if (!r || typeof r !== 'object') return
    const isNew = r.created_ms !== shownMs
    last = r
    if (isNew) {
      shownMs = r.created_ms
      open()
    } else if (visible()) {
      render(r)
      requestAnimationFrame(() => draw(r))
    }
  }

  window.__pvCalib = { onSnapshot, open, close, hasReport: () => !!last }

  window.addEventListener('DOMContentLoaded', () => {
    $('calib-close')?.addEventListener('click', close)
    $('calib-copy')?.addEventListener('click', copy)
    // 点遮罩关闭（和面板外区域点击一致）
    $('calib')?.addEventListener('click', (e) => {
      if (e.target === $('calib')) close()
    })
    window.addEventListener('resize', () => {
      if (visible() && last) draw(last)
    })
  })

  // 语言切换：同一份数据重画（结论、图注、静态串都要换语言）
  document.addEventListener('pv-langchange', () => {
    if (!last) return
    render(last)
    if (visible()) requestAnimationFrame(() => draw(last))
  })
})()
