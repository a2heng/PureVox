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
// **为什么要单独一个文件**：网络行不需要选设备，光一个下拉框等于什么都没说——用户
// 建完这行不知道下一步干什么。真正的操作全在这里：服务开关/重启、状态灯、扫码用的
// 网址与二维码、这行到底干什么的说明。与 `media.js`（音效板 / 播放器）同理，由
// 本模块独占这些控件，`columns.js` 不再画第二遍。
//
// 与顶栏「手机」面板读的是同一份状态（`net_status` / `net_running` / `net_local_addr`，
// 都是 `/debug` 的 `net` 段同源）。网络行卡片是**唯一**的手机/客户端操作入口：顶栏
// 不再有独立的「手机」面板（用户 2026-10-12 明确要求——「手机不要单独的面板，就是
// 这个网络面板」），没建网络行时想连客户端就只能先把行加上，这是有意的。

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

  /** 当前地址（`http://ip:port/`），失败给原因。同一份数据给顶栏面板与这里的二维码。 */
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
  /** 服务是否在跑。**别去 `net_status` 的中文串里匹配** —— 那串会改文案。 */
  let running = false
  /** 取不到地址的原因（没网卡 / 没默认路由）。空 = 取到了。 */
  let addrReason = ''
  /** @type {{text: string, ok: boolean}|null} */
  let lastStatus = null
  let remoteInput = false
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
    let ok = false
    let text = ''
    try {
      const [s, a, run, ifs, ring] = await Promise.all([
        invoke('net_status'),
        invoke('net_local_addr'),
        invoke('net_running'),
        invoke('net_ifaces'),
        invoke('net_ring_ms'),
      ])
      if (Number.isFinite(Number(ring))) ringTarget = Number(ring)
      ok = true
      text = String(s)
      addrUrl = a && a.url ? String(a.url) : ''
      addrIp = a && a.ip ? String(a.ip) : ''
      addrReason = a && a.reason ? String(a.reason) : ''
      if (a && a.port) addrPort = Number(a.port)
      running = !!run
      if (Array.isArray(ifs)) ifaces = ifs
    } catch (e) {
      text = T('状态不可用：') + String(e)
      addrUrl = ''
      addrIp = ''
      addrReason = String(e)
      running = false
    }
    lastStatus = { text, ok }
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
    // 服务没跑就不给码：地址是能算出来的，但扫过去连不上。与其给一张必然失败的码，不如
    // 说清楚为什么没码（AGENTS.md §1.1：不显示伪造 / 无效的值）。
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
      if (note) note.textContent = T('客户端扫码打开网页版客户端')
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
   * AEC 的数值参数。改完立刻 apply（重建会话）——这些数都参与建行，跑起来就固定了。
   */
  function netNum(row, key, min, max, step, apply) {
    const i = document.createElement('input')
    i.type = 'number'
    i.min = String(min)
    i.max = String(max)
    i.step = String(step)
    i.style.width = '4.5em'
    i.value = row.params[key] ?? '0'
    i.addEventListener('change', () => {
      row.params[key] = i.value
      if (typeof apply === 'function') apply()
    })
    return i
  }

  /**
   * @param {HTMLElement} elRow 行容器（`columns.js` 传进来）
   * @param {number} _ci @param {number} _ri @param {any} row
   * @param {{apply?: () => void}} [hooks]
   */
  function attach(elRow, _ci, _ri, row, hooks) {
    const apply = hooks && hooks.apply
    const out = row.ptype === 'remote_speaker'

    // 网络行卡片。**手机相关的全部操作都在这里**——顶栏不再有独立面板（用户明确要求：
    // 「手机不要单独的面板，就是这个网络面板」）。这一行既是链路的一环，也是客户端
    // 的说明书：状态、地址、二维码、重启、远程输入、端口，缺一样用户就接不上。
    //
    // 「手机」这个词只在指代真机时出现；客户端本身可能是手机、平板或浏览器网页版，
    // 所以一律叫「客户端」（见 AGENTS.md §1.2：远程音频与远程输入是**网络**功能）。
    const box = document.createElement('div')
    box.className = 'netcard'

    // 1) 状态灯 + 状态串（与 `/debug` 的 `net` 段同源；失败给原因，不显示伪造的 0）
    const bar = document.createElement('div')
    bar.className = 'netcard-bar'
    const dot = document.createElement('span')
    dot.className = 'netdot'
    const status = document.createElement('span')
    status.className = 'netcard-status'
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
    // 不会因为服务起停而重排，界面不跳（AGENTS.md §1.1 的固定行数刷新同理）。
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

    // 2) 客户端地址 + 复制。二维码的内容就是它，两边永远一致（同一个 `addrUrl`）。
    const addrBox = document.createElement('div')
    addrBox.className = 'netcard-addr'
    const lab = document.createElement('span')
    lab.className = 'netcard-label'
    lab.textContent = T('客户端地址')
    const url = document.createElement('span')
    url.className = 'mono netcard-url'
    const copyBtn = document.createElement('button')
    copyBtn.type = 'button'
    copyBtn.className = 'icon'
    copyBtn.textContent = T('复制')
    copyBtn.title = T('复制客户端地址')
    copyBtn.addEventListener('click', () => copy(addrUrl, copyBtn))
    addrBox.append(lab, url, copyBtn)
    box.appendChild(addrBox)

    // 2b) 网卡下拉 + 端口框。**换网卡不用重启服务**（服务绑 0.0.0.0，本来就监听所有
    //     网卡），换端口必须重启 —— 所以这两件事的行为不一样，提示文字也分开写。
    const netBox = document.createElement('div')
    netBox.className = 'netcard-addr'
    netBox.appendChild(nl(T('网卡')))
    const ifSel = document.createElement('select')
    ifSel.className = 'netcard-sel netcard-ifsel'
    ifSel.title = T('只决定二维码 / 复制的是哪个地址，服务不用重启（本来就在监听所有网卡）')
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
    netBox.appendChild(ifSel)
    netBox.appendChild(nl(T('端口')))
    const portIn = document.createElement('input')
    portIn.type = 'number'
    portIn.min = '1024'
    portIn.max = '65535'
    portIn.step = '1'
    portIn.className = 'netcard-port mono'
    portIn.title = T('改完自动重启服务；已连着的客户端需要重新连')
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
    // 4) 端口：**单独一行**，右边常驻一行「是否被占用」的结论（不静默）。
    //    原来把它塞在「网卡」那一行里，标签被裁成「端口」两个字、还有一半看不见。
    const portRow = document.createElement('div')
    portRow.className = 'netcard-addr'
    portRow.appendChild(nl(T('端口')))
    portRow.appendChild(portIn)
    const portHint = document.createElement('span')
    portHint.className = 'netcard-port-hint'
    portRow.appendChild(portHint)
    // 只由网卡/端口那一行承载：


    // 2c) 抖动缓冲深度（**手动**滑块，10~200 ms、10 ms 步进）。
    //     「缓冲只在接收方」是设计不变量，所以这个值由手机执行、电脑只负责设定与显示；
    //     手机那侧也有同一个滑块，谁改了另一边跟着变（协议 `settarget` ↔ `target`）。
    //     这不是自适应：值只由用户拧，运行期不会因为抖动自己动。
    const ringWrap = document.createElement('div')
    ringWrap.className = 'netcard-ring'
    const ringLabel = document.createElement('div')
    ringLabel.className = 'netcard-ring-label'
    const ringIn = document.createElement('input')
    ringIn.type = 'range'
    ringIn.min = '10'
    ringIn.max = '200'
    ringIn.step = '10'
    ringIn.className = 'netcard-ring-range'
    const ringVal = document.createElement('span')
    ringVal.className = 'netcard-ring-val mono'
    const ringMs = () => ringTarget
    /** 用户正在拖滑块（拖动期间不许回绘把它拽回去）。**必须在 `paintRing` 之前声明**：
     *  它在下面第一次 `paintRing()` 里被读，声明在后面的 `let` 会直接 TDZ 抛错，
     *  整张卡都建不出来（2026-10-27 实测：`Cannot access 'ringDragging' before initialization`，
     *  结果是整个列区空白）。 */
    let ringDragging = false
    const paintRing = (ms) => {
      ringLabel.textContent = T('抖动缓冲（手机缓冲帧数，1 帧 = 10 ms）')
      // 用户正在拖：别把滑块拽回去（读数照旧跟着手指）
      if (ringDragging) return
      ringIn.value = String(ms)
      ringVal.textContent = T('{f} 帧 / {ms} ms', { f: String(ms / 10), ms: String(ms) })
    }
    paintRing(ringMs())
    ringPainters.add(() => {
      if (ringWrap.isConnected) paintRing(ringMs())
    })

    // 拖动时只更新读数；抬手才下发（避免每 10 ms 打一次 IPC 把命令队列灌满）
    ringIn.addEventListener('pointerdown', () => {
      ringDragging = true
    })
    ringIn.addEventListener('pointerup', () => {
      ringDragging = false
    })
    ringIn.addEventListener('input', () => {
      ringVal.textContent = T('{f} 帧 / {ms} ms', {
        f: String(Number(ringIn.value) / 10),
        ms: String(Number(ringIn.value)),
      })
    })
    ringIn.addEventListener('change', () => {
      const ms = Number(ringIn.value)
      invoke('net_set_ring_ms', { ms })
        .then((applied) => {
          paintRing(Number(applied) || ms)
          toast(T('抖动缓冲已改为 {ms} ms', { ms: String(applied) }), 'ok', 3000)
        })
        .catch((e) => {
          paintRing(ringMs())
          toast(T('改抖动缓冲失败：') + String(e), 'error', 6000)
        })
    })
    ringWrap.appendChild(ringLabel)
    ringWrap.appendChild(ringIn)
    ringWrap.appendChild(ringVal)
    netBox.appendChild(ringWrap)

    box.appendChild(netBox)

    // 3) 装 CA 证书的按钮。网页版的麦克风靠 `getUserMedia`，它只在**安全上下文**里可用，
    //    所以必须走 https（DESIGN.md §4.1.11）。装一次 CA 之后手机就永久不再报警。
    //    按钮放在这里而不是「设置」里，因为它和地址、端口是同一件事：地址决定去哪台机器。
    const caBtn = document.createElement('button')
    caBtn.type = 'button'
    caBtn.className = 'netcard-ca'
    caBtn.textContent = T('下载证书（手机装一次）')
    caBtn.title = T(
      '手机浏览器打开这个地址下载 CA 证书，装到系统里，之后扫码访问就不会再报警告了',
    )
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
        const url = URL.createObjectURL(new Blob([pem], { type: 'application/x-x509-ca-cert' }))
        const a = document.createElement('a')
        a.href = url
        a.download = 'purevox-ca.crt'
        a.click()
        URL.revokeObjectURL(url)
        toast(T('CA 证书已下载'), 'ok')
        if (window.__pvDebug) window.__pvDebug(T('CA 证书已下载'))
      } catch (e) {
        toast(T('取证书失败：') + String(e), 'error', 6000)
        if (window.__pvDebug) window.__pvDebug(T('取证书失败：') + String(e))
      } finally {
        caBtn.disabled = false
      }
    })
    box.appendChild(caBtn)

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
      const sig = addrIp + ' ' + ifaces.map((n) => n.ip + ' ' + n.name + ' ' + n.desc).join('')
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

    // 3) 二维码在左，**说明文字在二维码右边**（一栏竖排，不散在卡片各处）。
    //    下面再放远程输入开关——高度由内容决定，不留空白。
    const qrBox = document.createElement('div')
    qrBox.className = 'netqr'
    const canvas = document.createElement('canvas')
    canvas.className = 'netqr-canvas'
    canvas.setAttribute('role', 'img')
    const side = document.createElement('div')
    side.className = 'netqr-side'
    const note = document.createElement('div')
    note.className = 'netqr-note'
    const help = document.createElement('div')
    help.className = 'netcard-help'
    help.textContent = out
      ? T('网络扬声器：把处理后的声音 Opus 编码后发给所有已订阅的客户端。客户端连上并订阅后就有声音。')
      : T('网络麦克风：远端设备的麦克风当电脑麦克风用，照样进降噪 / TSE / AEC 全链。客户端需连接上面的地址。')
    side.append(note, help)
    qrBox.append(canvas, side)
    box.appendChild(qrBox)

    // 5) 远程输入开关：打字 + 全尺寸键盘。默认关闭（局域网无鉴权，AGENTS.md §1.2）。
    const ri = document.createElement('label')
    ri.className = 'en netcard-remote'
    const cb = document.createElement('input')
    cb.type = 'checkbox'
    cb.addEventListener('change', () => {
      invoke('net_set_remote_input', { on: cb.checked })
        .then(() => {
          remoteInput = cb.checked
        })
        .catch((e) => {
          cb.checked = remoteInput // 失败必须回滚，不能界面显示已开而实际是关
          toast(T('远程输入开关失败：') + String(e), 'error', 6000)
          if (window.__pvDebug) window.__pvDebug(T('远程输入开关失败：') + String(e))
        })
    })
    ri.append(cb, document.createTextNode(T('远程输入（打字 + 键盘，默认关闭）')))
    ri.title = T('同网段无鉴权：开启后同一 Wi-Fi 下的任何设备都能向这台电脑打字和发按键')
    box.appendChild(ri)

    /** 重绘状态相关的部分（2 Hz，AGENTS.md §1.1：不卡 UI） */
    const paint = () => {
      const s = lastStatus
      if (s) {
        status.textContent = s.text
        // 灯也看服务在不在跑：服务停了但状态查询成功时，不能还是绿的
        dot.className = 'netdot ' + (s.ok && running ? 'ok' : 'na')
      }
      // 启停按钮按状态禁用（不隐藏，避免卡片重排）
      bStart.disabled = running
      bStop.disabled = !running
      // 地址取不到时要说**为什么**：没网卡 / 没有默认路由，和「服务没跑」是两回事，都写
      // 成「服务没启动」会让人一直去点启动按钮（AGENTS.md §1.1）。
      // 地址取得到时照常显示（服务没跑就是灰的），用户据此在手机上先填好 —— 只是别给一张
      // 扫不通的码，见 `drawQr`。
      url.textContent = addrUrl || addrReason || T('服务未启动')
      url.className = 'mono netcard-url ' + (addrUrl && running ? '' : 'na')
      fillIfaces()
      // 端口框只在**用户没在改它**时同步，否则正在输入的字符会被每 500 ms 的轮询吃掉
      if (document.activeElement !== portIn && portIn.value !== String(addrPort)) {
        portIn.value = String(addrPort)
      }
      // 端口是否被占用：从状态串里读（失败原因是唯一事实来源，不另猜）
      const st = (lastStatus && lastStatus.text) || ''
      const busy = /占用|in use|Address already/i.test(st)
      portHint.textContent = busy ? T('端口被占用') : running ? T('监听中') : ''
      portHint.className = 'netcard-port-hint ' + (busy ? 'bad' : 'ok')
      drawQr(qrBox)
    }

    // 5) AEC 复选框。网络麦克风**近端锁死是网络**（卡片上不可改 —— 这就是这张卡的
    //    意义），勾上后才长出完整 AEC 控件：远端来源 / 近端·远端增益 / 延时 / 校准 /
    //    直通。远端默认「系统默认输出回环」：PC 自己外放最常见，也最容易用
    //    VB-CABLE 之类的虚拟设备搭出来验证。
    // 复选框**必须常驻可见** —— 它不能放在下面那个默认 `display:none` 的容器里，
    // 否则用户连勾都勾不上（我第一版就是这么写的，界面上什么都看不到）。
    const aecHead = document.createElement('label')
    aecHead.className = 'en netcard-aec-head'
    const aecCb = document.createElement('input')
    aecCb.type = 'checkbox'
    aecCb.checked = row.params.aec === '1'
    aecHead.append(aecCb, document.createTextNode(T('AEC 消除回声')))
    // **只有网络输入行**才有 AEC（输出行没有「近端麦克风」这个概念）
    if (!out) box.appendChild(aecHead)

    aecCb.addEventListener('change', () => {
      row.params.aec = aecCb.checked ? '1' : '0'
      // AEC 是结构性变更（模型要加载、far 采集要起停），必须重建会话
      if (typeof apply === 'function') apply()
    })

    // 控件区：默认收起，勾上才展开。
    const aecBox = document.createElement('div')
    aecBox.className = 'netcard-aec'

    const near = document.createElement('div')
    near.className = 'netcard-far'
    near.appendChild(nl(T('近端')))
    // 近端锁死：网络麦克风本来就是网络输入，没有别的可选
    const nearLocked = document.createElement('span')
    nearLocked.className = 'netcard-locked'
    nearLocked.textContent = T('网络麦克风（不可更改）')
    near.appendChild(nearLocked)
    near.appendChild(nl(T('远端')))
    const farSel = document.createElement('select')
    farSel.className = 'netcard-sel'
    // 远端来源与**独立 AEC 行**同一份列表（`list_loopback_targets`：系统默认输出 +
    // 每只输出设备的回环），另外多一项「网络出站」—— 那是网络行独有的参考源。
    // 先填两个保底项，枚举回来再补：枚举要碰 cpal / pactl，慢一点不能挡住首屏。
    const fillFar = (targets) => {
      const keep = farSel.value
      farSel.textContent = ''
      const opts = [['loopback', T('系统默认输出')],
                    ['network', T('网络出站（本机刚发出的音频）')]]
      for (const t of targets || []) {
        const id = t && (t.id || t.device || t.name)
        if (!id || id === 'loopback') continue
        opts.push([id, t.name || t.label || String(id)])
      }
      for (const [v, t] of opts) {
        const o = document.createElement('option')
        o.value = v
        o.textContent = t
        farSel.appendChild(o)
      }
      if (!row.params.far_device) row.params.far_device = 'loopback'
      farSel.value = keep && [...farSel.options].some((o) => o.value === keep)
        ? keep
        : row.params.far_device || 'loopback'
    }
    fillFar(null)
    invoke('list_loopback_targets').then(fillFar).catch(() => {})
    farSel.addEventListener('change', () => {
      row.params.far_device = farSel.value
      if (typeof apply === 'function') apply()
    })
    near.appendChild(farSel)
    aecBox.appendChild(near)

    const gains = document.createElement('div')
    gains.className = 'netcard-far'
    // **与独立 `echo_cancel` 行完全同一份控件**：直接调 `columns.js` 的渲染器，
    // 用一个 ptype=echo_cancel 的代理行、`params` 指向本行 —— 于是模型下拉、远端设备、
    // 近端/远端增益滑块、直通、有符号远端延时，逐项同构，范围/步进/单位都一致。
    // 复制一套到自己这里就是漂移的根源（模型缺项、范围不一致都是这么来的）。
    const aecProxy = { ptype: 'echo_cancel', params: row.params }
    gains.appendChild(
      window.__pvParams
        ? window.__pvParams.controls(aecProxy, _ci, _ri)
        : document.createTextNode(T('参数控件未就绪')),
    )
    // 校准：与独立行**同一个面板**（`calib.js`），结果展示也就自然一致
    const cal = document.createElement('button')
    cal.type = 'button'
    cal.textContent = T('校准')
    cal.title = T('向远端送扫频探针，测出延时与两端电平并回填')
    cal.addEventListener('click', () => {
      cal.disabled = true
      invoke('calibrate_aec_delay')
        .then((t) => window.__pvDebug?.('开始校准：' + t))
        .catch((e) => {
          window.__pvToast?.(T('校准失败：') + String(e), 'error', 7000)
          cal.disabled = false
        })
        .finally(() => {
          window.setTimeout(() => {
            cal.disabled = false
          }, 1500)
        })
    })
    gains.appendChild(cal)

    aecBox.appendChild(gains)

    const aecHelp = document.createElement('p')
    aecHelp.className = 'netcard-help'
    aecHelp.textContent = T('勾上后这台网络的麦克风会先过 AEC：远端取哪一路由上面选。近端永远是这张卡的网络输入，不能改。')
    aecBox.appendChild(aecHelp)
    if (!out) box.appendChild(aecBox)

    /** 按勾选状态显隐 AEC 控件（勾上才有意义，不勾时占地方也没信息） */
    const syncAec = () => { aecBox.classList.toggle('on', aecCb.checked) }
    aecCb.addEventListener('change', syncAec)
    syncAec()

    /*
     * 八行顺序（用户 2026-10-27 指定，两张卡完全一致，只有第 8 行是输入行独有）：
     *   1) 启动/停止/重启 + 状态
     *   2) 网卡
     *   3) 客户端地址（IP:端口 + 复制）
     *   4) 端口 + 是否被占用
     *   5) 抖动缓冲（帧 = 10 ms）
     *   6) 二维码 + 右侧说明
     *   7) 远程输入
     *   8) AEC（**仅输入**）
     * 用 appendChild 重排既有节点：节点只会被「移动」，不会重建，监听器全都保留。
     */
    box.appendChild(bar)
    box.appendChild(netBox)
    box.appendChild(addrBox)
    box.appendChild(portRow)
    box.appendChild(ringWrap)
    box.appendChild(qrBox)
    box.appendChild(ri)
    if (!out) {
      box.appendChild(aecHead)
      box.appendChild(aecBox)
    }
    box.appendChild(caBtn)

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