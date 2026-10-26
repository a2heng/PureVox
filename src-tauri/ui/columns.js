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

// 音频列编辑器（DESIGN.md §3、§5）：编辑会话计划并应用。
// 每列自上而下是「固定输入行 + 可增删/拖动的中间行 + 固定输出行」；列与列独立。
// 任何结构性改动都会重建会话（当前阶段的取舍：应用即重启）。
;(() => {
const { invoke } = window.__TAURI__.core
const $ = (id) => document.getElementById(id)
const T = (s, vars) => (window.__pvT ? window.__pvT(s, vars) : s)

let plan = null
/** 上一次**成功应用**的计划（失败回滚用；否则界面显示新设备、引擎还在旧设备）。 */
let appliedPlan = null
const clonePlan = (p) =>
  typeof structuredClone === 'function' ? structuredClone(p) : JSON.parse(JSON.stringify(p))
let NODES = []
/** ptype → 可选模型列表（每个处理节点类型各自一份） */
let MODELS = {}
let devices = { inputs: [], outputs: [] }
/** AEC 远端可回环的目标（默认输出 + 各输出/sink 的 monitor），由 `list_loopback_targets` 填充 */
let loopTargets = []
let deviceStamp = null
let lastRec = ''
let lastCalib = ''
let running = false

function renderRunButton() {
  const b = $('btn-run')
  if (!b) return
  // 按「动作」上色：显示「启动」= 绿，显示「停止」= 红
  b.textContent = running ? T('停止') : T('启动')
  b.setAttribute('aria-label', running ? T('停止') : T('启动'))
  b.className = 'sidebar-run ' + (running ? 'run-stop' : 'run-start')
}

/** 行类型名（msgids 必须内联在 T() 里，i18n_lint 才能查缺键） */
const kindName = (kind) => T(kind === 'input' ? '输入' : kind === 'process' ? '处理' : '输出')

// 行右上角的图标（线性图标，统一 24 视箱 / 1.8 描边，由 CSS 上色）。以前用 ↑ ↓ ✕ 三个
// 文本字符：基线不齐、叉号还比箭头细一圈，看着很脏。
const ICON_UP = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 14.5l6-6 6 6"/></svg>'
const ICON_DOWN = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9.5l6 6 6-6"/></svg>'
const ICON_X = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7L7 17"/></svg>'
const ICON_LOCK = '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="5.5" y="10.5" width="13" height="9" rx="2"/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5"/></svg>'
const PROCESS_ORDER = ['denoise', 'tse', 'gain', 'noise_gate', 'compressor', 'limiter', 'expander', 'upward_compressor', 'eq3', 'invert']
/** 输入行的展示顺序：麦克风 → 网络 → 扬声器 → AEC → 测试音（用户 2026-10-26 定的顺序）。
 *  注册表里的顺序是历史上「后加的排后面」，界面按常用度重排；**只是排列**，
 *  不改任何 ptype 语义（默认行仍取 `audio_input`）。输出行注册表顺序即 扬声器 → 网络。 */
const INPUT_ORDER = ['audio_input', 'remote_mic', 'loopback', 'echo_cancel', 'tone']
const nodesOf = (kind) => {
  const list = NODES.filter((n) => n.kind === kind)
  const order = kind === 'process' ? PROCESS_ORDER : kind === 'input' ? INPUT_ORDER : null
  if (!order) return list
  return list.slice().sort((a, b) => {
    const ia = order.indexOf(a.ptype)
    const ib = order.indexOf(b.ptype)
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib)
  })
}
const specOf = (ptype) => NODES.find((n) => n.ptype === ptype)

/**
 * 这一行要不要选设备。
 * 不需要设备的：`tone`（合成的测试音）、`remote_mic` / `remote_speaker`（网络，
 * 与声卡无关 —— 网络麦克风直接进引擎，不需要虚拟声卡驱动）。
 */
const DEVICE_LESS = new Set(['tone', 'remote_mic', 'remote_speaker'])
const needsDevice = (kind, ptype) =>
  kind === 'output' ? !DEVICE_LESS.has(ptype) : kind === 'input' ? !DEVICE_LESS.has(ptype) : false

/**
 * `loopback` 输入行选的不是采集设备，而是**被回环的输出端点**（`session.rs` 按
 * `loopback` / `loopback:<端点ID｜sink 名>` 解析），所以走 `farOptions` 那套
 * 回环目标，不能塞输入设备列表——塞进去后端会拿输入设备 ID 当渲染端点打不开。
 */
const isLoopbackRow = (kind, ptype) => kind === 'input' && ptype === 'loopback'

function lbl(text) {
  const s = document.createElement('span')
  s.className = 'lbl'
  s.textContent = text
  return s
}

/** 数字参数输入框（写回 row.params[key] 并应用）。 */
function numParam(row, key, min, max, step, widthEm) {
  const input = document.createElement('input')
  input.type = 'number'
  input.min = min
  input.max = max
  input.step = step
  input.style.width = widthEm + 'em'
  // input[type=number] 不接受 "+22" 这类带正号的字符串，会显示为空 → 统一转成纯数字
  const n = Number(row.params[key])
  input.value = Number.isFinite(n) ? String(n) : '0'
  input.addEventListener('change', () => {
    row.params[key] = input.value
    apply()
  })
  return input
}

function defaultRow(kind) {
  const spec = nodesOf(kind)[0]
  return { kind, ptype: spec ? spec.ptype : '', device: null, enabled: true, params: {} }
}

function defaultColumn() {
  return { rows: [defaultRow('input'), defaultRow('output')] }
}

// 状态串是动态写入的（非 data-i18n 静态元素）：记录最近一次结果，切语言时按当前
// 语言重放（paintStatus 由 pv-langchange 调用）；null = 尚未 apply 过（保持空 span）。
let lastProblems = null
/** 这次状态是「信息」而不是「应用问题」：设备刷新/正在应用等不要显示成「N 个问题」。 */
let lastStatusInfo = false
function setStatus(problems, info = false) {
  lastProblems = problems
  lastStatusInfo = info
  paintStatus()
}
function paintStatus() {
  if (lastProblems === null) return
  const el = $('plan-status')
  if (lastProblems && lastProblems.length && lastStatusInfo) {
    el.textContent = lastProblems.join('；')
    el.title = ''
    el.className = 'mono status-chip'
    el.onclick = null
    el.onkeydown = null
    el.removeAttribute('role')
    el.removeAttribute('tabindex')
    el.style.cursor = ''
    return
  }
  if (!lastProblems || lastProblems.length === 0) {
    el.textContent = T('已应用')
    el.title = ''
    el.className = 'mono status-chip ok'
    el.onclick = null
    el.onkeydown = null
    el.removeAttribute('role')
    el.removeAttribute('tabindex')
    el.style.cursor = ''
  } else {
    // 顶部只放一个短标签：完整的模型/设备错误动辄几百字，塞进页头会把标题挤成
    // 竖排（2026-10-25 切换输出设备时实测）。全文进 title + 点击后的 Toast。
    el.textContent = lastProblems.length === 1 ? T('1 个问题') : T('{n} 个问题', { n: lastProblems.length })
    el.title = lastProblems.join('\n')
    el.className = 'mono status-chip na'
    el.style.cursor = 'pointer'
    el.setAttribute('role', 'button')
    el.tabIndex = 0
    el.onclick = () => window.__pvShell?.showProblems?.(lastProblems)
    el.onkeydown = (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') {
        ev.preventDefault()
        window.__pvShell?.showProblems?.(lastProblems)
      }
    }
  }
}

