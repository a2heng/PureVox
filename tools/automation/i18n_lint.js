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

// UI 字符串统一管理门禁（AGENTS.md §4；与 tsc 同挂 check.ps1 -Gate ui，CI 同源跑同一份）。
//   E1 裸中文上屏：ui/*.js / index.html 里没走三入口（data-i18n / T() / __pvTpl 模板）的中文；
//   E2 缺键：T('msgid')（含 STREAM_FIELDS 延迟标签）的 msgid 不在 i18n.js 的 EN 字典；
//   E3 缺键：data-i18n 值不在 EN 字典（en 会静默回退中文 = 漏译）；
//   E4 占位符奇偶：EN 键与值 / TPL 中英模板的 {name} 集合不一致（对应旧版 test_i18n.py）；
//   E5 TPL id 重复；
//   W1 孤儿键：字典定义了但没有任何入口引用（仅告警）；
//   W2 模板脱节：TPL 中文骨架的字面段在 src-tauri/src 里找不到（Rust 改拼串未同步，仅告警）。
// 豁免：注释；i18n.js（字典与模板表自身）；jsdebug.js（调试桥，输出走 ui_report = 调试日志）；
// __pvDebug / console 调试行（结构性识别，跨行调用也覆盖）；STREAM_FIELDS 块内延迟标签
// （渲染时经 labelOf → T() 翻译，只查缺键不查裸）。Rust 诊断文本（probe reason 体）按边界
// 不翻译、由变量拼接，天然不经过本扫描。
// 用法：node tools/automation/i18n_lint.js      （有错误 → 退出码 1）

'use strict'
const fs = require('fs')
const path = require('path')

const ROOT = path.resolve(__dirname, '..', '..')
const UI_DIR = path.join(ROOT, 'src-tauri', 'ui')
const CJK = /[\u4e00-\u9fff]/
const PH = /\{(\w+)\}/g

const errors = []
const warnings = []
const rel = (p) => path.relative(ROOT, p).replace(/\\/g, '/')
const err = (f, line, msg) => errors.push(`${f}:${line}: ${msg}`)
const warn = (f, line, msg) => warnings.push(`${f}:${line}: ${msg}`)

function phSet(s) {
  const out = new Set()
  let m
  PH.lastIndex = 0
  while ((m = PH.exec(s)) !== null) out.add(m[1])
  return out
}
const sameSet = (a, b) => a.size === b.size && [...a].every((x) => b.has(x))

// ---------- 1. 从 i18n.js 提取 EN 字典与 TPL（字符串感知配对扫描 + eval，只解析本仓库源码） ----------
function extractBlock(src, openTok, openCh, closeCh) {
  const at = src.indexOf(openTok)
  if (at < 0) throw new Error(`i18n.js 里找不到 ${openTok}`)
  let depth = 1
  let quote = ''
  for (let j = at + openTok.length; j < src.length; j++) {
    const c = src[j]
    if (quote) {
      if (c === '\\') {
        j++
        continue
      }
      if (c === quote) quote = ''
      continue
    }
    if (c === "'" || c === '"' || c === '`') {
      quote = c
      continue
    }
    if (c === openCh) depth++
    else if (c === closeCh) {
      depth--
      if (depth === 0) return src.slice(at + openTok.length, j)
    }
  }
  throw new Error(`i18n.js 里 ${openTok} 未闭合`)
}

const i18nSrc = fs.readFileSync(path.join(UI_DIR, 'i18n.js'), 'utf8')
const EN = eval(`({${extractBlock(i18nSrc, 'const EN = {', '{', '}')}})`)
const TPL = eval(`[${extractBlock(i18nSrc, 'const TPL = [', '[', ']')}]`)

