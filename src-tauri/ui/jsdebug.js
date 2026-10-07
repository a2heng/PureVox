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

// 前端调试桥（AGENTS.md §1）：脚本错误 / console 错误转发到调试接口
// （`GET /debug` 的 `ui` 字段），无控制台时也能定位；交互式调试用顶栏「开发者工具」。
;(() => {
  /** @param {'error'|'info'} kind @param {unknown} message */
  const send = (kind, message) => {
    try {
      window.__TAURI__.core.invoke('ui_report', { kind, message: String(message) })
    } catch (_) {
      /* Tauri 未就绪时忽略 */
    }
  }

  /** 手动上报一行（调试用）：window.__pvDebug('...') */
  window.__pvDebug = (message) => send('info', message)

  window.addEventListener('error', (e) => {
    const where = `${(e.filename || '').split('/').pop()}:${e.lineno || ''}`
    send('error', `${e.message || e.error} @ ${where}`)
  })
  window.addEventListener('unhandledrejection', (e) => {
    const r = /** @type {any} */ (e.reason)
    send('error', `未处理的 Promise 拒绝：${(r && (r.stack || r.message)) || r}`)
  })

  // console.error / console.warn 同时转发到调试接口（保留原行为）
  for (const level of /** @type {const} */ (['error', 'warn'])) {
    const orig = console[level].bind(console)
    console[level] = (...args) => {
      orig(...args)
      send(level === 'error' ? 'error' : 'info', args.map((a) => (a && a.stack) || String(a)).join(' '))
    }
  }

  // 顶栏「开发者工具」按钮：打开 WebView2 devtools（JS 交互式调试）
  window.addEventListener('DOMContentLoaded', () => {
    document.getElementById('btn-devtools')?.addEventListener('click', () => {
      window.__TAURI__.core.invoke('open_devtools')
    })
    // 调试面板开关（状态记忆）
    const btn = document.getElementById('btn-debug')
    const setDebug = (on) => {
      document.body.classList.toggle('debug-on', on)
      try {
        localStorage.setItem('pv_debug_on', on ? '1' : '0')
      } catch (_) {
        /* 忽略 */
      }
    }
    try {
      if (localStorage.getItem('pv_debug_on') === '1') document.body.classList.add('debug-on')
    } catch (_) {
      /* 忽略 */
    }
    btn?.addEventListener('click', () => {
      setDebug(!document.body.classList.contains('debug-on'))
      setTimeout(checkLayout, 100)
    })
  })

  // 布局自检：文档本身不得溢出（滚动只允许发生在音频列区与调试列）；顶栏子元素不得折行/溢出。
  const checkLayout = () => {
    const de = document.documentElement
    const over = de.scrollWidth > de.clientWidth + 1 || de.scrollHeight > de.clientHeight + 1
    const bar = document.getElementById('topbar')
    let barMsg = ''
    if (bar) {
      let tallest = 0
      bar.querySelectorAll('*').forEach((el) => {
        tallest = Math.max(tallest, el.getBoundingClientRect().height)
      })
      if (bar.scrollWidth > bar.clientWidth + 1 || tallest > bar.clientHeight + 1) {
        barMsg = `；顶栏溢出（scrollW ${bar.scrollWidth}/${bar.clientWidth}，最高子元素 ${Math.round(tallest)}/${bar.clientHeight}）`
      }
    }
    let rowMsg = ''
    document.querySelectorAll('.row').forEach((r) => {
      if (r.scrollWidth > r.clientWidth + 1) {
        rowMsg = `；行溢出（scrollW ${r.scrollWidth}/${r.clientWidth}）`
      }
    })
    if (over || barMsg || rowMsg) {
      send('error', `布局溢出：文档 ${de.scrollWidth}x${de.scrollHeight} > 视口 ${de.clientWidth}x${de.clientHeight}${barMsg}${rowMsg}`)
    } else {
      send('info', `布局正常：文档无外层滚动，顶栏/行无溢出，视口 ${de.clientWidth}x${de.clientHeight}`)
    }
  }
  window.addEventListener('load', () => setTimeout(checkLayout, 400))
  window.addEventListener('resize', checkLayout)
})()