function apply() {
  invoke('apply_plan', { plan })
    .then((/** @type {string[]} */ problems) => {
      appliedPlan = clonePlan(plan)
      setStatus(problems)
      if (problems && problems.length) {
        window.__pvToast?.(problems.join('；'), 'warn')
        window.__pvDebug?.('应用计划有问题：' + problems.join('；'))
      }
    })
    .catch(async (e) => {
      // 关键失败时 Rust **保留旧会话**；界面必须回到引擎真正在用的计划，
      // 否则会显示 R20、调试面板却还是 USB Audio（2026-10-25 用户实测）。
      try {
        const actual = /** @type {any} */ (await invoke('get_plan'))
        if (actual && actual.columns) {
          plan = actual
          appliedPlan = clonePlan(actual)
          render()
        }
      } catch (_) {
        if (appliedPlan) {
          plan = clonePlan(appliedPlan)
          render()
        }
      }
      setStatus([String(e)])
      window.__pvToast?.(T('应用计划失败：') + e, 'error', 7000)
      window.__pvDebug?.('应用计划失败：' + e)
    })
}

// ---------- 控件 ----------
function select(options, value, onChange, enabled = true) {
  const sel = document.createElement('select')
  for (const [val, label] of options) {
    const o = new Option(label, val)
    if (val === value) o.selected = true
    sel.add(o)
  }
  sel.disabled = !enabled
  sel.addEventListener('change', () => onChange(sel.value))
  return sel
}

function deviceOptions(kind, selectedId) {
  const list = kind === 'input' ? devices.inputs : devices.outputs
  const opts = [['', T('未选择设备')]].concat(
    list.map((d) => [d.id, (d.is_default ? '★ ' : '') + d.name]))
  if (selectedId && !opts.some((o) => o[0] === selectedId)) {
    opts.push([selectedId, T('设备不在')])
  }
  return opts
}

/**
 * 老配置里存的是**裸**端点 ID（`wasapi:...` / sink 名），没有 `loopback:` 前缀——
 * 后端 `strip_prefix("loopback:").unwrap_or(d)` 照样认得，但界面对不上选项，
 * 会误显示成「设备不在」。这里补上前缀，旧配置就能重新显示成正常条目。
 */
function normalizeLoopback(v) {
  if (!v || v.startsWith('loopback')) return v || ''
  return 'loopback:' + v
}

function loopbackOptions(selected) {
  // `loopback` 输入行选的不是采集设备，而是**被回环的输出端点**；值形如
  // `loopback`（系统默认输出）/ `loopback:<端点ID｜sink 名>`，与 AEC 远端同一套
  // 取值规则（后端 `list_loopback_targets` 给出后者），故直接复用 `farOptions`。
  return farOptions(normalizeLoopback(selected))
}

