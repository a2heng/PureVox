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

// 主窗口的快照轮询（2 Hz）。
//
// 以前这个循环住在 `debug.js` 里——但调试页搬去独立窗口（debug.html）后，主窗口就不再加载
// debug.js，于是 `__pvOnSnapshot` 永远收不到数据：设备下拉只剩「未选择设备 / 设备不在」，
// 麦克风、输出都选不了（2026-10-26 用户实测）。所以把它拆成独立模块：
// **主窗口也要有快照**，设备列表 / 运行状态 / 录制校准状态都靠它；调试窗口另有自己的轮询。
//
// 纪律：只取快照并转发，不渲染、不计算（AGENTS §1.3 单一数据源）。
;(() => {
  const { invoke } = window.__TAURI__.core
  const POLL_MS = 500

  async function tick() {
    try {
      const s = await invoke('debug_snapshot')
      window.__pvOnSnapshot?.(s)
      document.dispatchEvent(new CustomEvent('pv-snapshot', { detail: s }))
    } catch (e) {
      window.__pvDebug?.('主界面取快照失败：' + e)
      document.dispatchEvent(new CustomEvent('pv-snapshot-error', { detail: String(e) }))
    }
    setTimeout(tick, POLL_MS)
  }

  tick()
})()
