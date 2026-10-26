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

// 快照轮询（2 Hz）：主窗口与调试面板共用同一份实现、同一频率。
//
// 两条取数通道，**同一份 `DebugSnapshot`**（AGENTS §1.3 单一数据源）：
// - 主窗口：Tauri IPC `debug_snapshot`（进程内）；
// - 调试面板：浏览器里没有 IPC，走本机只读接口 `GET /debug`（页面与接口同源）。
// 以前这个循环住在 `debug.js` 里，面板搬走后主窗口就收不到快照（设备下拉只剩「未选择设备」，
// 2026-10-26 用户实测），所以拆成独立模块。
//
// 纪律：只取快照并转发，不渲染、不计算。
;(() => {
  const invoke = window.__TAURI__?.core?.invoke
  const POLL_MS = 500

  /** 取一份快照：IPC（主窗口）或 HTTP（浏览器里的调试面板）。 */
  async function getSnapshot() {
    if (invoke) return invoke('debug_snapshot')
    const r = await fetch('/debug', { cache: 'no-store' })
    if (!r.ok) throw new Error('HTTP ' + r.status + ' ' + r.statusText)
    return r.json()
  }

  async function tick() {
    try {
      const s = await getSnapshot()
      window.__pvOnSnapshot?.(s)
      document.dispatchEvent(new CustomEvent('pv-snapshot', { detail: s }))
    } catch (e) {
      window.__pvDebug?.('取快照失败：' + e)
      document.dispatchEvent(new CustomEvent('pv-snapshot-error', { detail: String(e) }))
    }
    setTimeout(tick, POLL_MS)
  }

  tick()
})()
