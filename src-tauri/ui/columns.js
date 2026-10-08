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
  b.className = running ? 'run-stop' : 'run-start'
}

/** 行类型名（msgids 必须内联在 T() 里，i18n_lint 才能查缺键） */
const kindName = (kind) => T(kind === 'input' ? '输入' : kind === 'process' ? '处理' : '输出')
const nodesOf = (kind) => NODES.filter((n) => n.kind === kind)
const specOf = (ptype) => NODES.find((n) => n.ptype === ptype)

/**
 * 这一行要不要选设备。
 * 不需要设备的：`tone`（合成的测试音）、`remote_mic` / `remote_speaker`（网络，
 * 与声卡无关 —— 手机麦克风直接进引擎，不需要虚拟声卡驱动）。
 */
const DEVICE_LESS = new Set(['tone', 'remote_mic', 'remote_speaker'])
const needsDevice = (kind, ptype) =>
  kind === 'output' ? !DEVICE_LESS.has(ptype) : kind === 'input' ? !DEVICE_LESS.has(ptype) : false

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
function setStatus(problems) {
  lastProblems = problems
  paintStatus()
}
function paintStatus() {
  if (lastProblems === null) return
  const el = $('plan-status')
  if (!lastProblems || lastProblems.length === 0) {
    el.textContent = T('已应用')
    el.className = 'mono ok'
  } else {
    el.textContent = lastProblems.join('；')
    el.className = 'mono na'
  }
}