function farOptions(selected) {
  // 远端 = 输出/sink 的监视回环（`loopback` = 系统默认输出，`loopback:<sink>` = 指定），
  // 由后端 `list_loopback_targets` 给出（Linux = PipeWire sinks；Windows = WASAPI 端点）。
  // 不再提供「未选择」：AEC 的远端就是它要消除的那只输出，缺省 = 系统默认输出。
  const opts = [['loopback', T('系统默认输出')]].concat(
    loopTargets.map((t) => [t.id, '🔁 ' + t.name]))
  if (selected && !opts.some((o) => o[0] === selected)) {
    opts.push([selected, T('设备不在')])
  }
  return opts
}

// 处理行的参数控件：`keys` 限定只渲染部分参数（TSE 分两行摆）；model 用型号下拉，其余文本框
//
// **热参数**（注册表标了 `hot`：增益 / 音量）走 `set_row_param`：拖/敲就实时生效，
// **不重建会话**（重建要关掉全部设备再重开，Linux 上几百毫秒起步）。其余的仍然走
// `apply_plan` 全量重建 —— 哪些算热参数由后端注册表说了算（`registry::is_hot`），
// 界面不自己判。
/** OBS 参数单位（与 OBS 的 suffix 对齐；标签里已经带单位的就不重复）。 */
function paramUnit(key, label) {
  if (/dB|ms|Hz|:1/.test(label)) return ''
  if (['threshold', 'open_threshold', 'close_threshold', 'output_gain', 'gain_db', 'volume_db', 'low', 'mid', 'high', 'knee'].includes(key)) return 'dB'
  if (key.endsWith('_time') || key.endsWith('_ms')) return 'ms'
  if (key === 'ratio') return ':1'
  return ''
}

function paramControls(row, ci, ri, keys) {
  const spec = specOf(row.ptype)
  const box = document.createElement('span')
  box.className = 'params'
  if (!spec) return box

  /** 写参数：hot 参数下一拍生效，其它参数抬手（change）时重建会话。 */
  const commit = (p, value, hot) => {
    row.params[p.key] = value
    if (hot) {
      invoke('set_row_param', { col: ci, row: ri, key: p.key, value }).catch((e) => {
        setStatus([String(e)])
        window.__pvDebug?.('热参数失败：' + e)
      })
    } else {
      apply()
    }
  }

  for (const p of spec.params) {
    if (keys && !keys.includes(p.key)) continue
    const item = document.createElement('span')
    item.className = 'param'
    if (p.key === 'model' || p.key === 'reference' || p.key === 'path' || p.key === 'pads' || p.kind === 'choice') {
      item.classList.add('wide')
    }
    const label = document.createElement('span')
    label.className = 'plabel'
    label.textContent = T(p.label)
    item.appendChild(label)

    if (p.kind === 'choice') {
      const opts = (p.options || []).map(([v, l]) => [v, T(l)])
      item.appendChild(select(opts, row.params[p.key] ?? p.default, (v) => commit(p, v, false)))
    } else if (p.key === 'model') {
      const opts = (MODELS[row.ptype] ?? []).map((m) => [m.file, T(m.label)])
      item.appendChild(select(opts, row.params.model ?? p.default, (v) => commit(p, v, false)))
    } else if (p.kind === 'number' && p.min != null && p.max != null) {
      const value = Number(row.params[p.key] ?? p.default)
      const range = document.createElement('input')
      range.type = 'range'
      range.min = String(p.min)
      range.max = String(p.max)
      range.step = String(p.step ?? 1)
      range.value = Number.isFinite(value) ? String(value) : String(p.default)
      const num = document.createElement('input')
      num.type = 'number'
      num.min = String(p.min)
      num.max = String(p.max)
      num.step = String(p.step ?? 1)
      num.value = range.value
      const sync = () => {
        num.value = range.value
        row.params[p.key] = range.value
        if (p.hot) commit(p, range.value, true)
      }
      range.addEventListener('input', sync)
      range.addEventListener('change', () => {
        if (!p.hot) apply()
      })
      num.addEventListener('change', () => {
        let v = Number(num.value)
        if (!Number.isFinite(v)) v = Number(p.default)
        v = Math.min(Number(p.max), Math.max(Number(p.min), v))
        num.value = String(v)
        range.value = String(v)
        commit(p, String(v), p.hot)
      })
      item.append(range, num)
      const unit = paramUnit(p.key, label.textContent || '')
      if (unit) {
        const u = document.createElement('span')
        u.className = 'unit'
        u.textContent = unit
        item.appendChild(u)
      }
    } else if (p.kind === 'bool') {
      const cb = document.createElement('input')
      cb.type = 'checkbox'
      cb.checked = String(row.params[p.key] ?? p.default) === 'true'
      cb.addEventListener('change', () => commit(p, cb.checked ? 'true' : 'false', false))
      item.appendChild(cb)
    } else {
      const input = document.createElement('input')
      input.type = 'text'
      input.value = row.params[p.key] ?? p.default
      input.placeholder =
        p.key === 'reference' ? T('默认 {path}', { path: '~/.purevox/tse_reference.wav' }) : ''
      input.addEventListener('change', () => commit(p, input.value, false))
      item.appendChild(input)
    }
    box.appendChild(item)
  }
  return box
}

