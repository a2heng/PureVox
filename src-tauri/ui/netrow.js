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

// 网络行（`remote_mic` 网络麦克风 / `remote_speaker` 网络扬声器）的卡片内容。
//
// **整张卡片的职责只有两件事**：让客户端连上来（地址 / 二维码 / 证书），以及把这一行
// 自己的可调项摆出来（网卡 / 端口 / 缓冲 / 远程输入 / AEC）。所以 2026-10-27 按用户要求
// 做成**极致紧凑**：
//   - 不可改的状态一律收成「状态灯 + 一句短状态」，全文在 `/debug` 的 `net` 段；
//   - 静态说明（「这个行是干什么的」「近端不可更改」）全部删掉 —— 看一次就懂的东西
//     不该每块卡片都占一行；
//   - 二维码右边就是按钮 / 开关，不再放一栏说明文字；
//   - AEC 的模型是**内置的、与独立 AEC 行同一个**（`infer::aec::MODEL_AEC`），
//     所以这里没有模型下拉（模型内置），也没有「近端」这个不可改的
//     选择器 —— 近端本来就是这一行的网络输入。
//
// 与顶栏「手机」面板读的是同一份状态（`net_card` / `net_local_addr` / `net_ifaces`，
// 都是 `/debug` 的 `net` 段同源）。网络行卡片是**唯一**的手机/客户端操作入口：顶栏
// 不再有独立的「手机」面板（网络行就是唯一的手机入口），没建网络行时想连客户端就只能先把行加上，这是有意的。