// ---------- 2. JS 字面量扫描（注释剥除 + 引号/模板/括号栈，标记是否在 T( 与调试调用内） ----------
function scanJs(src) {
  const lits = []
  const stack = [] // 每层 '(' 一个标记：'T' | 'dbg' | 'other'
  let i = 0
  let line = 1

  const callNameAt = (parenIdx) => {
    // 回溯 '(' 前的调用名：跳过空白与 ?. / .，读标识符（可多段，如 window.__pvDebug）
    let k = parenIdx - 1
    while (k >= 0 && /\s/.test(src[k])) k--
    if (src[k] === '.') k--
    if (src[k] === '?') k--
    while (k >= 0 && /\s/.test(src[k])) k--
    const readIdent = () => {
      const end = k + 1
      while (k >= 0 && /[\w$]/.test(src[k])) k--
      return src.slice(k + 1, end)
    }
    let name = readIdent()
    if (name && (src[k] === '.' || src[k] === '?')) {
      if (src[k] === '?') k--
      k--
      while (k >= 0 && /\s/.test(src[k])) k--
      const parent = readIdent()
      if (parent) name = parent + '.' + name
    }
    return name
  }

  const readStr = (q) => {
    let out = ''
    i++
    while (i < src.length) {
      const c = src[i]
      if (c === '\\') {
        out += src[i + 1] === undefined ? '' : src[i + 1]
        i += 2
        continue
      }
      if (c === q) {
        i++
        break
      }
      if (c === '\n') line++
      out += c
      i++
    }
    return out
  }

  const addLit = (text, lno) =>
    lits.push({ text, line: lno, inT: stack.includes('T'), dbg: stack.includes('dbg') })

  const readTemplate = () => {
    let buf = ''
    while (i < src.length) {
      const c = src[i]
      if (c === '\\') {
        buf += src[i + 1] === undefined ? '' : src[i + 1]
        i += 2
        continue
      }
      if (c === '`') {
        i++
        break
      }
      if (c === '$' && src[i + 1] === '{') {
        if (buf) addLit(buf, line)
        buf = ''
        i += 2
        scanCode('}') // ${} 内按代码扫描（含嵌套 T('…') 字面量）
        continue
      }
      if (c === '\n') line++
      buf += c
      i++
    }
    if (buf) addLit(buf, line)
  }

  function scanCode(until) {
    let depth = 0
    while (i < src.length) {
      const c = src[i]
      if (c === '\n') {
        line++
        i++
        continue
      }
      if (c === '/' && src[i + 1] === '/') {
        while (i < src.length && src[i] !== '\n') i++
        continue
      }
      if (c === '/' && src[i + 1] === '*') {
        i += 2
        while (i < src.length && !(src[i] === '*' && src[i + 1] === '/')) {
          if (src[i] === '\n') line++
          i++
        }
        i += 2
        continue
      }
      if (c === "'" || c === '"') {
        const lno = line
        addLit(readStr(c), lno)
        continue
      }
      if (c === '`') {
        i++
        readTemplate()
        continue
      }
      if (c === '(') {
        const name = callNameAt(i)
        const mark = name === 'T' || name === '__pvT' ? 'T' : /(^|\.)__pvDebug$|^console(\.|$)|^send$/.test(name) ? 'dbg' : 'other'
        stack.push(mark)
        i++
        continue
      }
      if (c === ')') {
        stack.pop()
        i++
        continue
      }
      if (until === '}') {
        if (c === '{') depth++
        else if (c === '}') {
          if (depth === 0) {
            i++
            return
          }
          depth--
        }
      }
      i++
    }
  }

  scanCode(null)
  return lits
}