// ---------- 行 / 列 ----------
function rowElement(col, ci, row, ri) {
  const el = document.createElement('div')
  el.className = `row ${row.kind}`
  el.dataset.ci = String(ci)
  el.dataset.ri = String(ri)
  if (!row.enabled) el.classList.add('disabled')
  const last = col.rows.length - 1
  const fixed = ri === 0 || ri === last
  const kind = row.kind

  // 头部一行：类型徽标 + 类型下拉 + 动作区。行是 flex 纵向容器，头部单独一行，
  // 下面的内容条数随便加——**高度由内容决定**（debug.css「布局铁律」）。
  const head = document.createElement('div')
  head.className = 'row-head'

  // 行启用开关：**统一放在第一行最左**。以前只有处理行有，而且在第二行（detail）里，
  // 挤占参数区本来就窄的横向空间，行内一眼也看不出开没开。语义不变 = 挂摘这一行
  // （不产出 + 放掉这一列对设备的引用），配置全留（DESIGN.md §6.5）。
  const en = document.createElement('label')
  en.className = 'row-en'
  const enCb = document.createElement('input')
  enCb.type = 'checkbox'
  // 稳定 id：UIA 自动化（TAURI3.md §3.6）按 AutomationId 点得到，名字里有中文太难匹配
  enCb.id = `row-en-${ci}-${ri}`
  enCb.checked = row.enabled !== false
  enCb.title = T('启用这一行')
  enCb.setAttribute('aria-label', T('启用这一行'))
  enCb.addEventListener('change', () => {
    row.enabled = enCb.checked
    el.classList.toggle('disabled', !row.enabled)
    apply()
  })
  en.append(enCb)
  head.appendChild(en)

  // 健康点：快照到达后由 shell.js 按 engine_info 上色（前端不判定，AGENTS §1.3）
  const health = document.createElement('span')
  health.className = 'row-dot pending'
  head.appendChild(health)

  const badge = document.createElement('span')
  badge.className = 'badge'
  badge.textContent = kindName(kind)
  head.appendChild(badge)

  // 类型下拉（两端固定行也可换类型，只是不能删/移动）
  const ptype = select(nodesOf(kind).map((n) => [n.ptype, T(n.label)]), row.ptype, (v) => {
    const prev = row.ptype
    row.ptype = v
    row.params = {}
    // 无设备的行：测试音（合成）与网络行（网络麦克风 / 网络扬声器）
    if (!needsDevice(kind, v)) row.device = null
    // 回环行（电脑播放音频）选的是**被回环的输出端点**，与输入设备 ID 不是同一套：
    // 换到回环行必须复位成 `loopback`，否则会把原来的麦克风 ID 当渲染端点，
    // 后端 `Initialize` 直接 AUDCLNT_E_WRONG_ENDPOINT_TYPE（0x88890003）→ 应用计划失败。
    else if (isLoopbackRow(kind, v)) row.device = 'loopback'
    // 反过来（回环 → 录音输入）也不能留着 `loopback`，那不是输入设备 ID。
    else if (isLoopbackRow(kind, prev)) row.device = null
    render()
    apply()
  })
  ptype.className = 'ptype'
  head.appendChild(ptype)

  // 动作区常驻。固定行用**虚线框 + 锁**占住同一块位置（宽度按 3 枚图标按钮算），
  // 这样每张卡右上角的尺寸都一样、右缘对齐；以前是「固定」两个汉字，宽度随字变，
  // 一看就歪（2026-10-26 用户要求统一）。
  if (fixed) {
    const tag = document.createElement('span')
    tag.className = 'row-actions fixed-slot'
    tag.title = T('固定行（不可移动 / 删除）')
    tag.innerHTML = ICON_LOCK
    head.appendChild(tag)
  } else {
    const acts = document.createElement('span')
    acts.className = 'row-actions'
    const mk = (icon, title, fn, disabled, danger) => {
      const b = document.createElement('button')
      b.type = 'button'
      b.className = danger ? 'icon danger' : 'icon'
      b.innerHTML = icon
      b.title = title
      b.setAttribute('aria-label', title)
      b.disabled = !!disabled
      b.addEventListener('click', fn)
      return b
    }
    acts.appendChild(mk(ICON_UP, T('上移'), () => {
      const [r] = col.rows.splice(ri, 1)
      col.rows.splice(ri - 1, 0, r)
      render(); apply()
    }, ri - 1 <= 0))
    acts.appendChild(mk(ICON_DOWN, T('下移'), () => {
      const [r] = col.rows.splice(ri, 1)
      col.rows.splice(ri + 1, 0, r)
      render(); apply()
    }, ri + 1 >= last))
    acts.appendChild(mk(ICON_X, T('删除'), () => {
      col.rows.splice(ri, 1)
      render(); apply()
    }, false, true))
    head.appendChild(acts)
  }
  el.appendChild(head)

  // 细节行：输入/输出给设备下拉，处理给参数，其余给提示占位（行开关在第一行最左，不在这里）。
  // **媒体行例外**：soundpad / player 的全部控件由 `media.js` 提供（音效网格 / 走带 /
  // 每个音效的音量与热键），这里不能再画一遍通用参数控件，否则同一件事有两个入口。
  const isMediaRow = kind === 'process' && (row.ptype === 'soundpad' || row.ptype === 'player')
  const detail = document.createElement('div')
  detail.className = 'detail'
  if (isMediaRow) {
    // 媒体行不画 detail，由 media.js 接管
  } else if (kind === 'process') {
    detail.appendChild(paramControls(row, ci, ri, row.ptype === 'tse' ? ['model'] : undefined))
  } else if (kind === 'input' && row.ptype === 'echo_cancel') {
    // AEC 行的全部控件在下面单独的 AEC 块里画（两列网格：左标签 / 右控件，两只下拉
    // 左缘对齐）；这里刻意不放东西，免得同一行有两个布局体系。
  } else if (isLoopbackRow(kind, row.ptype)) {
    // 电脑播放音频（扬声器）：选被回环的输出端点 + 行内增益（系统混音偏响，默认 -6 dB）
    if (!row.device) row.device = 'loopback'
    detail.appendChild(lbl(T('设备')))
    detail.appendChild(select(loopbackOptions(row.device), row.device, (v) => {
      row.device = v
      apply()
    }))
    detail.appendChild(paramControls(row, ci, ri, ['volume_db']))
  } else if (kind === 'input' && row.ptype === 'remote_mic' ||
             kind === 'output' && row.ptype === 'remote_speaker') {
    // 网络行：**不画任何通用控件**。整张卡片由 `netrow.js` 提供，且刻意做得很小 ——
    // 这一行是链路里的一环，不是主角：状态 + 地址 + 一枚小码 + 一个「重启」就够。
    // 详细开关与说明在顶栏「手机」面板里。
  } else if (needsDevice(kind, row.ptype)) {
    // 设备下拉也要有标签：没有标签时既看不出这一行在选什么，控件也没法跟别行对齐
    // （以前用 margin-left 硬顶，等于留一大块空白，2026-10-26 用户指出）。
    detail.appendChild(lbl(T('设备')))
    detail.appendChild(select(deviceOptions(kind, row.device), row.device ?? '', (v) => {
      row.device = v || null
      apply()
    }))
  } else {
    const hint = document.createElement('span')
    hint.className = 'hint'
    hint.textContent = kind === 'input' ? T('无需设备测试音') : '—'
    detail.appendChild(hint)
  }
  el.appendChild(detail)

  // AEC 行：一律「左标签 / 右控件」，标签列与其它行同宽（见 alignRowLabels）。
  // 顺序按用户定的直觉：麦克风 → 扬声器 → 增益 → 校准（直通紧跟在校准之后）。
  if (kind === 'input' && row.ptype === 'echo_cancel') {
    el.classList.add('aec')
    // 麦克风 = 近端（AEC 要清理的那一路）；扬声器 = 远端参考（系统正在放的声音）。
    detail.appendChild(lbl(T('麦克风')))
    detail.appendChild(select(deviceOptions('input', row.device), row.device ?? '', (v) => {
      row.device = v || null
      apply()
    }))
    detail.appendChild(lbl(T('扬声器')))
    // AEC 远端必选：缺省 = 系统默认输出（输出设备的声音就是 AEC 要消除的回声）
    if (!row.params.far_device) row.params.far_device = 'loopback'
    detail.appendChild(select(farOptions(normalizeLoopback(row.params.far_device)), normalizeLoopback(row.params.far_device), (v) => {
      row.params.far_device = v
      apply()
    }))

    // 增益与延时**跟其它行一样带滑块**（只有数字时右侧大片空白，看着像没做完）：
    // 复用 paramControls，标签 / 滑块 / 数字 / 单位四列与处理行完全同一套。
    detail.appendChild(paramControls(row, ci, ri, ['mic_gain_db', 'far_gain_db', 'far_delay_ms']))

    const acts = document.createElement('div')
    acts.className = 'aec-actions'
    const cal = document.createElement('button')
    cal.type = 'button'
    cal.className = 'primary'
    // 稳定 id：无人值守验证（`ui.py` / UIA）要能直接点到「校准」
    cal.id = 'aec-cal'
    cal.textContent = T('校准')
    cal.title = T('送扫频探针：测延时并自动配平近端 / 远端')
    cal.addEventListener('click', () => {
      cal.disabled = true
      cal.textContent = T('校准中')
      invoke('calibrate_aec_delay')
        .then((t) => window.__pvDebug?.('开始校准：' + t))
        .catch((e) => {
          // 失败原因必须**看得见**（顶栏校准状态串），不能只进调试日志
          calSnap = { state: 'failed', value: '', reason: String(e) }
          paintRecCal()
          window.__pvDebug?.('校准失败：' + e)
        })
        .finally(() => {
          cal.disabled = false
          cal.textContent = T('校准')
        })
    })
    acts.appendChild(cal)
    // 直通（A/B 对比）：紧跟在校准后面，不再挤在「麦克风」那一行。
    const by = document.createElement('label')
    by.className = 'en'
    const cb = document.createElement('input')
    cb.type = 'checkbox'
    cb.checked = row.params.bypass === '1' || row.params.bypass === 'true'
    cb.title = T('直通（A/B 对比）')
    cb.addEventListener('change', () => {
      row.params.bypass = cb.checked ? '1' : '0'
      apply()
    })
    by.append(cb, document.createTextNode(T('直通')))
    acts.appendChild(by)
    detail.appendChild(acts)

    const note = document.createElement('div')
    note.className = 'cal-note'
    note.textContent = T('消除麦克风中的扬声器声音。校准时请先关闭音频播放，并把扬声器音量调到正常使用时的音量。')
    detail.appendChild(note)
  }

  // TSE 行：参考路径 + 卡片（当前参考 / 10 秒录音 / 进度 / 完成或失败）
  if (kind === 'process' && row.ptype === 'tse') {
    const d2 = document.createElement('div')
    d2.className = 'detail2'
    d2.appendChild(paramControls(row, ci, ri, ['reference']))
    el.appendChild(d2)
    if (window.__pvTse?.attach(el, ci, ri, row, { apply, running: () => running })) {
      el.classList.add('tse-row')
    }
  }

  // 网络行（网络麦克风 / 网络扬声器）：整张卡片交给 `netrow.js`——状态灯、启停重启、
  // 地址与二维码、这行干什么的说明。跟媒体行同一个道理：控件只由一个模块画。
  if ((kind === 'input' && row.ptype === 'remote_mic') ||
      (kind === 'output' && row.ptype === 'remote_speaker')) {
    if (window.__pvNetRow?.attach(el, ci, ri, row, { apply })) el.classList.add('net')
  }

  // 媒体处理行（音效板 / 播放器）：控制区交给 `media.js`（音效网格 + 走带控制 +
  // 每个音效的全局热键 + 2 Hz 状态刷新都在那边，与本文件的行骨架解耦）。
  if (isMediaRow && window.__pvMedia?.attach(el, ci, ri, row, { apply })) {
    el.classList.add('media')
  }
  return el
}

