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

// 「驱动」面板 · Linux 实现：Linux 虚拟驱动（PipeWire 虚拟麦克风 out + mic）。
// 状态来自 Tauri 命令 `virtual_mic_status`（与调试快照 `virtual_mic` 同源）；
// 模板 id 与本地化标题交由 ui/drivers.js 外壳统一挂载。
window.__pvDriversLinux = (() => {
  const { invoke } = window.__TAURI__.core
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  const $ = (id) => document.getElementById(id)
  const $btn = (id) => /** @type {HTMLButtonElement} */ ($(id))
  const $txt = (id) => /** @type {HTMLElement} */ ($(id))

  /** @type {any} */ let probe = null
  let busy = false
  /** @type {any} */ let els = null

  /** 按当前 probe / busy 重绘（动态文字；重挂时重放）。 */
  function paint() {
    if (!els) return
    const created = (b) => (b ? T('已创建') : T('未创建'))
    if (!probe || probe.state === 'pending') {
      els.sink.textContent = '—'
      els.src.textContent = '—'
      els.state.textContent = T('正在处理…')
    } else if (probe.state === 'ok') {
      els.sink.textContent = created(!!probe.value.sink)
      els.src.textContent = created(!!probe.value.source)
      els.state.textContent = T('正常')
    } else {
      els.sink.textContent = '—'
      els.src.textContent = '—'
      els.state.textContent = T('不可用') + '：' + (probe.reason || '')
    }
    const dis = busy || !probe || probe.state !== 'ok'
    if (els.create) els.create.disabled = dis
    if (els.remove) els.remove.disabled = dis
  }

  function refresh() {
    invoke('virtual_mic_status')
      .then((/** @type {any} */ p) => {
        probe = p
        paint()
      })
      .catch((e) => {
        probe = { state: 'unavailable', reason: String(e) }
        paint()
      })
  }

  function action(cmd) {
    if (busy) return
    busy = true
    paint()
    invoke(cmd)
      .then((/** @type {any} */ s) => {
        probe = { state: 'ok', value: s }
        window.__pvDebug?.(cmd + ' 成功')
      })
      .catch((e) => window.__pvDebug?.(cmd + ' 失败：' + e))
      .finally(() => {
        busy = false
        refresh()
      })
  }

  return {
    template: 'drv-linux-tpl',
    title: () => T('Linux 虚拟驱动'),
    mount() {
      els = {
        sink: $txt('drv-sink'),
        src: $txt('drv-source'),
        state: $txt('drv-state'),
        create: $btn('drv-create'),
        remove: $btn('drv-remove'),
      }
      $btn('drv-refresh')?.addEventListener('click', refresh)
      $btn('drv-create')?.addEventListener('click', () => action('virtual_mic_create'))
      $btn('drv-remove')?.addEventListener('click', () => action('virtual_mic_remove'))
      paint()
    },
    refresh,
  }
})()
