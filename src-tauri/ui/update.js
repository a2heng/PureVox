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

// 顶栏「更新」面板（状态机在 `src/update/`，数据源是本仓库 GitHub Releases）。
//
// 三步：**检查 → 下载 → 安装**，每一步只在前一步的状态下可点；进度与失败原因
// 直接显示（Rust 侧 reason 是诊断文本，按 AGENTS.md §4 边界不上 i18n）。
// 面板打开时顺手查一次，关着时不打扰；`update_status` 2 Hz 轮询刷新下载进度。

;(function () {
  'use strict'
  const { invoke } = window.__TAURI__.core
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  const $ = (id) => document.getElementById(id)

  /** @type {any} */
  let st = null
  /** @type {number|null} */
  let timer = null

  /** 阶段 → 界面文字。key 必须内联在 T() 里（i18n_lint 才查得到缺键）。 */
  const PHASE_TEXT = {
    idle: () => T('尚未检查'),
    checking: () => T('正在检查…'),
    uptodate: () => T('已是最新'),
    available: () => T('有新版本'),
    downloading: () => T('正在下载…'),
    ready: () => T('已下载，可以安装'),
    installing: () => T('正在安装…'),
    installed: () => T('已安装，重启 PureVox 生效'),
    unsupported: () => T('本平台不支持自动更新'),
    failed: () => T('失败'),
  }

  const mb = (n) => (n >= 1048576 ? (n / 1048576).toFixed(1) + ' MB' : Math.round(n / 1024) + ' KB')

  /** 阶段 → 状态灯颜色（与 .up-phase 的 class 对应）。 */
  const PHASE_CLASS = {
    idle: '', checking: '', uptodate: 'ok', available: 'ok', downloading: 'busy',
    ready: 'ok', installing: 'busy', installed: 'ok', unsupported: 'na', failed: 'na',
  }

  /**
   * 更新说明渲染：release body 是 **Markdown**，以前整段塞进 `textContent` → 换行全没了、
   * 一堆 `- feat(x): …` 连成一大坨（2026-10-26 用户：「没有换行，杂乱」）。
   * 这里只认发布说明实际会用到的四种：`#`/`##` 标题、`- ` 列表、空行分段、其余按段落
   * （`pre-wrap` + `overflow-wrap` 保证长行不撑破面板）。不引第三方 Markdown 解析器。
   */
  /**
   * 行内 Markdown 的最小实现：`**粗体**`、`` `等宽` ``、`[文字](链接)`。
   *
   * 以前整段用 `textContent` 写进去 → 面板里直接把 `**Windows**` 和反引号原样显示出来
   * （2026-10-26 用户：「猴版」）。这里按标记切成 text / strong / code / a 节点逐个挂上，
   * **不碰 innerHTML**（release 说明来自外部，别给它开 HTML 注入口子）。
   */
  function inlineInto(el, text) {
    // 反引号写成 \u0060：i18n 门禁的扫描器不认正则字面量，会把裸反引号当成模板字符串
    // 的开头，于是后面整段代码都被当成「没翻译的中文串」（E1 误报）。转义之后绕开。
    const re = /\*\*([^*]+)\*\*|[\u0060]([^\u0060]+)[\u0060]|\[([^\]]+)\]\(([^)\s]+)\)/g
    let last = 0
    let m
    while ((m = re.exec(text)) !== null) {
      if (m.index > last) el.appendChild(document.createTextNode(text.slice(last, m.index)))
      if (m[1] != null) {
        const b = document.createElement('strong')
        b.textContent = m[1]
        el.appendChild(b)
      } else if (m[2] != null) {
        const c = document.createElement('code')
        c.textContent = m[2]
        el.appendChild(c)
      } else {
        const a = document.createElement('a')
        a.textContent = m[3]
        a.href = m[4]
        a.addEventListener('click', (ev) => {
          ev.preventDefault()
          invoke('update_open_page', { url: m[4] }).catch(() => {})
        })
        el.appendChild(a)
      }
      last = re.lastIndex
    }
    if (last < text.length) el.appendChild(document.createTextNode(text.slice(last)))
  }

  function renderNotes(text) {
    const box = $('up-notes')
    const len = $('up-notes-len')
    if (!box) return
    box.replaceChildren()
    const raw = String(text || '').replace(/\r\n?/g, '\n').trim()
    if (!raw) {
      if (len) len.textContent = ''
      const p = document.createElement('p')
      p.className = 'up-note-empty'
      p.textContent = T('这个 release 没有更新说明')
      box.appendChild(p)
      return
    }
    const lines = raw.split('\n')
    if (len) len.textContent = T('{n} 行', { n: lines.length })
    let list = null
    const flush = () => { list = null }
    for (const line of lines) {
      const t = line.trimEnd()
      if (!t.trim()) { flush(); continue }
      const h = /^(#{1,3})\s+(.*)$/.exec(t)
      if (h) {
        flush()
        const el = document.createElement('div')
        el.className = 'up-note-h' + (h[1].length === 1 ? ' h1' : '')
        inlineInto(el, h[2])
        box.appendChild(el)
        continue
      }
      const li = /^[-*]\s+(.*)$/.exec(t)
      if (li) {
        if (!list) {
          list = document.createElement('ul')
          list.className = 'up-note-list'
          box.appendChild(list)
        }
        const item = document.createElement('li')
        inlineInto(item, li[1])
        list.appendChild(item)
        continue
      }
      flush()
      const p = document.createElement('p')
      p.className = 'up-note-p'
      inlineInto(p, t.trim())
      box.appendChild(p)
    }
  }

  function paint() {
    if (!st) return
    const dev = !st.current_stamp
    const cur = $('up-cur')
    if (cur) cur.textContent = st.current_version + (dev ? '（' + T('开发构建') + '）' : '')
    const latest = $('up-latest')
    if (latest) {
      // 日期只留「年-月-日」：带时间与 UTC 在窄面板里换行，且对「要不要更新」没影响
      const day = (st.published_at || '').slice(0, 10)
      // 只写 tag（里面已经带 yy.MM.dd）+ 日期：再写一遍显示版本是纯重复
      latest.textContent = st.latest_tag ? st.latest_tag + (day ? '  ·  ' + day : '') : '—'
    }
    const asset = $('up-asset')
    if (asset) {
      if (st.asset_name) asset.textContent = st.asset_name + (st.asset_size ? '  ·  ' + mb(st.asset_size) : '')
      else if (st.latest_tag) asset.textContent = T('没有匹配到本机安装方式可用的包') + '（' + st.install_kind + '）'
      else asset.textContent = '—'
    }
    const phase = $('up-phase')
    if (phase) {
      phase.textContent = PHASE_TEXT[st.phase] ? PHASE_TEXT[st.phase]() : st.phase
      phase.className = 'up-phase ' + (PHASE_CLASS[st.phase] || '')
    }
    // 状态行只在「有额外信息」时出现：是否有新版本由标题旁的 chip 说，不重复一遍
    // （以前这里再写一遍「有新版本」，加上 dev 构建的说明，一屏三处重复同样的意思）。
    const stateRow = $('up-state-row')
    const state = $('up-state')
    const extra = st.reason || ''
    const needRow = !!extra && st.phase !== 'available' && st.phase !== 'uptodate'
    if (stateRow && state) {
      stateRow.classList.toggle('hidden', !needRow)
      state.textContent = needRow ? extra : ''
      state.className = st.phase === 'failed' || st.phase === 'unsupported' ? 'na'
        : st.phase === 'ready' || st.phase === 'installed' ? 'ok' : ''
    }

    // 下载进度：只在下载/已下载时占位，平时收起来（不占高度）
    const prog = $('up-progress')
    const bar = $('up-bar')
    const show = (st.phase === 'downloading' || st.phase === 'ready') && st.asset_size > 0
    if (prog && bar) {
      prog.classList.toggle('on', show)
      const pct = show ? Math.min(100, (st.received / st.asset_size) * 100) : 0
      bar.style.width = pct.toFixed(1) + '%'
      prog.title = show ? `${mb(st.received)} / ${mb(st.asset_size)} (${pct.toFixed(0)}%)` : ''
    }

    renderNotes(st ? st.notes : '')

    // 按钮可用性：严格按状态机（不允许跳步，避免对着错误产物操作）
    const busy = st && (st.phase === 'checking' || st.phase === 'downloading' || st.phase === 'installing')
    const bCheck = /** @type {HTMLButtonElement|null} */ ($('up-check'))
    const bDl = /** @type {HTMLButtonElement|null} */ ($('up-download'))
    const bIn = /** @type {HTMLButtonElement|null} */ ($('up-install'))
    const bPage = /** @type {HTMLButtonElement|null} */ ($('up-page'))
    if (bCheck) bCheck.disabled = !!busy
    if (bDl) {
      bDl.disabled = !!busy || !st || (st.phase !== 'available' && st.phase !== 'failed')
      bDl.textContent = st && st.phase === 'ready' ? T('重新下载') : T('下载更新')
    }
    if (bIn) bIn.disabled = !st || st.phase !== 'ready'
    if (bPage) bPage.disabled = !st || !st.url
  }

  async function refresh() {
    try {
      st = /** @type {any} */ (await invoke('update_status'))
      paint()
    } catch (e) {
      if (window.__pvDebug) window.__pvDebug(T('读取更新状态失败：') + e)
    }
  }

  function openPanel() {
    $('update')?.classList.remove('hidden')
    refresh().then(() => check())
  }
  function closePanel() {
    $('update')?.classList.add('hidden')
  }

  function check() {
    invoke('update_check')
      .then((s) => {
        st = s
        paint()
      })
      .catch((e) => {
        window.__pvDebug?.('检查更新失败：' + e)
        refresh()
      })
  }

  function download() {
    invoke('update_download')
      .then((s) => {
        st = s
        paint()
      })
      .catch((e) => {
        window.__pvDebug?.('下载失败：' + e)
        refresh()
      })
  }

  function install() {
    invoke('update_install')
      .then((msg) => {
        // Windows：安装器接管，进程 1 s 后退出（界面来不及显示什么）；
        // Linux：装完状态变 installed，刷新一次就能看到「重启生效」。
        window.__pvDebug?.(typeof msg === 'string' ? msg : '已拉起安装器')
        refresh()
      })
      .catch((e) => {
        window.__pvDebug?.('安装失败：' + e)
        refresh()
      })
  }

  function openPage() {
    if (st && st.url) invoke('update_open_page', { url: st.url }).catch(() => {})
  }

  window.addEventListener('DOMContentLoaded', () => {
    $('btn-update')?.addEventListener('click', openPanel)
    $('up-close')?.addEventListener('click', closePanel)
    $('up-check')?.addEventListener('click', check)
    $('up-download')?.addEventListener('click', download)
    $('up-install')?.addEventListener('click', install)
    $('up-page')?.addEventListener('click', openPage)
    refresh()
    timer = window.setInterval(refresh, 500)
    // 语言切换：阶段文字与按钮标签是动态写入的，按当前语言重放
    document.addEventListener('pv-langchange', () => {
      paint()
      refresh()
    })
    window.addEventListener('beforeunload', () => {
      if (timer !== null) window.clearInterval(timer)
    })
  })
})()