function columnElement(col, ci) {
  const el = document.createElement('div')
  el.className = 'column audio'
  el.dataset.ci = String(ci)

  const head = document.createElement('div')
  head.className = 'col-head'
  // 列开关也放**最前**：与行开关同一条视觉规则（左边 = 开/关这一路），一眼扫得出来。
  // 停用这一列（**不是删除**）——行、设备选择、参数全留，只放掉这一列对设备的引用，
  // 别的列不受影响（设备由池按引用计数持有，见 DESIGN.md §6.5「启用 / 停用」）。
  const onLabel = document.createElement('label')
  onLabel.className = 'en'
  const on = document.createElement('input')
  on.type = 'checkbox'
  on.id = `col-en-${ci}`
  on.checked = col.enabled !== false
  on.title = T('停用这一列（保留配置，放掉设备引用）')
  on.addEventListener('change', () => {
    col.enabled = on.checked
    apply()
  })
  on.title = col.enabled === false ? T('启用这一列') : T('停用这一列（保留配置，放掉设备引用）')
  on.setAttribute('aria-label', T('启用 / 停用这一列'))
  onLabel.append(on)
  head.appendChild(onLabel)
  const colDot = document.createElement('span')
  colDot.className = 'col-dot idle'
  colDot.title = T('列状态')
  head.appendChild(colDot)
  const title = document.createElement('span')
  title.textContent = T('列 {n}', { n: ci + 1 })
  head.appendChild(title)
  // 常驻（仅一列时禁用），避免按钮出现/消失
  const del = document.createElement('button')
  del.type = 'button'
  del.textContent = T('删除列')
  del.disabled = plan.columns.length <= 1
  del.title = del.disabled ? T('至少保留一列') : T('删除此列')
  del.addEventListener('click', () => {
    if (plan.columns.length <= 1) return
    plan.columns.splice(ci, 1)
    render(); apply()
  })
  head.appendChild(del)
  el.appendChild(head)

  const rows = document.createElement('div')
  rows.className = 'rows'
  col.rows.forEach((r, ri) => rows.appendChild(rowElement(col, ci, r, ri)))
  el.appendChild(rows)

  // 中间加行（插在固定输出行之前）
  const foot = document.createElement('div')
  foot.className = 'col-foot'
  for (const kind of ['input', 'process', 'output']) {
    const b = document.createElement('button')
    b.type = 'button'
    b.textContent = `+ ${kindName(kind)}`
    b.addEventListener('click', () => {
      col.rows.splice(col.rows.length - 1, 0, defaultRow(kind))
      render(); apply()
    })
    foot.appendChild(b)
  }
  el.appendChild(foot)
  return el
}

