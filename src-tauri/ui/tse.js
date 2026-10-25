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

// TSE 参考音频卡片（DESIGN.md §7）。
//
// 行为对齐 legacy `uitk/dialogs.py::open_tse_dialog`：
// - 显示当前参考（文件名 / 大小 / 修改时间 / 时长）；没有参考时明确写「TSE 将直通」；
// - 「开始录音 (10s)」录的是**降噪后、TSE 前**的信号（后端 `recorder.rs` 的 Pre 抽头）；
// - 录音必须引擎在跑；10 秒内没收到音频由后端看门狗明确失败，不永远停在「录制中」；
// - 完成后由 `columns.js` 的全局逻辑重建会话加载新参考；卡片随后刷新文件状态。
//
// 本文件独占 TSE 卡片控件；`columns.js` 只放「参考路径输入框」与挂载点（与 media.js 同一纪律）。

;(() => {
  const { invoke } = window.__TAURI__.core
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)

  /** @type {Set<any>} */
  const cards = new Set()

  function baseName(p) {
    if (!p) return ''
    const parts = String(p).split(/[\\/]/)
    return parts[parts.length - 1] || p
  }

  function fmtSize(n) {
    if (!n) return '0 KB'
    if (n >= 1024 * 1024) return (n / (1024 * 1024)).toFixed(1) + ' MB'
    return Math.max(1, Math.round(n / 1024)) + ' KB'
  }

  function fmtTime(ms) {
    if (!ms) return '—'
    try {
      return new Date(ms).toLocaleString()
    } catch {
      return '—'
    }
  }

  function setStatus(card, text, cls = '') {
    if (!card.status) return
    card.status.textContent = text
    card.status.className = 'tse-status mono ' + cls
  }

  async function refresh(card) {
    try {
      const reference = (card.row.params && card.row.params.reference) || null
      const info =
        /** @type {{path:string,exists:boolean,size:number,modified_ms:number,duration_s:number|null,is_default:boolean}} */ (
          await invoke('tse_reference_info', { reference })
        )
      if (!cards.has(card)) return
      if (info && info.exists) {
        setStatus(
          card,
          T('已有参考：{name}（{size} · {time} · {dur}）', {
            name: baseName(info.path),
            size: fmtSize(info.size),
            time: fmtTime(info.modified_ms),
            dur: info.duration_s ? info.duration_s.toFixed(1) + ' s' : '—',
          }),
          'ok',
        )
      } else {
        setStatus(card, T('尚无参考音频——TSE 将直通。'), 'na')
      }
    } catch (e) {
      if (cards.has(card)) setStatus(card, T('参考状态读取失败：') + e, 'na')
    }
  }

  /**
   * 把 TSE 卡片挂到 `el` 上。`ctx.running()` 取当前引擎运行状态。
   * @returns {boolean} 是否挂载成功
   */
  function attach(el, ci, ri, row, ctx) {
    if (!el) return false
    const card = {
      row,
      ctx,
      recording: false,
      status: document.createElement('div'),
      btn: document.createElement('button'),
      progress: document.createElement('span'),
    }
    card.status.className = 'tse-status mono'
    card.status.textContent = T('读取参考状态…')

    const note = document.createElement('div')
    note.className = 'note'
    note.textContent = T(
      '录的是「降噪后、TSE 前」的 10 秒信号，音量归一化到 -20 dBFS。请先启动音频处理，并对麦克风持续说话。',
    )

    const ctl = document.createElement('div')
    ctl.className = 'tse-controls'
    card.btn.type = 'button'
    card.btn.textContent = T('● 开始录音 (10s)')
    card.progress.className = 'tse-progress mono'
    card.btn.addEventListener('click', async () => {
      if (card.recording) return
      if (ctx.running && !ctx.running()) {
        setStatus(card, T('请先启动音频处理，再录制参考。'), 'na')
        return
      }
      card.recording = true
      card.btn.disabled = true
      card.progress.textContent = ''
      setStatus(card, T('录音中… {sec}s（请持续说话）', { sec: 10 }), 'ok')
      try {
        await invoke('record_tse_reference', { seconds: 10 })
      } catch (e) {
        card.recording = false
        card.btn.disabled = false
        setStatus(card, T('录制失败：') + e, 'na')
      }
    })

    ctl.append(card.btn, card.progress)
    const box = document.createElement('div')
    box.className = 'tse-card'
    box.append(card.status, note, ctl)
    el.appendChild(box)
    cards.add(card)
    void refresh(card)
    return true
  }

  /** 2 Hz 快照：只更新正在录音的卡片（不自己另算状态，AGENTS §1.3）。 */
  function onSnapshot(s) {
    const rec = s && s.recorder
    if (!rec) return
    for (const card of cards) {
      if (!card.recording) continue
      if (rec.state === 'ok') {
        const tpl = window.__pvTpl ? window.__pvTpl(rec.value) : null
        card.progress.textContent = tpl ? tpl.text : rec.value
        if (tpl && tpl.id === 'rec.done') {
          card.recording = false
          card.btn.disabled = false
          setStatus(card, T('完成！参考已生效。'), 'ok')
          setTimeout(() => {
            if (cards.has(card)) void refresh(card)
          }, 1500)
        }
      } else if (rec.state === 'unavailable') {
        card.recording = false
        card.btn.disabled = false
        setStatus(card, T('录制失败：') + rec.reason, 'na')
        void refresh(card)
      }
    }
  }

  function reset() {
    cards.clear()
  }

  window.__pvTse = { attach, onSnapshot, reset }
})()
