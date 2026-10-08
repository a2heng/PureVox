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

// 「驱动」面板 · Windows 实现：VB-CABLE 虚拟声卡驱动（下载 / 教程 / 有无检测）。
// 有无检测复用设备枚举结果（输出含 "CABLE Input" 且输入含 "CABLE Output"）；
// 模板 id 与本地化标题交由 ui/drivers.js 外壳统一挂载。
window.__pvDriversWindows = (() => {
  const { invoke } = window.__TAURI__.core
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  const $ = (id) => document.getElementById(id)
  const $btn = (id) => /** @type {HTMLButtonElement} */ ($(id))
  const $txt = (id) => /** @type {HTMLElement} */ ($(id))

  // 官方驱动包与视频教程（与 legacy 同一链接）。
  const DOWNLOAD_URL = 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip'
  const TUTORIAL_URL = 'https://www.bilibili.com/video/BV1i2bazGEKe/'

  /** null = 检测中 */
  /** @type {boolean|null} */ let installed = null
  /** @type {any} */ let els = null

  function paint() {
    if (!els) return
    if (installed === null) els.state.textContent = T('检测中…')
    else els.state.textContent = installed ? T('已安装') : T('未安装')
    if (els.guide) els.guide.style.display = installed ? 'none' : ''
  }

  function refresh() {
    installed = null
    paint()
    invoke('debug_snapshot')
      .then((/** @type {any} */ s) => {
        const ds = s.devices && s.devices.state === 'ok' ? s.devices.value.devices : []
        const out = ds.some(
          (/** @type {any} */ d) => d.direction === 'output' && d.name.includes('CABLE Input')
        )
        const inp = ds.some(
          (/** @type {any} */ d) => d.direction === 'input' && d.name.includes('CABLE Output')
        )
        installed = out && inp
        paint()
      })
      .catch((e) => {
        window.__pvDebug?.('检测 VB-CABLE 失败：' + e)
        installed = false
        paint()
      })
  }

  function openLink(url) {
    invoke('open_url', { url }).catch((e) => window.__pvDebug?.('打开链接失败：' + e))
  }

  return {
    template: 'drv-windows-tpl',
    title: () => T('VB-CABLE 驱动'),
    mount() {
      els = { state: $txt('vb-state'), flow: $txt('vb-flow'), guide: $txt('vb-guide') }
      if (els.flow) els.flow.textContent = T('麦克风 → PureVox → CABLE In → CABLE Out → 其它软件')
      $btn('vb-download')?.addEventListener('click', () => openLink(DOWNLOAD_URL))
      $btn('vb-tutorial')?.addEventListener('click', () => openLink(TUTORIAL_URL))
      $btn('vb-refresh')?.addEventListener('click', refresh)
      paint()
    },
    refresh,
  }
})()