function render() {
  const host = $('audio-columns')
  if (!plan.columns.length) plan.columns.push(defaultColumn())
  // 整表重渲染：先让 media.js 丢掉对旧 DOM 的引用（2 Hz 状态轮询按挂载表走）
  window.__pvMedia?.reset()
  window.__pvNetRow?.reset()
  window.__pvTse?.reset()
  host.replaceChildren(...plan.columns.map((c, i) => columnElement(c, i)))

  const add = document.createElement('button')
  add.type = 'button'
  add.className = 'add-column'
  add.textContent = '+ ' + T('加一列')
  add.addEventListener('click', () => {
    plan.columns.push(defaultColumn())
    render(); apply()
  })
  host.appendChild(add)
  alignRowLabels()
  window.__pvShell?.onPlan(plan)
}

/**
 * 标签列对齐（纯呈现，无业务语义）：每列取**实测**最宽的标签，写进该列的 `--label-w`，
 * 于是同一列里所有「左标签 / 右控件」的标签左缘、控件左缘各自成一条线。
 *
 * 用 `Range` 量文字本身的宽度，**不要**用 `offsetWidth`：标签是网格项，宽度已经被
 * 标签列撑满了，量出来永远是列宽，量不出「最宽的那个」。
 */
function alignRowLabels() {
  document.querySelectorAll('#audio-columns .column.audio').forEach((col) => {
    let w = 0
    col.querySelectorAll('.lbl, .params .plabel').forEach((el) => {
      const r = document.createRange()
      r.selectNodeContents(el)
      w = Math.max(w, r.getBoundingClientRect().width)
    })
    const host = /** @type {HTMLElement} */ (col)
    if (w > 0) host.style.setProperty('--label-w', `${Math.ceil(w) + 6}px`)
  })
}