;(function () {
  'use strict'

  /** @param {string} s @param {Record<string, string|number>} [vars] */
  const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)
  /**
   * @param {string} text
   * @param {'info'|'ok'|'warn'|'error'} [level]
   * @param {number} [ms]
   */
  const toast = (text, level = 'info', ms) => window.__pvToast?.(text, level, ms)

  /**
   * @param {string} cmd @param {Record<string, unknown>} [args]
   * @returns {Promise<any>}
   */
  function invoke(cmd, args) {
    if (!window.__TAURI__) return Promise.reject(new Error(T('无 Tauri 运行时')))
    return window.__TAURI__.core.invoke(cmd, args)
  }

  /** 当前地址（`https://ip:port/`），失败给原因。同一份数据给二维码与「复制」。 */
  let addrUrl = ''
  /** 当前对外广告的 IP（子网下拉要选中它） */
  let addrIp = ''
  /** 服务端口（端口框要显示它） */
  let addrPort = 59123
  /** 接收方播放环深度（ms，手动可调；滑块显示它） */
  let ringTarget = 20
  /** 可选网卡列表（子网下拉的选项）。`[]` = 不支持枚举或一张都没有。 */
  /** @type {{ip: string, name: string, desc: string, is_default: boolean}[]} */
  let ifaces = []
  /** 取不到地址的原因（没网卡 / 没默认路由）。空 = 取到了。 */
  let addrReason = ''
  /**
   * 最近一次 `net_card`（结构化、紧凑；判定与数字全在 Rust 侧算）。
   * `null` = 还没取到 / 取失败。
   * @type {{running: boolean, port: number, clients: number, subs: number,
   *   rx_level_ms: number|null, rx_target_ms: number, remote_input: boolean,
   *   keys_backend: string|null, keys_reason: string|null,
   *   last_error: string|null, codec_ok: boolean, codec_reason: string|null}|null}
   */
  let card = null
  /** 取不到状态 / 地址的原因（合并显示，不静默） */
  let statusReason = ''
  /** 本次挂载里建过的卡片，重绘时统一收摊 */
  /** @type {Set<{el: HTMLElement, paint: () => void}>} */
  const cards = new Set()
  /**
   * 环深度滑块的「回绘」回调（每张网络卡一份）。
   *
   * **必须单独收集**：`cards` 里的 `paint` 是整张卡的数据回绘，而滑块的值来自
   * `net_ring_ms`（单独一次 IPC），手机那边拖动后电脑要跟着动，就得在 `poll()` 里
   * 再刷一遍滑块 —— 只在建卡时画一次的话，手机上改了这边界面纹丝不动（实测 2026-10-27）。
   * @type {Set<() => void>}
   */
  const ringPainters = new Set()
  /** @type {number|null} */
  let timer = null

  /**
   * 拉一次状态 + 地址 + 网卡，刷新所有已挂载的网络行卡片。
   * @returns {Promise<void>}
   */
  async function poll() {
    let failed = ''
    try {
      const [c, a, ifs, ring] = await Promise.all([
        invoke('net_card'),
        invoke('net_local_addr'),
        invoke('net_ifaces'),
        invoke('net_ring_ms'),
      ])
      card = c || null
      if (Number.isFinite(Number(ring))) ringTarget = Number(ring)
      addrUrl = a && a.url ? String(a.url) : ''
      addrIp = a && a.ip ? String(a.ip) : ''
      addrReason = a && a.reason ? String(a.reason) : ''
      if (a && a.port) addrPort = Number(a.port)
      if (Array.isArray(ifs)) ifaces = ifs
      if (!card) failed = T('状态不可用')
    } catch (e) {
      failed = String(e)
      card = null
      addrUrl = ''
      addrIp = ''
      addrReason = String(e)
    }
    statusReason = failed
    for (const c of cards) {
      if (c.el.isConnected) c.paint()
      else cards.delete(c)
    }
    // 环深度可能在**别的客户端**被改过（手机滑块 / 另一台电脑）：每轮都回绘一次。
    // 只在建卡时画一次的话，手机上改了这边界面纹丝不动（实测 2026-10-27）。
    for (const f of ringPainters) f()
  }

  /** @param {HTMLButtonElement} b @param {string} label */
  function busy(b, label) {
    b.disabled = true
    b.dataset.was = b.textContent || ''
    if (label) b.textContent = label
  }

  /**
   * 跑一个服务动作，按钮上给反馈，成功 / 失败都走右下角气泡（不静默）。
   * @param {HTMLButtonElement} btn @param {string} cmd @param {string} doing
   * @param {string} done 成功文案（调用点已过 T()）
   */
  async function action(btn, cmd, doing, done) {
    busy(btn, doing)
    try {
      const r = await invoke(cmd)
      if (window.__pvDebug) window.__pvDebug(T('网络服务：') + String(r))
      if (done) toast(done, 'ok')
    } catch (e) {
      toast(T('网络服务失败：') + String(e), 'error', 7000)
      if (window.__pvDebug) window.__pvDebug(T('网络服务失败：') + String(e))
    } finally {
      btn.disabled = false
      btn.textContent = btn.dataset.was || ''
      await poll()
    }
  }

  /** 复制到剪贴板；失败要说出来（WebView2 的剪贴板在非用户手势里可能被拒）。 */
  async function copy(text, btn) {
    try {
      if (!navigator.clipboard) throw new Error(T('浏览器不支持剪贴板'))
      await navigator.clipboard.writeText(text)
      const old = btn.dataset.was2 || btn.textContent || ''
      btn.dataset.was2 = old
      btn.textContent = T('已复制')
      window.setTimeout(() => {
        btn.textContent = old
      }, 1200)
      toast(T('已复制客户端地址'), 'ok', 2500)
    } catch (e) {
      toast(T('复制失败：') + String(e), 'error', 6000)
      if (window.__pvDebug) window.__pvDebug(T('复制失败：') + String(e))
    }
  }

  /**
   * 在一个卡片里画二维码。内容就是本机地址，手机扫了直接打开网页版客户端。
   * @param {HTMLElement} host
   */
  function drawQr(host) {
    const url = addrUrl
    const canvas = /** @type {HTMLCanvasElement|null} */ (host.querySelector('canvas'))
    const note = /** @type {HTMLElement|null} */ (host.querySelector('.netqr-note'))
    if (!canvas || !window.__pvQr) return
    const running = !!card?.running
    // 服务没跑就不给码：地址是能算出来的，但扫过去连不上。与其给一张必然失败的码，不如
    // 说清楚为什么没码（DEBUG.md §1.1：不显示伪造 / 无效的值）。
    if (!url || !running) {
      const ctx = canvas.getContext('2d')
      if (ctx) {
        ctx.clearRect(0, 0, canvas.width, canvas.height)
      }
      canvas.classList.add('off')
      if (note) {
        note.textContent = url
          ? T('服务未启动，没有可扫的地址')
          : addrReason || T('取不到本机地址')
      }
      return
    }
    canvas.classList.remove('off')
    try {
      window.__pvQr.render(url, canvas, { scale: 4, margin: 2 })
      if (note) note.textContent = ''
    } catch (e) {
      canvas.classList.add('off')
      if (note) note.textContent = T('二维码生成失败：') + String(e)
    }
  }

  /** 小标签 */
  const nl = (t) => {
    const s = document.createElement('span')
    s.className = 'netcard-label'
    s.textContent = t
    return s
  }

  /**
   * @param {HTMLElement} elRow 行容器（`columns.js` 传进来）
   * @param {number} ci @param {number} ri @param {any} row
   * @param {{apply?: () => void}} [hooks]
   */
  function attach(elRow, ci, ri, row, hooks) {
    const apply = hooks && hooks.apply
    const out = row.ptype === 'remote_speaker'
    // 旧配置里「直通」可能存成 `'1'`（独立 AEC 行的写法），而参数渲染器只认 `'true'`：
    // 不归一化就会出现「实际在直通、界面没勾」。在这里收口一次，不给第二套判定。
    if (row.params.bypass === '1') row.params.bypass = 'true'

    const box = document.createElement('div')
    box.className = 'netcard'

    // ---- 1) 状态灯 + 一句短状态 + 生命周期按钮 ----
    const bar = document.createElement('div')
    bar.className = 'netcard-bar'
    const dot = document.createElement('span')
    dot.className = 'netdot'
    const status = document.createElement('span')
    status.className = 'netcard-status mono'
    bar.append(dot, status)

    const acts = document.createElement('span')
    acts.className = 'netcard-acts'
    /**
     * 按钮文案必须**在调用点**就写 `T('…')` 字面量：i18n 门禁（E1）靠扫描
     * `T('…')` 的字面量收集 msgid，`T(变量)` 它看不见，动态文案也翻译不了。
     * @param {string} text 已翻译的按钮文字
     * @param {string} cmd @param {string} doing 已翻译的进行中文字
     * @param {string} done 已翻译的成功文字
     * @param {string} tip 已翻译的 tooltip
     * @param {boolean} [primary]
     */
    const mk = (text, cmd, doing, done, tip, primary) => {
      const b = document.createElement('button')
      b.type = 'button'
      b.textContent = text
      b.title = tip
      if (primary) b.className = 'primary'
      b.addEventListener('click', () => action(b, cmd, doing, done))
      acts.appendChild(b)
      return b
    }
    // 三个生命周期按钮。**不做「隐藏」，只做「禁用」** —— 禁用时按钮还在原位，卡片
    // 不会因为服务起停而重排，界面不跳（DEBUG.md §1.1 的固定行数刷新同理）。
    const bStart = mk(
      T('启动'),
      'net_start',
      T('启动中'),
      T('网络服务已启动'),
      T('启动局域网服务，客户端才能连进来'),
    )
    const bStop = mk(
      T('停止'),
      'net_stop',
      T('停止中'),
      T('网络服务已停止'),
      T('停止服务并断开所有客户端（远程输入同时强制关闭）'),
    )
    mk(
      T('重启服务'),
      'net_restart',
      T('重启中'),
      T('网络服务已重启'),
      T('重开监听并重新绑定端口（换端口后用这个生效）'),
      true,
    )
    bar.appendChild(acts)
    box.appendChild(bar)

    // ---- 2) 地址 + 复制 + 网卡 + 端口（全是一行，能折行） ----
    // 不再写「客户端地址 / 网卡 / 端口」三个标签：它们都是**可调项的标题**，
    // 每张卡都写一遍只是在挤占横向空间。语义放进 title / aria-label。
    const addrBox = document.createElement('div')
    addrBox.className = 'netcard-addr'
    const url = document.createElement('span')
    url.className = 'mono netcard-url'
    url.title = T('客户端地址')
    const copyBtn = document.createElement('button')
    copyBtn.type = 'button'
    copyBtn.className = 'icon netcard-copy'
    copyBtn.textContent = T('复制')
    copyBtn.title = T('复制客户端地址')
    copyBtn.setAttribute('aria-label', T('复制客户端地址'))
    copyBtn.addEventListener('click', () => copy(addrUrl, copyBtn))

    const ifSel = document.createElement('select')
    ifSel.className = 'netcard-sel netcard-ifsel'
    ifSel.title = T('只决定二维码 / 复制的是哪个地址，服务不用重启（本来就在监听所有网卡）')
    ifSel.setAttribute('aria-label', T('网卡'))
    ifSel.addEventListener('change', () => {
      const v = ifSel.value
      invoke('net_select_ip', { ip: v })
        .then((r) => {
          addrIp = v
          if (window.__pvDebug) window.__pvDebug(String(r))
        })
        .catch((e) => {
          toast(T('切换网卡失败：') + String(e), 'error', 6000)
          if (window.__pvDebug) window.__pvDebug(T('切换网卡失败：') + String(e))
        })
        .finally(() => {
          poll()
        })
    })

    const portIn = document.createElement('input')
    portIn.type = 'number'
    portIn.min = '1024'
    portIn.max = '65535'
    portIn.step = '1'
    portIn.className = 'netcard-port mono'
    portIn.title = T('改完自动重启服务；已连着的客户端需要重新连')
    portIn.setAttribute('aria-label', T('端口'))
    const portHint = document.createElement('span')
    portHint.className = 'netcard-port-hint'
    portIn.addEventListener('change', () => {
      const p = Number(portIn.value)
      if (!Number.isInteger(p) || p < 1024 || p > 65535) {
        portIn.value = String(addrPort) // 非法值回滚，别留个填不回去的框
        toast(T('端口要在 1024~65535 之间'), 'warn', 5000)
        if (window.__pvDebug) window.__pvDebug(T('端口要在 1024~65535 之间'))
        return
      }
      portIn.disabled = true
      invoke('net_set_port', { port: p })
        .then((r) => {
          addrPort = p
          toast(T('端口已改为 {p}', { p }), 'ok', 3000)
          if (window.__pvDebug) window.__pvDebug(String(r))
        })
        .catch((e) => {
          portIn.value = String(addrPort)
          toast(T('改端口失败：') + String(e), 'error', 6000)
          if (window.__pvDebug) window.__pvDebug(T('改端口失败：') + String(e))
        })
        .finally(() => {
          portIn.disabled = false
          poll()
        })
    })
    addrBox.append(url, copyBtn, ifSel, portIn, portHint)
    box.appendChild(addrBox)

    /**
     * 重建网卡下拉的选项。
     *
     * **重建而不是增量改**：网卡随时会插拔（USB 共享、蓝牙共享、移动热点），两秒前还在
     * 的那张下一轮可能就没了，增量维护要处理一堆「上一轮有这一轮没有」的边界。整份重建
     * 只有五行，而且选项数是个位数，重建的成本可以忽略。
     */
    const fillIfaces = () => {
      // 每 500 ms 重建一次会把用户正打开的下拉弹回去。签名相同就直接跳过 —— 选项内容
      // 和选中项都没变，重建纯属自找麻烦。
      const sig =
        addrIp + '\u0000' + ifaces.map((n) => n.ip + '\u0000' + n.name + '\u0000' + n.desc).join('\u0001')
      if (ifSel.dataset.sig === sig) return
      ifSel.dataset.sig = sig
      ifSel.textContent = ''
      // 自动项永远在：用户选的那张可能已经拔了（`advertised_ip()` 已经回落到默认路由），
      // 这时得有个地方能一键回到自动
      const auto = document.createElement('option')
      auto.value = ''
      auto.textContent = T('自动（默认路由）')
      ifSel.appendChild(auto)
      for (const n of ifaces) {
        const o = document.createElement('option')
        o.value = n.ip
        // 名字带网卡全名，用户才认得出「这张是 USB 共享还是 Wi-Fi」
        const tag = n.desc && n.desc !== n.name ? n.name + ' · ' + n.desc : n.name
        o.textContent = n.ip + (tag ? '  ' + tag : '')
        ifSel.appendChild(o)
      }
      if (!ifaces.length) {
        const o = document.createElement('option')
        o.value = ''
        o.disabled = true
        o.textContent = T('没有可直连局域网的网卡')
        ifSel.appendChild(o)
      }
      ifSel.value = addrIp
    }

    // ---- 3) 二维码在左，**按钮在右边**（右边只放能点的东西） ----
    const qrBox = document.createElement('div')
    qrBox.className = 'netqr'
    const canvas = document.createElement('canvas')
    canvas.className = 'netqr-canvas'
    canvas.setAttribute('role', 'img')
    canvas.setAttribute('aria-label', T('客户端扫码打开网页版客户端'))

    const side = document.createElement('div')
    side.className = 'netqr-side'

    const sideActs = document.createElement('div')
    sideActs.className = 'netqr-acts'

    // 证书：网页版的麦克风靠 `getUserMedia`，它只在**安全上下文**里可用，所以必须走
    // https（NETWORK.md §4.1.12）。装一次 CA 之后手机就永久不再报警告。
    const caBtn = document.createElement('button')
    caBtn.type = 'button'
    caBtn.className = 'netcard-ca'
    caBtn.textContent = T('证书')
    caBtn.title = T('手机浏览器打开这个地址下载 CA 证书，装到系统里，之后扫码访问就不会再报警告了')
    caBtn.setAttribute('aria-label', T('下载证书（手机装一次）'))
    caBtn.addEventListener('click', async () => {
      caBtn.disabled = true
      try {
        const pem = await invoke('net_ca_pem')
        if (!pem) {
          toast(T('证书还没准备好：先启动服务'), 'warn', 5000)
          if (window.__pvDebug) window.__pvDebug(T('证书还没准备好：先启动服务'))
          return
        }
        // 用 Blob + a[download]：不经过服务端额外路由，证书内容直接来自 Rust 侧读的那份
        const u = URL.createObjectURL(new Blob([pem], { type: 'application/x-x509-ca-cert' }))
        const a = document.createElement('a')
        a.href = u
        a.download = 'purevox-ca.crt'
        a.click()
        URL.revokeObjectURL(u)
        toast(T('CA 证书已下载'), 'ok')
        if (window.__pvDebug) window.__pvDebug(T('CA 证书已下载'))
      } catch (e) {
        toast(T('取证书失败：') + String(e), 'error', 6000)
        if (window.__pvDebug) window.__pvDebug(T('取证书失败：') + String(e))
      } finally {
        caBtn.disabled = false
      }
    })

    // 远程输入开关：打字 + 全尺寸键盘。默认关闭（局域网无鉴权，DEBUG.md §1.2）。
    const riBox = document.createElement('label')
    riBox.className = 'en netcard-remote'
    const cb = document.createElement('input')
    cb.type = 'checkbox'
    cb.title = T('同网段无鉴权：开启后同一 Wi-Fi 下的任何设备都能向这台电脑打字和发按键')
    cb.addEventListener('change', () => {
      invoke('net_set_remote_input', { on: cb.checked })
        .then(() => {
          // 成功后再用下一轮 `net_card` 校准显示，避免「界面说开了其实没开」
          poll()
        })
        .catch((e) => {
          cb.checked = !cb.checked // 失败必须回滚，不能界面显示已开而实际是关
          toast(T('远程输入开关失败：') + String(e), 'error', 6000)
          if (window.__pvDebug) window.__pvDebug(T('远程输入开关失败：') + String(e))
        })
    })
    riBox.append(cb, document.createTextNode(T('远程输入')))

    sideActs.append(caBtn, riBox)

    // AEC 开关（**只有网络输入行**有意义：输出行没有「近端麦克风」）
    const aecHead = document.createElement('label')
    aecHead.className = 'en netcard-aec-head'
    const aecCb = document.createElement('input')
    aecCb.type = 'checkbox'
    aecCb.checked = row.params.aec === '1' || row.params.aec === 'true'
    aecCb.title = T('AEC 消除回声')
    aecHead.append(aecCb, document.createTextNode(T('AEC')))
    if (!out) sideActs.appendChild(aecHead)
    aecCb.addEventListener('change', () => {
      row.params.aec = aecCb.checked ? '1' : '0'
      // AEC 是结构性变更（模型要加载、far 采集要起停），必须重建会话
      if (typeof apply === 'function') apply()
      syncAec()
    })

    // 抖动缓冲（**手动**滑块，10~200 ms、10 ms 步进）：「缓冲只在接收方」是设计不变量，
    // 所以这个值由手机执行、电脑只负责设定与显示；手机那侧也有同一个滑块，谁改了另一边
    // 跟着变（协议 `settarget` ↔ `target`）。**不是自适应**：值只由用户拧。
    const ringWrap = document.createElement('div')
    ringWrap.className = 'netcard-ring'
    const ringIn = document.createElement('input')
    ringIn.type = 'range'
    ringIn.min = '10'
    ringIn.max = '200'
    ringIn.step = '10'
    ringIn.className = 'netcard-ring-range'
    ringIn.setAttribute(
      'aria-label',
      T('抖动缓冲（手机缓冲帧数，1 帧 = 10 ms）'),
    )
    ringIn.title = T('抖动缓冲（手机缓冲帧数，1 帧 = 10 ms）')
    const ringVal = document.createElement('span')
    ringVal.className = 'netcard-ring-val mono'
    ringWrap.append(nl(T('缓冲')), ringIn, ringVal)
    /** 用户正在拖滑块（拖动期间不许回绘把它拽回去）。**必须在 `paintRing` 之前声明**：
     *  它在下面第一次 `paintRing()` 里被读，声明在后面的 `let` 会直接 TDZ 抛错，
     *  整张卡都建不出来（2026-10-27 实测：`Cannot access 'ringDragging' before initialization`，
     *  结果是整个列区空白）。 */
    let ringDragging = false
    const paintRing = (ms) => {
      // 用户正在拖：别把滑块拽回去（读数照旧跟着手指）
      if (ringDragging) return
      ringIn.value = String(ms)
      ringVal.textContent = T('{ms} ms', { ms: String(ms) })
    }
    paintRing(ringTarget)
    ringPainters.add(() => {
      if (ringWrap.isConnected) paintRing(ringTarget)
    })
    ringIn.addEventListener('pointerdown', () => {
      ringDragging = true
    })
    ringIn.addEventListener('pointerup', () => {
      ringDragging = false
    })
    ringIn.addEventListener('input', () => {
      ringVal.textContent = T('{ms} ms', { ms: String(Number(ringIn.value)) })
    })
    ringIn.addEventListener('change', () => {
      const ms = Number(ringIn.value)
      invoke('net_set_ring_ms', { ms })
        .then((applied) => {
          paintRing(Number(applied) || ms)
          toast(T('抖动缓冲已改为 {ms} ms', { ms: String(applied) }), 'ok', 3000)
        })
        .catch((e) => {
          paintRing(ringTarget)
          toast(T('改抖动缓冲失败：') + String(e), 'error', 6000)
        })
    })

    const note = document.createElement('div')
    note.className = 'netqr-note'
    side.append(sideActs, ringWrap, note)
    qrBox.append(canvas, side)
    box.appendChild(qrBox)

    // ---- 4) AEC 控件（勾上才展开：远端来源 + 增益 / 延时 / 直通 + 校准 / 结果）----
    const aecBox = document.createElement('div')
    aecBox.className = 'netcard-aec'

    const farRow = document.createElement('div')
    farRow.className = 'netcard-far'
    farRow.appendChild(nl(T('远端')))
    const farSel = document.createElement('select')
    farSel.className = 'netcard-sel'
    farSel.setAttribute('aria-label', T('远端设备'))
    // 远端来源与**独立 AEC 行**同一份列表（`list_loopback_targets`：系统默认输出 +
    // 每只输出设备的回环），另外多一项「网络出站」—— 那是网络行独有的参考源。
    // 先填两个保底项，枚举回来再补：枚举要碰 cpal / pactl，慢一点不能挡住首屏。
    const fillFar = (targets) => {
      const keep = row.params.far_device || 'loopback'
      farSel.textContent = ''
      const opts = [
        ['loopback', T('系统默认输出')],
        ['network', T('网络出站（本机刚发出的音频）')],
      ]
      for (const t of targets || []) {
        const id = t && (t.id || t.device || t.name)
        if (!id || id === 'loopback' || id === 'network') continue
        opts.push([id, t.name || t.label || String(id)])
      }
      for (const [v, t] of opts) {
        const o = document.createElement('option')
        o.value = v
        o.textContent = t
        farSel.appendChild(o)
      }
      farSel.value = [...farSel.options].some((o) => o.value === keep) ? keep : 'loopback'
    }
    fillFar(null)
    invoke('list_loopback_targets').then(fillFar).catch(() => {})
    farSel.addEventListener('change', () => {
      row.params.far_device = farSel.value
      if (typeof apply === 'function') apply()
    })
    farRow.appendChild(farSel)
    aecBox.appendChild(farRow)

    // 增益 / 延时 / 直通：**与独立 `echo_cancel` 行完全同一份控件**（`columns.js` 的
    // 参数渲染器），范围 / 步进 / 单位都一致。复制一套到自己这里就是漂移的根源。
    // 明确排除 `aec`（开关在二维码右边）与 `far_device`（上面那只下拉），也不给
    // `model`（内置模型，与 AEC 行同一个，不给选）。
    aecBox.appendChild(
      window.__pvParams
        ? window.__pvParams.controls(row, ci, ri, [
            'mic_gain_db',
            'far_gain_db',
            'far_delay_ms',
            'bypass',
          ])
        : document.createTextNode(T('参数控件未就绪')),
    )

    const aecActs = document.createElement('div')
    aecActs.className = 'netcard-aec-acts'
    const calBtn = document.createElement('button')
    calBtn.type = 'button'
    calBtn.textContent = T('校准')
    calBtn.title = T('向远端送扫频探针，测出延时与两端电平并回填')
    calBtn.addEventListener('click', () => {
      calBtn.disabled = true
      calBtn.textContent = T('校准中')
      invoke('calibrate_aec_delay', { col: ci, row: ri })
        .then((t) => window.__pvDebug?.('开始校准：' + t))
        .catch((e) => {
          window.__pvToast?.(T('校准失败：') + String(e), 'error', 7000)
          window.__pvDebug?.('校准失败：' + e)
        })
        .finally(() => {
          window.setTimeout(() => {
            calBtn.disabled = false
            calBtn.textContent = T('校准')
          }, 1800)
        })
    })
    // 结果：打开最近一次校准的曲线 + 结论（与 AEC 行**同一个面板**，数据来自快照
    // `calib_report`，见 calib.js）。没有报告时禁用，不假装有结果。
    const resBtn = document.createElement('button')
    resBtn.type = 'button'
    resBtn.textContent = T('结果')
    resBtn.title = T('查看最近一次校准结果（含曲线与结论）')
    resBtn.addEventListener('click', () => window.__pvCalib?.open())
    aecActs.append(calBtn, resBtn)
    aecBox.appendChild(aecActs)
    if (!out) box.appendChild(aecBox)

    /** 按勾选状态显隐 AEC 控件（勾上才有意义，不勾时占地方也没信息） */
    function syncAec() {
      aecBox.classList.toggle('on', aecCb.checked)
    }
    syncAec()

    // ---- 重绘（2 Hz，DEBUG.md §1.1：不卡 UI）----
    const paint = () => {
      const c = card
      const running = !!c?.running
      bStart.disabled = running
      bStop.disabled = !running
      // 地址取不到时要说**为什么**：没网卡 / 没有默认路由，和「服务没跑」是两回事。
      // 地址取得到时照常显示（服务没跑就是灰的），用户据此在手机上先填好 —— 只是别给一张
      // 扫不通的码，见 `drawQr`。
      url.textContent = addrUrl || addrReason || T('服务未启动')
      url.className = 'mono netcard-url ' + (addrUrl && running ? '' : 'na')
      fillIfaces()
      // 端口框只在**用户没在改它**时同步，否则正在输入的字符会被每 500 ms 的轮询吃掉
      if (document.activeElement !== portIn && portIn.value !== String(addrPort)) {
        portIn.value = String(addrPort)
      }

      // 状态：**只给一句短的**，全文留在 `/debug` 的 `net` 段（`net_status` 那条长串
      // 不适合放进几行高的卡片）。每一段都来自 Rust 的结构化字段，前端不另算。
      const parts = []
      if (statusReason) parts.push(statusReason)
      else if (c) {
        parts.push(running ? T('就绪') : T('未启动'))
        if (c.clients) parts.push(T('{n} 客户端', { n: c.clients }))
        if (c.subs) parts.push(T('{n} 订阅', { n: c.subs }))
        if (!c.codec_ok) parts.push(T('编解码器不可用'))
      } else {
        parts.push(T('状态不可用'))
      }
      status.textContent = parts.join(' · ')
      // 灯也看服务在不在跑 + 编解码器是否可用（缺 opus 时网络功能是废的）
      const ok = running && !!c?.codec_ok && !statusReason
      dot.className = 'netdot ' + (ok ? 'ok' : 'na')
      // 细节（端口占用 / 后端 / 最近错误 / 水位 / 入站环）放进 tooltip，不占版面
      const tips = []
      tips.push(T('端口') + '：' + addrPort)
      if (c) {
        tips.push(T('客户端') + '：' + c.clients + '，' + T('订阅') + '：' + c.subs)
        tips.push(
          T('缓冲') + '：' +
            (c.rx_level_ms == null
              ? '—'
              : Math.round(c.rx_level_ms) + '/' + Math.round(c.rx_target_ms) + ' ms'),
        )
        if (c.keys_backend) tips.push(T('远程输入') + '：' + c.keys_backend)
        if (c.keys_reason) tips.push(T('远程输入') + '：' + c.keys_reason)
        if (!c.codec_ok && c.codec_reason) tips.push(T('编解码器不可用') + '：' + c.codec_reason)
        if (c.last_error) tips.push(c.last_error)
      }
      if (addrReason) tips.push(addrReason)
      status.title = tips.join('\n')

      // 端口是否被占用：从状态里读（失败原因是唯一事实来源，不另猜）
      // Rust 的 `codec_reason` 不区分端口；端口占用会体现在 `last_error` 里。
      const lastErr = (c && c.last_error) || ''
      const busyPort = /占用|in use|address already/i.test(lastErr)
      portHint.textContent = busyPort ? T('端口被占用') : ''
      portHint.className = 'netcard-port-hint ' + (busyPort ? 'bad' : '')

      // 远程输入：开关状态跟着 Rust 走（手机 / 托盘关掉时这里也要变）；后端不可用时
      // 说明为什么（禁用只是不让人白点，原因必须在 tooltip 里看得见）
      const riOn = !!c?.remote_input
      if (document.activeElement !== cb) cb.checked = riOn
      const keysOk = !(c && c.keys_reason)
      cb.disabled = !keysOk
      riBox.title = keysOk
        ? T('同网段无鉴权：开启后同一 Wi-Fi 下的任何设备都能向这台电脑打字和发按键')
        : T('远程输入在这个平台不可用：') + (c?.keys_reason || '')

      // 校准结果：有报告才让点
      resBtn.disabled = !window.__pvCalib?.hasReport?.()

      drawQr(qrBox)
    }

    // 节点只 append 一次；顺序就是最终的视觉顺序（不要先 append 再移动）
    elRow.appendChild(box)
    cards.add({ el: elRow, paint })
    paint()
    if (timer === null) timer = window.setInterval(poll, 500)
    return true
  }

  /** 重绘整张计划前调用（`columns.js` 的 render 会重建 DOM） */
  function reset() {
    cards.clear()
    ringPainters.clear()
    if (timer !== null) {
      window.clearInterval(timer)
      timer = null
    }
  }

  window.addEventListener('DOMContentLoaded', () => {
    poll()
    document.addEventListener('pv-langchange', () => {
      for (const c of cards) c.paint()
    })
    window.addEventListener('beforeunload', reset)
  })

  window.__pvNetRow = { attach, reset }
})()