// ---------- 3. STREAM_FIELDS 延迟标签块（渲染时经 labelOf → T()，只查缺键不查裸） ----------
function streamBlockRange(src) {
  const m = /const STREAM_FIELDS = \[/.exec(src)
  if (!m) return null
  const startLine = src.slice(0, m.index).split('\n').length
  let idx = src.indexOf('\n]', m.index)
  if (idx < 0) return { startLine, endLine: startLine }
  const endLine = src.slice(0, idx).split('\n').length + 1
  return { startLine, endLine }
}

// ---------- 4. 扫描 ui/*.js ----------
const usedKeys = new Set()
const EN_FILES = ['i18n.js', 'jsdebug.js'] // 字典自身 / 调试桥：整体豁免
const jsFiles = fs.readdirSync(UI_DIR).filter((f) => f.endsWith('.js'))

for (const f of jsFiles) {
  if (EN_FILES.includes(f)) continue
  const p = path.join(UI_DIR, f)
  const src = fs.readFileSync(p, 'utf8')
  const label = rel(p)
  const lines = src.split('\n')
  const block = streamBlockRange(src)
  for (const lit of scanJs(src)) {
    if (!CJK.test(lit.text)) continue
    const lineText = lines[lit.line - 1] ?? ''
    if (/^\s*(\/\/|\*|\/\*)/.test(lineText)) continue // 整行注释
    if (lit.dbg) continue // __pvDebug / console 调试输出 = 调试日志，不翻译
    if (lit.inT) {
      usedKeys.add(lit.text)
      if (!Object.prototype.hasOwnProperty.call(EN, lit.text)) {
        err(label, lit.line, `E2 T() msgid 不在字典：${JSON.stringify(lit.text)}`)
      }
      continue
    }
    const inBlock = block && lit.line >= block.startLine && lit.line <= block.endLine
    if (inBlock) {
      usedKeys.add(lit.text)
      if (!Object.prototype.hasOwnProperty.call(EN, lit.text)) {
        err(label, lit.line, `E2 延迟标签 msgid 不在字典：${JSON.stringify(lit.text)}`)
      }
      continue
    }
    err(label, lit.line, `E1 裸中文上屏（未走 data-i18n / T() / __pvTpl）：${JSON.stringify(lit.text)}`)
  }
}

// ---------- 5. 扫描 index.html ----------
{
  const p = path.join(UI_DIR, 'index.html')
  const src = fs.readFileSync(p, 'utf8')
  const label = rel(p)
  let inComment = false
  src.split('\n').forEach((raw, n) => {
    let line = raw
    const lno = n + 1
    if (inComment) {
      if (line.includes('-->')) inComment = false
      return
    }
    const open = line.indexOf('<!--')
    if (open >= 0) {
      const close = line.indexOf('-->', open)
      if (close < 0) {
        inComment = true
        line = line.slice(0, open)
      } else {
        line = line.slice(0, open) + line.slice(close + 3)
      }
    }
    let m
    const attr = /\bdata-i18n\s*=\s*("([^"]*)"|'([^']*)')/g
    while ((m = attr.exec(line)) !== null) {
      const key = m[2] ?? m[3] ?? ''
      usedKeys.add(key)
      if (!Object.prototype.hasOwnProperty.call(EN, key)) {
        err(label, lno, `E3 data-i18n 不在字典：${JSON.stringify(key)}`)
      }
    }
    if (CJK.test(line)) {
      // 带 data-i18n 的行：属性值与元素文本都由它管辖
      if (!/\bdata-i18n\s*=/.test(line)) {
        const noComment = line.replace(/<!--.*?-->/g, '')
        const mm = /[\u4e00-\u9fff]+[^<]*?/.exec(noComment)
        err(label, lno, `E1 裸中文上屏（缺 data-i18n）：${JSON.stringify(mm ? mm[0].trim() : noComment.trim())}`)
      }
    }
  })
}

