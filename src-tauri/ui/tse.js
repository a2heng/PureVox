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

// TSE 参考音频卡片（ARCHITECTURE.md §7）。
//
// 行为对齐 legacy `uitk/dialogs.py::open_tse_dialog`：
// - 参考语音固定为 `~/.purevox/tse_reference.wav`，界面**不给路径输入**（用户 2026-10-26
//   明确要求），只显示录制时间 + 一个「打开参考语音文件夹」入口；
// - 「开始录音」录的是**降噪后、TSE 前**的信号（后端 `recorder.rs` 的 Pre 抽头）；
// - 录音必须引擎在跑；录满时间内没收到音频由后端看门狗明确失败，不永远停在「录制中」；
// - 完成 / 失败走**右下角气泡**（与其它操作同一套 `__pvToast`），卡片里只留持久状态；
// - 完成后由 `columns.js` 的全局逻辑重建会话加载新参考；卡片随后刷新参考信息。
//
// 本文件独占 TSE 卡片控件；`columns.js` 只挂载点（与 media.js 同一纪律）。

;(() => {
  const { invoke } = window.__TAURI__.core
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  /**
   * @param {string} text
   * @param {'info'|'ok'|'warn'|'error'} [level]
   * @param {number} [ms]
   */
  const toast = (text, level = 'info', ms) => window.__pvToast?.(text, level, ms)

  /** @type {Set<any>} */
  const cards = new Set()

  /**
   * 录制时长由后端给（`tse_reference_info.record_seconds` = Rust 的 `REF_RECORD_SECONDS`），
   * 界面不另写一份（现役 TSE 注册 1001 帧 = 10 s，录多少喂多少）。
   * info 还没回来时先用 10 兜底（与后端常量同值）。
   */
  const FALLBACK_RECORD_SECONDS = 10

  /** 录制时间按固定格式显示（`YYYY/MM/DD HH:mm`）：不随系统 locale 变，行宽也稳定。 */
  function fmtTime(ms) {
    if (!ms) return '—'
    const d = new Date(ms)
    if (Number.isNaN(d.getTime())) return '—'
    const p = (n) => String(n).padStart(2, '0')
    return `${d.getFullYear()}/${p(d.getMonth() + 1)}/${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
  }

  /** 参考信息一行：只有录制时间（时长恒为 `REF_SECONDS`、文件名固定，都不占地方）。 */
  function setInfo(card, text, cls = '') {
    if (!card.info) return
    card.info.textContent = text
    card.info.className = 'tse-info mono ' + cls
  }

  /** 介绍/按钮上的时长跟后端走（`record_seconds`）；info 没回来时用兜底值。 */
  function applyTexts(card) {
    const sec = card.recordSeconds ?? FALLBACK_RECORD_SECONDS
    // 介绍只讲 TSE 是干什么的；「录的是什么信号」属于实现细节，不再上屏。
    card.intro.textContent = T(
      'TSE 介绍：只保留目标说话人的声音。先录 {sec} 秒自己的说话声作为参考，之后按这个声音过滤其他人；录制前请先启动音频处理，并对着麦克风持续说话。',
      { sec },
    )
    card.btn.textContent = T('● 开始录音 ({sec}s)', { sec })
  }

  async function refresh(card) {
    try {
      const info =
        /** @type {{path:string,exists:boolean,size:number,modified_ms:number,duration_s:number|null,record_seconds:number}} */ (
          await invoke('tse_reference_info')
        )
      if (!cards.has(card)) return
      if (info && typeof info.record_seconds === 'number') card.recordSeconds = info.record_seconds
      applyTexts(card)

      if (info && info.exists) {
        setInfo(card, T('参考语音：录制于 {time}', { time: fmtTime(info.modified_ms) }), 'ok')
      } else {
        setInfo(card, T('尚无参考语音（TSE 直通）'), 'na')
      }
    } catch (e) {
      if (cards.has(card)) setInfo(card, T('参考状态读取失败：') + e, 'na')
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
      /** 录制时长（秒）：首次刷新后由后端填，之前用兜底值 */
      recordSeconds: FALLBACK_RECORD_SECONDS,
      info: document.createElement('div'),
      intro: document.createElement('div'),
      btn: document.createElement('button'),
      folder: document.createElement('button'),
      progress: document.createElement('span'),
    }
    card.info.className = 'tse-info mono'
    card.info.textContent = T('读取参考状态…')

    const intro = card.intro
    intro.className = 'note tse-intro'

    const ctl = document.createElement('div')
    ctl.className = 'tse-controls'
    card.btn.type = 'button'
    card.btn.className = 'primary'
    applyTexts(card)
    card.btn.addEventListener('click', async () => {
      if (card.recording) return
      if (ctx.running && !ctx.running()) {
        toast(T('请先启动音频处理，再录制参考。'), 'warn', 6000)
        return
      }
      card.recording = true
      // 只有先见到本次的「录制中」快照，才认后面的「完成」：点下去的瞬间轮询可能
      // 还带着上一轮的完成快照（那时会立刻弹「已录制」并把卡片复位）
      card.awaiting = true
      card.btn.disabled = true
      card.progress.textContent = T('录音中… {sec}s（请持续说话）', { sec: card.recordSeconds })
      try {
        await invoke('record_tse_reference')
      } catch (e) {
        card.recording = false
        card.btn.disabled = false
        card.progress.textContent = ''
        toast(T('录制失败：') + e, 'error', 7000)
        void refresh(card)
      }
    })

    card.folder.type = 'button'
    card.folder.textContent = T('打开参考语音文件夹')
    card.folder.addEventListener('click', () => {
      card.folder.disabled = true
      invoke('tse_open_reference_folder')
        .then(() => toast(T('已打开参考语音文件夹'), 'ok', 2500))
        .catch((e) => toast(T('打开参考语音文件夹失败：') + e, 'error', 6000))
        .finally(() => {
          card.folder.disabled = false
        })
    })

    card.progress.className = 'tse-progress mono'
    ctl.append(card.btn, card.folder, card.progress)
    const box = document.createElement('div')
    box.className = 'tse-card'
    box.append(intro, card.info, ctl)
    el.appendChild(box)
    cards.add(card)
    void refresh(card)
    return true
  }

  /** 2 Hz 快照：只更新正在录音的卡片（不自己另算状态，DEBUG.md §1.3）。 */
  function onSnapshot(s) {
    const rec = s && s.recorder
    if (!rec) return
    for (const card of cards) {
      if (!card.recording) continue
      if (rec.state === 'ok') {
        const tpl = window.__pvTpl ? window.__pvTpl(rec.value) : null
        if (card.awaiting) {
          // 上一轮的残留（完成/失败）先丢掉；见到「录制中」才算这次真正开始
          if (!tpl || tpl.id !== 'rec.progress') continue
          card.awaiting = false
        }
        card.progress.textContent = tpl ? tpl.text : rec.value
        if (tpl && tpl.id === 'rec.done') {
          card.recording = false
          card.btn.disabled = false
          card.progress.textContent = ''
          toast(T('参考语音已录制，TSE 已生效'), 'ok')
          // 会话重建由 `columns.js` 在同一个快照里触发；这里只刷新卡片自己的信息行。
          setTimeout(() => {
            if (cards.has(card)) void refresh(card)
          }, 800)
        }
      } else if (rec.state === 'unavailable') {
        card.recording = false
        card.btn.disabled = false
        card.progress.textContent = ''
        toast(T('录制失败：') + rec.reason, 'error', 7000)
        void refresh(card)
      }
    }
  }

  function reset() {
    cards.clear()
  }

  window.__pvTse = { attach, onSnapshot, reset }
})()
