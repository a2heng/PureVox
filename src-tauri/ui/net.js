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

// 手机 ⇄ 电脑（DESIGN.md §4.1）：服务开关 + 远程输入开关 + 状态轮询。
// 状态每 1 s 从 `net_status` 拉一次（只读，不进音频线程）；`/debug` 的 `net` 段是同源数据。

;(function () {
  'use strict'

  /** 与 columns.js 同款：i18n 字典查找（缺键回退中文）。无占位符，只传一个参数。 */
  const T = (/** @type {string} */ s) => (window.__pvT ? window.__pvT(s) : s)

  /** @type {number|null} */
  let timer = null
  let remoteInput = false

  /**
   * 调 Rust 命令。返回值类型由调用方断言（与项目其余前端一致），
   * 因为 invoke 只能返回 unknown。
   * @param {string} cmd
   * @param {Record<string, unknown>} [args]
   */
  function invoke(cmd, args) {
    if (!window.__TAURI__) return Promise.reject(new Error(T('无 Tauri 运行时')))
    return window.__TAURI__.core.invoke(cmd, args)
  }

  /** 取字符串返回值（Rust 侧都是 String）。 */
  async function invokeText(cmd, args) {
    return /** @type {string} */ (await invoke(cmd, args))
  }

  function el(id) {
    return /** @type {HTMLElement|null} */ (document.getElementById(id))
  }

  /** 状态文字优先用 Rust 给的（`/debug` 同源），失败才退回本地拼接。 */
  async function refresh() {
    const cell = el('net-status')
    if (!cell) return
    try {
      const s = await invokeText('net_status')
      cell.textContent = s
    } catch (e) {
      cell.textContent = T('状态不可用：') + String(e)
    }
  }

  function setBusy(on) {
    for (const id of ['net-start', 'net-stop']) {
      const b = /** @type {HTMLButtonElement|null} */ (el(id))
      if (b) b.disabled = on
    }
  }

  async function startService() {
    setBusy(true)
    try {
      const msg = await invokeText('net_start')
      if (window.__pvDebug) window.__pvDebug(T('网络服务：') + msg)
    } catch (e) {
      const cell = el('net-status')
      if (cell) cell.textContent = T('启动失败：') + String(e)
      if (window.__pvDebug) window.__pvDebug(T('网络服务启动失败：') + String(e))
    } finally {
      setBusy(false)
      refresh()
    }
  }

  async function stopService() {
    setBusy(true)
    try {
      await invoke('net_stop')
      // 停服务同时强制关远程输入，同步勾选框
      remoteInput = false
      const cb = /** @type {HTMLInputElement|null} */ (el('net-remote-input'))
      if (cb) cb.checked = false
    } catch (e) {
      if (window.__pvDebug) window.__pvDebug(T('网络服务停止失败：') + String(e))
    } finally {
      setBusy(false)
      refresh()
    }
  }

  async function onRemoteInputChange(checked) {
    try {
      const msg = await invokeText('net_set_remote_input', { on: checked })
      remoteInput = checked
      if (window.__pvDebug) window.__pvDebug(T('远程输入：') + msg)
    } catch (e) {
      // 开关失败必须回滚勾选状态，不能让界面显示「已开」而实际是关的
      const cb = /** @type {HTMLInputElement|null} */ (el('net-remote-input'))
      if (cb) cb.checked = remoteInput
      if (window.__pvDebug) window.__pvDebug(T('远程输入开关失败：') + String(e))
    }
    refresh()
  }

  window.addEventListener('DOMContentLoaded', () => {
    el('net-start')?.addEventListener('click', startService)
    el('net-stop')?.addEventListener('click', stopService)
    el('net-remote-input')?.addEventListener('change', (ev) => {
      onRemoteInputChange(/** @type {HTMLInputElement} */ (ev.target).checked)
    })
    refresh()
    timer = window.setInterval(refresh, 1000)
    // 语言切换后重绘（AGENTS.md §4：动态写入的文本必须可重放）
    document.addEventListener('pv-langchange', refresh)
    window.addEventListener('beforeunload', () => {
      if (timer !== null) window.clearInterval(timer)
    })
  })
})()