function apply() {
  invoke('apply_plan', { plan })
    .then((problems) => setStatus(problems))
    .catch((e) => {
      setStatus([String(e)])
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
function paramControls(row, keys) {
  const spec = specOf(row.ptype)
  const box = document.createElement('span')
  box.className = 'params'
  if (!spec) return box
  for (const p of spec.params) {
    if (keys && !keys.includes(p.key)) continue
    const label = document.createElement('span')
    label.className = 'plabel'
    label.textContent = T(p.label)
    box.appendChild(label)
    if (p.key === 'model') {
      const opts = (MODELS[row.ptype] ?? []).map((m) => [m.file, T(m.label)])
      box.appendChild(select(opts, row.params.model ?? p.default, (v) => {
        row.params.model = v
        apply()
      }))
    } else if (p.kind === 'number') {
      const input = document.createElement('input')
      input.type = 'number'
      if (p.min != null) input.min = String(p.min)
      if (p.max != null) input.max = String(p.max)
      if (p.step != null) input.step = String(p.step)
      input.value = row.params[p.key] ?? p.default
      input.addEventListener('change', () => {
        row.params[p.key] = input.value
        apply()
      })
      box.appendChild(input)
    } else {
      const input = document.createElement('input')
      input.type = 'text'
      input.value = row.params[p.key] ?? p.default
      input.placeholder =
    p.key === 'reference' ? T('默认 {path}', { path: '~/.purevox/tse_reference.wav' }) : ''
      input.addEventListener('change', () => {
        row.params[p.key] = input.value
        apply()
      })
      box.appendChild(input)
    }
  }
  return box
}

// ---------- 行 / 列 ----------
function rowElement(col, ci, row, ri) {
  const el = document.createElement('div')
  el.className = `row ${row.kind}`
  const last = col.rows.length - 1
  const fixed = ri === 0 || ri === last
  const kind = row.kind

  const badge = document.createElement('span')
  badge.className = 'badge'
  badge.textContent = kindName(kind)
  el.appendChild(badge)

  // 类型下拉（两端固定行也可换类型，只是不能删/移动）
  const ptype = select(nodesOf(kind).map((n) => [n.ptype, T(n.label)]), row.ptype, (v) => {
    row.ptype = v
    row.params = {}
    // 无设备的行：测试音（合成）与网络行（手机麦克风 / 手机扬声器）
    if (!needsDevice(kind, v)) row.device = null
    render()
    apply()
  })
  ptype.className = 'ptype'
  el.appendChild(ptype)

  // 动作区常驻（固定行用同格、同高的「固定」标签），避免出现/消失改变行高
  if (fixed) {
    const tag = document.createElement('span')
    tag.className = 'fixed-tag'
    tag.textContent = T('固定')
    el.appendChild(tag)
  } else {
    const acts = document.createElement('span')
    acts.className = 'row-actions'
    const mk = (text, title, fn, disabled) => {
      const b = document.createElement('button')
      b.type = 'button'; b.textContent = text; b.title = title; b.disabled = !!disabled
      b.addEventListener('click', fn)
      return b
    }
    acts.appendChild(mk('↑', T('上移'), () => {
      const [r] = col.rows.splice(ri, 1)
      col.rows.splice(ri - 1, 0, r)
      render(); apply()
    }, ri - 1 <= 0))
    acts.appendChild(mk('↓', T('下移'), () => {
      const [r] = col.rows.splice(ri, 1)
      col.rows.splice(ri + 1, 0, r)
      render(); apply()
    }, ri + 1 >= last))
    acts.appendChild(mk('✕', T('删除'), () => {
      col.rows.splice(ri, 1)
      render(); apply()
    }))
    el.appendChild(acts)
  }

  // 细节行常驻（固定高）：输入/输出给设备下拉，处理给启用+参数，其余给提示占位
  const detail = document.createElement('div')
  detail.className = 'detail'
  if (kind === 'process') {
    const lab = document.createElement('label')
    lab.className = 'en'
    const cb = document.createElement('input')
    cb.type = 'checkbox'
    cb.checked = row.enabled
    cb.addEventListener('change', () => {
      row.enabled = cb.checked
      apply()
    })
    lab.append(cb, document.createTextNode(T('启用')))
    detail.appendChild(lab)
    detail.appendChild(paramControls(row, row.ptype === 'tse' ? ['model'] : undefined))
  } else if (kind === 'input' && row.ptype === 'echo_cancel') {
    // 近端 mic + 直通开关；远端 + 端侧增益在第 3 行
    detail.appendChild(lbl(T('近端')))
    detail.appendChild(select(deviceOptions('input', row.device), row.device ?? '', (v) => {
      row.device = v || null
      apply()
    }))
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
    detail.appendChild(by)
  } else if (needsDevice(kind, row.ptype)) {
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

  // AEC 行：远端单独一行；近端增益 / 远端增益 / 延时 / 校准 一行
  if (kind === 'input' && row.ptype === 'echo_cancel') {
    el.classList.add('aec')
    const d2 = document.createElement('div')
    d2.className = 'detail2'
    d2.appendChild(lbl(T('远端')))
    // AEC 远端必选：缺省 = 系统默认输出（输出设备的声音就是 AEC 要消除的回声）
    if (!row.params.far_device) row.params.far_device = 'loopback'
    d2.appendChild(select(farOptions(row.params.far_device), row.params.far_device, (v) => {
      row.params.far_device = v
      apply()
    }))
    el.appendChild(d2)

    const d3 = document.createElement('div')
    d3.className = 'detail3'
    d3.appendChild(lbl(T('近端增益')))
    d3.appendChild(numParam(row, 'mic_gain_db', '-40', '40', '1', 3.2))
    d3.appendChild(lbl(T('远端增益')))
    d3.appendChild(numParam(row, 'far_gain_db', '-40', '40', '1', 3.2))
    d3.appendChild(lbl(T('延时')))
    const num = document.createElement('input')
    num.type = 'number'
    num.min = '-1000'
    num.max = '1000'
    num.step = '10'
    num.style.width = '4em'
    const dn = Number(row.params.far_delay_ms)
    num.value = Number.isFinite(dn) ? String(dn) : '0'
    num.title = T('远端相对近端的延时（ms）')
    num.addEventListener('change', () => {
      row.params.far_delay_ms = num.value
      apply()
    })
    d3.appendChild(num)
    const cal = document.createElement('button')
    cal.type = 'button'
    cal.textContent = T('校准')
    cal.title = T('送扫频探针：测延时并自动配平近端 / 远端')
    cal.addEventListener('click', () => {
      invoke('calibrate_aec_delay')
        .then((t) => window.__pvDebug?.('开始校准：' + t))
        .catch((e) => window.__pvDebug?.('校准失败：' + e))
    })
    d3.appendChild(cal)
    el.appendChild(d3)
  }

  // TSE 行第 3 行：参考语音路径 + 录制按钮（录「降噪后」信号，音量归一化）
  if (kind === 'process' && row.ptype === 'tse') {
    el.classList.add('tall')
    const d2 = document.createElement('div')
    d2.className = 'detail2'
    d2.appendChild(paramControls(row, ['reference']))
    const rec = document.createElement('button')
    rec.type = 'button'
    rec.textContent = T('录制参考')
    rec.title = T('录制 10 s「降噪后」的信号作为 TSE 参考（音量归一化）')
    rec.addEventListener('click', () => {
      invoke('record_tse_reference', { seconds: 10 })
        .then((p) => window.__pvDebug?.('开始录制参考：' + p))
        .catch((e) => window.__pvDebug?.('录制参考失败：' + e))
    })
    d2.appendChild(rec)
    el.appendChild(d2)
  }
  return el
}

function columnElement(col, ci) {
  const el = document.createElement('div')
  el.className = 'column audio'

  const head = document.createElement('div')
  head.className = 'col-head'
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
    : v.state === 'unavailable'
      ? failPrefix + v.reason
      : ''
function paintRecCal() {
  const r = $('rec-status')
  if (r) {
    r.textContent = probeShow(recSnap, T('录制失败：'))
    r.className = 'mono ' + (recSnap.state === 'ok' ? 'ok' : 'na')
  }
  const c = $('calib-status')
  if (c) {
    c.textContent = probeShow(calSnap, T('校准失败：'))
    c.className = 'mono ' + (calSnap.state === 'ok' ? 'ok' : 'na')
  }
}

// ---------- 快照回调（设备列表来自调试快照） ----------
window.__pvOnSnapshot = (s) => {
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
      window.__pvDebug?.(`设备列表已更新：输入 ${devices.inputs.length} / 输出 ${devices.outputs.length}`)
    }
  }
}

// ---------- 启动 ----------
$('btn-refresh').addEventListener('click', () => {
  setStatus([T('正在刷新设备')])
  invoke('refresh_devices')
})

// 语言切换后重渲染列（静态串由 i18n.js 处理；顶栏状态串在此立即重排）
document.addEventListener('pv-langchange', () => {
  if (plan) render()
  renderRunButton()
  paintRecCal()
  paintStatus()
})

$('btn-run').addEventListener('click', () => {
  invoke('set_running', { on: !running }).catch((e) => window.__pvDebug?.('启动/停止失败：' + e))
})

async function init() {
  try {
    NODES = await invoke('list_nodes')
    MODELS = {}
    for (const n of NODES.filter((n) => n.kind === 'process')) {
      MODELS[n.ptype] = await invoke('list_models', { ptype: n.ptype })
    }
    plan = await invoke('get_plan')
    loopTargets = await invoke('list_loopback_targets')
    if (!plan || !plan.columns) plan = { columns: [defaultColumn()] }
    render()
    setStatus([])
    window.__pvDebug?.('界面就绪')
  } catch (e) {
    setStatus([T('初始化失败：') + e])
    window.__pvDebug?.('界面初始化失败：' + e)
  }
}

init()
})()