/**
 * 把计划里缺 `loopback:` 前缀的回环值补上，返回是否改动过 —— 调用方据此落盘一次。
 *
 * **只在该值确实是已知回环目标时才补前缀**（裸 ID 与某个 `loopback:<id>` 去掉前缀后
 * 相同）。否则不动：值可能本来就指向一只已经不存在的设备，那是「设备不在」的问题，
 * 乱加前缀只会把它伪装成合法值。
 */
function migrateLoopbackIds(plan) {
  const known = new Set(loopTargets.map((t) => t.id))
  let changed = false
  const fix = (v) => {
    if (!v) return null
    const norm = normalizeLoopback(v)
    return norm !== v && known.has(norm) ? norm : null
  }
  for (const c of plan.columns || []) {
    for (const r of c.rows || []) {
      if (r.kind === 'input' && r.ptype === 'loopback') {
        const v = fix(r.device)
        if (v) { r.device = v; changed = true }
      }
      if (r.ptype === 'echo_cancel' && r.params && r.params.far_device) {
        const v = fix(r.params.far_device)
        if (v) { r.params.far_device = v; changed = true }
      }
    }
  }
  if (changed) window.__pvDebug?.('已把旧配置的回环设备值补上 loopback: 前缀')
  return changed
}

// ---------- 顶栏状态串（录制 / 校准） ----------
// Rust 侧的值是中文模板串（调试接口同源）：显示走 __pvTpl 按语言重排（zh 恒显原文），
// 判定 / 参数回填用模板 id 与占位符取值，业务代码不写中文正则、不写中文拼接。
let recSnap = { state: '', value: '', reason: '' }
let calSnap = { state: '', value: '', reason: '' }
/** ok 行 → 模板重排；失败行 → 前缀调用点先过 T()（reason 体是 Rust 诊断文本，按边界保持中文）。 */
const probeShow = (v, failPrefix) =>
  v.state === 'ok'
    ? window.__pvTpl(v.value)?.text ?? v.value
    : v.state === 'unavailable' || v.state === 'failed'
      ? failPrefix + v.reason
      : ''
function paintRecCal() {
  const r = $('rec-status')
  if (r) {
    r.textContent = probeShow(recSnap, T('录制失败：'))
    r.className = 'mono status-line ' + (recSnap.state === 'ok' ? 'ok' : 'na')
  }
  const c = $('calib-status')
  if (c) {
    c.textContent = probeShow(calSnap, T('校准失败：'))
    c.className = 'mono status-line ' + (calSnap.state === 'ok' ? 'ok' : 'na')
  }
}

