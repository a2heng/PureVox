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

// 「驱动」面板外壳：顶栏按钮 → 同一面板；按平台把内容交给对应的平台模块
// （Linux = drivers_linux.js / Windows = drivers_windows.js），位置与 UI 一致。
;(() => {
  const { invoke } = window.__TAURI__.core
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  const $ = (id) => document.getElementById(id)

  /** @type {any} */ let impl = null

  /** 对已挂载的子树重放静态 data-i18n（动态插入的节点不在 DOMContentLoaded 的扫描里）。 */
  function applyStatic(root) {
    root.querySelectorAll('[data-i18n]').forEach((/** @type {HTMLElement} */ el) => {
      el.textContent = T(el.dataset.i18n || '')
    })
  }

  function mount() {
    const body = $('drivers-body')
    const title = $('drivers-title')
    if (!body) return
    body.replaceChildren()
    if (!impl) {
      if (title) title.textContent = T('驱动')
      return
    }
    if (title) title.textContent = impl.title()
    const tpl = document.getElementById(impl.template)
    if (tpl instanceof HTMLTemplateElement) body.append(tpl.content.cloneNode(true))
    applyStatic(body)
    impl.mount(body)
  }

  function open() {
    $('drivers')?.classList.remove('hidden')
    mount()
    impl?.refresh?.()
  }
  function close() {
    $('drivers')?.classList.add('hidden')
  }

  window.addEventListener('DOMContentLoaded', async () => {
    $('btn-drivers')?.addEventListener('click', open)
    $('drv-close')?.addEventListener('click', close)
    let platform = 'other'
    try {
      platform = await invoke('get_platform')
    } catch (e) {
      window.__pvDebug?.('取平台失败：' + e)
    }
    if (platform === 'linux') impl = window.__pvDriversLinux
    else if (platform === 'windows') impl = window.__pvDriversWindows
    else window.__pvDebug?.('驱动页：当前平台无实现（' + platform + '）')
    // 语言切换：整块重挂（平台模块的静态串与动态串都会按新语言重建）
    document.addEventListener('pv-langchange', () => {
      if (!$('drivers')?.classList.contains('hidden')) mount()
    })
  })
})()
