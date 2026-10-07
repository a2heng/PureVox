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

// 设置面板：全局热键（开关 + 键位录制）与提示音（开关 + 启动/停止预设）。
;(() => {
  const { invoke } = window.__TAURI__.core
  const T = (s) => (window.__pvT ? window.__pvT(s) : s)
  const $ = (id) => document.getElementById(id)
  const $in = (id) => /** @type {HTMLInputElement} */ ($(id))
  const $sel = (id) => /** @type {HTMLSelectElement} */ ($(id))

  /** @type {any} */
  let settings = null
  let autostart = false

  // 键位：WebView 键盘事件 → 规范串（修饰键顺序 Ctrl+Alt+Shift+Win）
  function specFromEvent(e) {
    const parts = []
    if (e.ctrlKey) parts.push('Ctrl')
    if (e.altKey) parts.push('Alt')
    if (e.shiftKey) parts.push('Shift')
    if (e.metaKey) parts.push('Win')
    const k = e.key
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(k)) return '' // 只按了修饰键，继续等
    let token
    if (k.length === 1) token = k.toUpperCase()
    else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(k)) token = k
    else {
      const map = {
        Backspace: 'Backspace', Tab: 'Tab', Enter: 'Enter', Escape: 'Esc', ' ': 'Space',
        PageUp: 'PageUp', PageDown: 'PageDown', End: 'End', Home: 'Home',
        ArrowLeft: 'Left', ArrowUp: 'Up', ArrowRight: 'Right', ArrowDown: 'Down',
        Insert: 'Insert', Delete: 'Delete', '.': '.', ',': ',', '/': '/', ';': ';', "'": "'",
        '[': '[', ']': ']', '\\': '\\', '-': '-', '=': '=', '`': '`',
      }
      token = map[k]
    }
    if (!token) return ''
    parts.push(token)
    // 无修饰键时只允许功能键
    if (parts.length === 1 && !/^F([1-9]|1[0-9]|2[0-4])$/.test(token)) return ''
    return parts.join('+')
  }

  function render() {
    if (!settings) return
    $in('set-autostart').checked = !!autostart
    $in('set-start-hidden').checked = !!settings.start_hidden
    $in('set-hotkey-on').checked = !!settings.hotkey_on
    $in('set-hotkey').value = settings.hotkey || ''
    $in('set-cue-on').checked = !!settings.cue_on
    $sel('set-cue-start').value = settings.cue_start || 'soft'
    $sel('set-cue-stop').value = settings.cue_stop || 'soft'
  }

  function open() {
    Promise.all([invoke('get_settings'), invoke('get_autostart'), invoke('debug_snapshot')])
      .then((res) => {
        const s = /** @type {any} */ (res[0])
        autostart = !!res[1]
        const snap = /** @type {any} */ (res[2])
        settings = s
        const ver = $('about-ver')
        if (ver) ver.textContent = snap.app.version
        const http = $('about-http')
        if (http) {
          http.textContent =
            snap.app.debug_http && snap.app.debug_http.state === 'ok' ? snap.app.debug_http.value : ''
        }
        render()
        $('settings')?.classList.remove('hidden')
      })
      .catch((e) => window.__pvDebug?.('读取设置失败：' + e))
  }

  function close() {
    $('settings')?.classList.add('hidden')
  }

  function save() {
    settings.hotkey_on = $in('set-hotkey-on').checked
    settings.hotkey = $in('set-hotkey').value.trim()
    settings.cue_on = $in('set-cue-on').checked
    settings.cue_start = $sel('set-cue-start').value
    settings.cue_stop = $sel('set-cue-stop').value
    settings.start_hidden = $in('set-start-hidden').checked
    invoke('set_autostart', { on: $in('set-autostart').checked }).catch(() => {})
    invoke('set_settings', { settings })
      .then(() => {
        window.__pvDebug?.('设置已保存（热键 ' + settings.hotkey + '）')
        close()
      })
      .catch((e) => window.__pvDebug?.('保存设置失败：' + e))
  }

  window.addEventListener('DOMContentLoaded', () => {
    // 提示音预设下拉
    invoke('list_cues')
      .then((raw) => {
        const list = /** @type {Array<{id: string, label: string}>} */ (raw)
        for (const id of ['set-cue-start', 'set-cue-stop']) {
          $sel(id).replaceChildren(...list.map((c) => new Option(c.label, c.id)))
        }
        if (settings) render()
      })
      .catch(() => {})
    $('btn-settings')?.addEventListener('click', open)
    $('set-close')?.addEventListener('click', close)
    $('set-save')?.addEventListener('click', save)
    // 键位录制
    const rec = $('set-rec')
    rec?.addEventListener('click', () => {
      rec.textContent = T('按键…')
      rec.focus()
    })
    rec?.addEventListener('blur', () => {
      rec.textContent = T('录制')
    })
    rec?.addEventListener('keydown', (e) => {
      e.preventDefault()
      const spec = specFromEvent(e)
      if (spec) {
        $in('set-hotkey').value = spec
        rec.textContent = T('录制')
        rec.blur()
      }
    })
  })
})()