// ---------- 6. E4 / E5：字典与模板的占位符奇偶 ----------
for (const [k, v] of Object.entries(EN)) {
  if (typeof v !== 'string') {
    err('ui/i18n.js', 0, `E4 字典值不是字符串：${JSON.stringify(k)}`)
    continue
  }
  if (!sameSet(phSet(k), phSet(v))) {
    err('ui/i18n.js', 0, `E4 占位符不一致：${JSON.stringify(k)} → ${JSON.stringify(v)}`)
  }
}
const seenIds = new Set()
for (const t of TPL) {
  if (!t || typeof t.id !== 'string' || typeof t.zh !== 'string' || typeof t.en !== 'string') {
    err('ui/i18n.js', 0, `E5 TPL 条目结构错误：${JSON.stringify(t)}`)
    continue
  }
  if (seenIds.has(t.id)) err('ui/i18n.js', 0, `E5 TPL id 重复：${t.id}`)
  seenIds.add(t.id)
  if (!sameSet(phSet(t.zh), phSet(t.en))) {
    err('ui/i18n.js', 0, `E4 TPL 中英占位符不一致：${t.id}（${t.zh} → ${t.en}）`)
  }
}

// ---------- 7. Rust 源（标签覆盖 E6 + 孤儿判定 + 模板骨架 W2） ----------
const rsRoot = path.join(ROOT, 'src-tauri', 'src')
/** @type {Map<string, string>} */
const rustFiles = new Map()
const walk = (d) => {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name)
    if (e.isDirectory()) walk(p)
    else if (e.name.endsWith('.rs')) rustFiles.set(rel(p), fs.readFileSync(p, 'utf8'))
  }
}
walk(rsRoot)
const rustAll = [...rustFiles.values()].join('\n')

// E6：Rust 侧的用户可见标签（注册表节点/参数名、模型表、提示音预设）必须都有 en 条目。
// 界面侧经 T() 翻译（中文即 msgid），条目缺失就是漏译——直接对 Rust 源校验覆盖。
const labelSources = [
  { file: 'src-tauri/src/engine/registry.rs', re: /label:\s*"([^"]+)"/g },
  { file: 'src-tauri/src/cues.rs', re: /,\s*"([^"]*[\u4e00-\u9fff][^"]*)"\s*\)/g },
  { file: 'src-tauri/src/infer/mod.rs', re: /,\s*"([^"]*[\u4e00-\u9fff][^"]*)"\s*\)/g },
  { file: 'src-tauri/src/infer/tse.rs', re: /,\s*"([^"]*[\u4e00-\u9fff][^"]*)"\s*\)/g },
  { file: 'src-tauri/src/infer/aec.rs', re: /,\s*"([^"]*[\u4e00-\u9fff][^"]*)"\s*\)/g },
]
for (const { file, re } of labelSources) {
  const src = rustFiles.get(file)
  if (src === undefined) {
    err(file, 0, 'E6 Rust 标签源文件缺失')
    continue
  }
  const seen = new Set()
  let m
  re.lastIndex = 0
  while ((m = re.exec(src)) !== null) {
    const key = m[1]
    if (seen.has(key)) continue
    seen.add(key)
    if (!Object.prototype.hasOwnProperty.call(EN, key)) {
      err(file, 0, `E6 Rust 标签缺 en 条目：${JSON.stringify(key)}`)
    }
  }
}

// W1 孤儿键（仅告警）：Rust 侧提供的标签（键在 Rust 源里出现）不算孤儿。
for (const k of Object.keys(EN)) {
  if (usedKeys.has(k)) continue
  if (rustAll.includes(k)) continue
  warn('ui/i18n.js', 0, `W1 孤儿键（无入口引用）：${JSON.stringify(k)}`)
}

// W2 TPL 骨架与 Rust 源比对（仅告警）
for (const t of TPL) {
  if (typeof t.zh !== 'string') continue
  for (const seg of t.zh.split(/\{[^}]+\}/)) {
    if (seg && !rustAll.includes(seg)) {
      warn('ui/i18n.js', 0, `W2 模板 ${t.id} 的骨架段在 Rust 源找不到：${JSON.stringify(seg)}`)
    }
  }
}

// ---------- 9. 汇总 ----------
for (const w of warnings) console.log(`warn  ${w}`)
for (const e of errors) console.error(`ERROR ${e}`)
console.log(`i18n_lint: ${errors.length} error(s), ${warnings.length} warning(s)`)
process.exit(errors.length ? 1 : 0)