// ---------- 快照回调（设备列表来自调试快照） ----------
window.__pvOnSnapshot = (s) => {
  // TSE 卡片的录音进度（与顶栏同源快照；本模块只渲染）
  window.__pvTse?.onSnapshot(s)
  // 运行状态：顶栏启动/停止按钮
  if (typeof s.running === 'boolean') {
    running = s.running
    renderRunButton()
  }

  // 录制状态：顶栏显示；从「录制中」变「完成」时重建会话以加载新参考
  const rec = s.recorder
  if (rec) {
    recSnap = { state: rec.state ?? '', value: rec.value ?? '', reason: rec.reason ?? '' }
    paintRecCal()
    const t = recSnap.state === 'ok' ? window.__pvTpl(recSnap.value) : null
    if (t && t.id === 'rec.done' && recSnap.value !== lastRec) {
      lastRec = recSnap.value
      window.__pvDebug?.('参考录制完成，重建会话以加载新参考')
      apply()
    }
  }

  // 延时校准：顶栏显示；测出「延时 X ms」时回填第一个 AEC 行并重建会话
  const cal = s.calib
  if (cal) {
    calSnap = { state: cal.state ?? '', value: cal.value ?? '', reason: cal.reason ?? '' }
    paintRecCal()
    const t = calSnap.state === 'ok' ? window.__pvTpl(calSnap.value) : null
    if (t && t.id === 'cal.result' && plan && calSnap.value !== lastCalib) {
      lastCalib = calSnap.value
      const col = plan.columns.find((c) => c.rows.some((r) => r.ptype === 'echo_cancel'))
      const row = col && col.rows.find((r) => r.ptype === 'echo_cancel')
      if (row) {
        row.params.far_delay_ms = t.vars.d
        row.params.mic_gain_db = t.vars.m
        row.params.far_gain_db = t.vars.f
        window.__pvDebug?.(
          `校准：延时 ${t.vars.d} ms，近端 ${t.vars.m} dB，远端 ${t.vars.f} dB，已回填并重建`
        )
        render()
        apply()
      }
    }
  }

  const d = s.devices
  if (d && d.state === 'ok') {
    devices = {
      inputs: d.value.devices.filter((x) => x.direction === 'input'),
      outputs: d.value.devices.filter((x) => x.direction === 'output'),
    }
    if (d.value.enumerated_at !== deviceStamp) {
      deviceStamp = d.value.enumerated_at
      if (plan) render()
      // 刷新有结果了：把顶栏状态串从「正在刷新」收尾，别一直挂着
      setStatus([T('设备已更新（{a} 入 / {b} 出）', {
        a: devices.inputs.length,
        b: devices.outputs.length,
      })], true)
      window.__pvDebug?.(`设备列表已更新：输入 ${devices.inputs.length} / 输出 ${devices.outputs.length}`)
    }
  }

  // 外壳（侧栏 / 状态栏 / 行健康点）与顶栏读同一份快照
  window.__pvShell?.onSnapshot(s)
}

// ---------- 启动 ----------
// 顶栏「刷新设备」：**必须给反馈**。以前点了按钮只是 `setStatus('正在刷新设备')`
// 然后 `invoke('refresh_devices')`（Rust 侧静默 return），之后状态串再也不会变，
// 看起来就是「按钮没用 / 被卡住」。现在按 Rust 回的话显示，并跟踪刷新状态。
$('btn-refresh').addEventListener('click', () => {
  const b = /** @type {HTMLButtonElement} */ ($('btn-refresh'))
  b.disabled = true
  setStatus([T('正在刷新设备')], true)
  invoke('refresh_devices')
    .then((msg) => {
      setStatus([msg], true)
      window.__pvDebug?.(T('刷新设备：') + msg)
    })
    .catch((e) => {
      setStatus([T('刷新设备失败：') + e])
      window.__pvToast?.(T('刷新设备失败：') + e, 'error')
      window.__pvDebug?.(T('刷新设备失败：') + e)
    })
    .finally(() => {
      b.disabled = false
    })
})

// 语言切换后重渲染列（静态串由 i18n.js 处理；顶栏状态串在此立即重排）
document.addEventListener('pv-langchange', () => {
  if (plan) render()
  renderRunButton()
  paintRecCal()
  paintStatus()
})

$('btn-run').addEventListener('click', () => {
  invoke('set_running', { on: !running })
    .catch((e) => {
      window.__pvToast?.(T('启动/停止失败：') + e, 'error')
      window.__pvDebug?.('启动/停止失败：' + e)
    })
})

async function init() {
  // 首屏只需要两件快事：节点表（静态）与计划（读一个小 JSON）。
  // **不要** await 模型列表 / 回环目标 / 设备枚举：后三者会碰磁盘、pactl 或 cpal，
  // 任一慢住就会让第一列永远不出现（用户看到「只有菜单栏和外框」，系统弹等待/强制退出）。
  try {
    NODES = await invoke('list_nodes')
    plan = await invoke('get_plan')
    if (!plan || !plan.columns) plan = { columns: [defaultColumn()] }
    appliedPlan = clonePlan(plan)
    render()
    setStatus([])
    window.__pvDebug?.('界面就绪（首屏）')
  } catch (e) {
    const st = e && e.stack ? String(e.stack).split('\n').slice(0, 5).join(' ⏎ ') : ''
    setStatus([T('初始化失败：') + e])
    window.__pvDebug?.('界面初始化失败：' + e + (st ? ' ⏎ ' + st : ''))
    return
  }

  // 辅助数据：后台加载，拿到什么就补什么；任何一项失败都不影响已经出现的列。
  ;(async () => {
    try {
      const pairs = await Promise.all(
        NODES.filter((n) => n.kind === 'process').map(async (n) => [
          n.ptype,
          await invoke('list_models', { ptype: n.ptype }),
        ]),
      )
      MODELS = Object.fromEntries(pairs)
      if (plan) render()
    } catch (e) {
      window.__pvDebug?.('加载模型列表失败：' + e)
    }
  })()
  ;(async () => {
    try {
      loopTargets = await invoke('list_loopback_targets')
      if (plan) {
        // 老配置的回环值可能缺 `loopback:` 前缀 → 补上并落盘一次
        const migrated = migrateLoopbackIds(plan)
        render()
        if (migrated) apply()
      }
    } catch (e) {
      window.__pvDebug?.('加载回环目标失败：' + e)
    }
  })()
}

init()
})()
