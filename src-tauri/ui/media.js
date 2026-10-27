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

// 媒体处理行的界面（ARCHITECTURE.md §4）：音效板 `soundpad` 与音乐播放器 `player`。
//
// 二者都是**处理行**（不是输入行）：在自身链位置把媒体样本与信号相加。
// 播放 / 暂停 / seek / 音量走 `media_command` 实时命令（**不重建会话**），
// 只有增删音效、换曲才是结构性变更 → `apply_plan`。
//
// 变更分类（与 ARCHITECTURE.md §5 一致，界面必须照此表现给用户）：
//   热参数（拖动即生效，不卡顿）：音量、进度拖动
//   结构性（落盘 + 重建会话）：增删音效、改热键、换曲
//
// 行状态（`media_status`）用一个全局 2 Hz 轮询刷新所有挂载中的媒体行；
// 语言切换时按 `pv-langchange` 重绘（动态文字必须可重放，CONTRIBUTING.md）。
;(function () {
  'use strict'
  const { invoke } = window.__TAURI__.core

  /**
   * i18n 字典查找（缺键回退中文）。`vars` 可选，用于 `{name}` 占位符替换。
   * @param {string} s
   * @param {Record<string, string | number>} [vars]
   * @returns {string}
   */
  function T(s, vars) {
    return window.__pvT ? window.__pvT(s, vars) : s
  }

  /**
   * 一行已挂载的媒体行：2 Hz 轮询按这张表刷新状态文字 / 进度条 / 在播标记。
   * @typedef {{
   *   ci: number, ri: number, ptype: string, rowEl: HTMLElement,
   *   status: HTMLElement,
   *   seek: HTMLInputElement | null,
   *   cards?: () => HTMLButtonElement[],
   *   setDur?: (d: number) => void,
   *   setPos?: (p: number) => void,
   *   setPlaying?: (on: boolean) => void
   * }} Mounted
   */

  /** @type {Set<Mounted>} */
  const mounted = new Set()
  /** @type {number|null} */
  let ticker = null

  /** ---------- 音效列表：计划参数 `pads`（JSON 数组）互转 ---------- */

  /**
   * @param {any} row
   * @returns {Array<{name:string, path:string, volume_db:number, hotkey:string}>}
   */
  const readPads = (row) => {
    try {
      const raw = JSON.parse(row.params.pads || '[]')
      return Array.isArray(raw)
        ? raw.map((p) => ({
            name: p.name || '',
            path: p.path || '',
            volume_db: Number(p.volume_db) || 0,
            hotkey: p.hotkey || '',
          }))
        : []
    } catch (_) {
      return []
    }
  }

  /**
   * @param {any} row
   * @param {Array<{name:string, path:string, volume_db:number, hotkey:string}>} pads
   */
  const writePads = (row, pads) => {
    row.params.pads = JSON.stringify(
      pads.map((p) => ({
        name: p.name,
        path: p.path,
        volume_db: Number(p.volume_db) || 0,
        hotkey: p.hotkey || '',
      }))
    )
  }

  // ---------- 通用小控件 ----------

  /**
   * @param {string} tag @param {string} [cls] @param {string} [text]
   * @returns {HTMLElement}
   */
  const el = (tag, cls, text) => {
    const e = /** @type {HTMLElement} */ (document.createElement(tag))
    if (cls) e.className = cls
    if (text != null) e.textContent = text
    return e
  }

  /**
   * @param {string} text @param {string} title @param {() => void} fn
   * @param {string} [cls]
   * @returns {HTMLButtonElement}
   */
  const btn = (text, title, fn, cls) => {
    const b = /** @type {HTMLButtonElement} */ (document.createElement('button'))
    b.type = 'button'
    if (cls) b.className = cls
    b.textContent = text
    if (title) b.title = title
    b.addEventListener('click', fn)
    return b
  }

  /** 秒 → `m:ss`（播放器进度；>1 h 用 `h:mm:ss`）。 */
  const fmt = (sec) => {
    const s = Math.max(0, Math.floor(sec || 0))
    const h = Math.floor(s / 3600)
    const m = Math.floor((s % 3600) / 60)
    const ss = s % 60
    const two = (/** @type {number} */ n) => String(n).padStart(2, '0')
    return h > 0 ? h + ':' + two(m) + ':' + two(ss) : m + ':' + two(ss)
  }

  const baseName = (/** @type {string} */ p) => (p || '').split(/[\\/]/).pop() || ''

  /** ±dB 显示（统一 `+3 dB` / `-3 dB` / `0 dB`）。 */
  const dbText = (/** @type {number} */ db) => (db > 0 ? '+' : '') + db + ' dB'

  // ---------- 文件导入 ----------

  /** Uint8Array → base64（分块，避开 apply 参数上限）。 */
  function bytesToBase64(/** @type {Uint8Array} */ bytes) {
    let bin = ''
    const CH = 0x8000
    for (let i = 0; i < bytes.length; i += CH) {
      bin += String.fromCharCode.apply(
        null,
        /** @type {any} */ (bytes.subarray(i, i + CH))
      )
    }
    return btoa(bin)
  }

  /**
   * 打开系统文件选择器 → 读字节 → base64 → `media_import`（解码归一后落库）。
   * 返回库内条目数组。用户取消时 resolve 空数组（不是错误）。
   * @param {'soundpad'|'music'} kind @param {boolean} multiple
   * @returns {Promise<Array<{path:string,name:string,reused:boolean}>>}
   */
  function pickAndImport(kind, multiple) {
    return new Promise((resolve, reject) => {
      const inp = /** @type {HTMLInputElement} */ (document.createElement('input'))
      inp.type = 'file'
      inp.accept = 'audio/*,.wav,.mp3,.flac,.m4a,.aac,.ogg,.opus,.wma,.aiff'
      inp.multiple = multiple
      inp.style.display = 'none'
      document.body.appendChild(inp)
      inp.addEventListener('change', () => {
        const files = Array.from(inp.files || [])
        inp.remove()
        if (!files.length) return resolve([])
        Promise.all(
          files.map(
            (f) =>
              new Promise((res, rej) => {
                const fr = new FileReader()
                fr.onerror = () => rej(new Error(T('读取文件失败：') + f.name))
                fr.onload = () => {
                  const bytes = new Uint8Array(/** @type {ArrayBuffer} */ (fr.result))
                  invoke('media_import', { dataB64: bytesToBase64(bytes), name: f.name, kind })
                    .then((info) => res(/** @type {any} */ (info)))
                    .catch(rej)
                }
                fr.readAsArrayBuffer(f)
              })
          )
        )
          .then((list) => resolve(list))
          .catch(reject)
      })
      inp.click()
    })
  }

  // ---------- 键位录制（音效的全局热键） ----------

  /**
   * 只读输入框：点一下开始录下一个组合键（规范串由 Rust `hotkey::normalize_spec` 归一，
   * 这里只报「用户按了什么」）。Delete / Backspace 清除。
   * @param {() => string} get @param {(v: string) => void} set
   * @returns {HTMLInputElement}
   */
  function hotkeyField(get, set) {
    const input = /** @type {HTMLInputElement} */ (document.createElement('input'))
    input.type = 'text'
    input.readOnly = true
    input.placeholder = T('未绑定')
    input.title = T('点一下再按组合键；Delete 清除')
    let rec = false
    const paint = () => {
      input.value = get() || ''
    }
    paint()
    input.addEventListener('focus', () => {
      rec = true
      input.classList.add('rec')
      input.placeholder = T('按键…')
      input.select()
    })
    input.addEventListener('blur', () => {
      rec = false
      input.classList.remove('rec')
      input.placeholder = T('未绑定')
      paint()
    })
    input.addEventListener('keydown', (e) => {
      if (!rec) return
      e.preventDefault()
      if (e.key === 'Delete' || e.key === 'Backspace') {
        set('')
        rec = false
        input.blur()
        paint()
        return
      }
      const parts = []
      if (e.ctrlKey) parts.push('Ctrl')
      if (e.altKey) parts.push('Alt')
      if (e.shiftKey) parts.push('Shift')
      if (e.metaKey) parts.push('Win')
      const k = e.key
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(k)) return // 只按了修饰键，继续等
      let token = ''
      if (k.length === 1) token = k.toUpperCase()
      else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(k)) token = k
      else
        token =
          /** @type {Record<string,string>} */ ({
            Backspace: 'Backspace',
            Tab: 'Tab',
            Enter: 'Enter',
            Escape: 'Esc',
            ' ': 'Space',
            PageUp: 'PageUp',
            PageDown: 'PageDown',
            End: 'End',
            Home: 'Home',
            ArrowLeft: 'Left',
            ArrowUp: 'Up',
            ArrowRight: 'Right',
            ArrowDown: 'Down',
            Insert: 'Insert',
            Delete: 'Delete',
            '.': '.',
            ',': ',',
            '/': '/',
            ';': ';',
            "'": "'",
            '[': '[',
            ']': ']',
            '\\': '\\',
            '-': '-',
            '=': '=',
            '`': '`',
          })[k] || k
      if (!token) return
      parts.push(token)
      // 无修饰键只允许功能键（与 hotkey.rs 同一契约，否则录了注册不上）
      if (parts.length === 1 && !/^F([1-9]|1[0-9]|2[0-4])$/.test(token)) return
      set(parts.join('+'))
      rec = false
      input.blur()
      paint()
    })
    return input
  }

  // ---------- 音量滑杆（热参数：拖动实时、抬手落盘） ----------

  /**
   * @param {() => number} get @param {(v: number) => void} live 拖动中
   * @param {() => void} commit 抬手（落盘，不重建会话）
   * @returns {HTMLElement}
   */
  function volumeSlider(get, live, commit) {
    const wrap = el('span', 'vol')
    const r = /** @type {HTMLInputElement} */ (document.createElement('input'))
    r.type = 'range'
    r.min = '-10'
    r.max = '10'
    r.step = '1'
    const num = el('span', 'vol-num mono')
    const paint = () => {
      r.value = String(get())
      num.textContent = dbText(get())
    }
    paint()
    r.addEventListener('input', () => {
      live(Number(r.value))
      num.textContent = dbText(get())
    })
    r.addEventListener('change', commit)
    wrap.append(r, num)
    return wrap
  }

  // ---------- 音效板 ----------

  /**
   * @param {any} row @param {number} ci @param {number} ri
   * @param {{apply:()=>void}} rerender
   * @returns {{status: HTMLElement, grid: HTMLElement, foot: HTMLElement,
   *            cards: () => HTMLButtonElement[]}}
   */
  function soundpadPanel(row, ci, ri, rerender) {
    /** @type {Array<{name:string,path:string,volume_db:number,hotkey:string}>} */
    let pads = readPads(row)
    const status = el('div', 'media-status')
    /** @type {HTMLButtonElement[]} */
    const cards = []

    const send = (/** @type {any} */ cmd) => invoke('media_command', { col: ci, row: ri, cmd })

    /** 结构性变更（增删音效 / 改热键）：落盘 + 重建会话（热键随之重注册）。 */
    const commit = () => {
      writePads(row, pads)
      rerender.apply()
    }

    const grid = el('div', 'pad-grid')

    const paint = () => {
      cards.length = 0
      grid.replaceChildren()

      if (!pads.length) {
        const dz = btn('', '', () => doImport(), 'dropzone')
        dz.append(
          el('span', 'dz-icon', '＋'),
          el('span', 'dz-main', T('添加音效')),
          el('span', 'dz-sub', T('导入音频文件（自动转 48 kHz 单声道入库）'))
        )
        grid.appendChild(dz)
        return
      }
      pads.forEach((p, i) => {
        const card = el('div', 'pad')
        // 顶部：圆形播放键 + 名字（点名字也能播）+ 删除
        const top = el('div', 'pad-top')
        const play = btn('▶', T('播放这个音效'), () => send({ SoundpadPlay: i }), 'pad-play')
        cards.push(play)
        const names = el('div', 'pad-names')
        const name = btn(p.name, p.path, () => send({ SoundpadPlay: i }), 'pad-name')
        const meta = el('div', 'pad-meta')
        const hkBadge = el('span', 'pad-hotkey' + (p.hotkey ? '' : ' none'), p.hotkey || T('无热键'))
        meta.appendChild(hkBadge)
        names.append(name, meta)
        top.append(play, names, btn('✕', T('移除这个音效'), () => remove(i), 'icon danger'))
        // 音量（热参数，拖动实时）；热键录制单独一行、占满整行才好点
        const bottom = el('div', 'pad-bottom')
        bottom.append(
          volumeSlider(
            () => p.volume_db,
            (v) => {
              p.volume_db = v
              send({ Volume: { index: i, db: v } })
            },
            () => rerender.apply() // 落盘（persist_plan），不重建
          )
        )
        const hk = hotkeyField(
          () => p.hotkey || '',
          (s) => {
            p.hotkey = s
            hkBadge.textContent = s || T('无热键')
            hkBadge.className = 'pad-hotkey' + (s ? '' : ' none')
            commit()
          }
        )
        hk.className = 'pad-hk'
        card.append(top, bottom, hk)
        grid.appendChild(card)
      })
    }

    const remove = (/** @type {number} */ i) => {
      invoke('media_discard', { path: pads[i].path })
        .then(() => {
          pads.splice(i, 1)
          commit()
          paint()
        })
        .catch((e) => {
          status.className = 'media-status'
          status.textContent = ''
          window.__pvToast?.(T('移除失败：') + String(e), 'error', 7000)
        })
    }

    const doImport = () => {
      status.className = 'media-status'
      status.textContent = T('正在解码入库…')
      pickAndImport('soundpad', true)
        .then((infos) => {
          status.textContent = ''
          if (!infos.length) return
          for (const info of infos) {
            pads.push({ name: info.name, path: info.path, volume_db: 0, hotkey: '' })
          }
          commit()
          paint()
          window.__pvToast?.(T('已添加 {n} 个音效', { n: infos.length }), 'ok')
        })
        .catch((e) => {
          status.className = 'media-status'
          status.textContent = ''
          window.__pvToast?.(T('导入失败：') + String(e), 'error', 7000)
        })
    }

    paint()

    const foot = el('div', 'media-foot')
    foot.append(
      btn(T('添加音效'), T('导入音频文件（自动转 48 kHz 单声道入库）'), () => doImport()),
      btn(T('全部停止'), T('停止所有正在播放的音效'), () =>
        send({ SoundpadStopAll: null })
      ),
      status
    )
    return { status: status, grid: grid, foot: foot, cards: () => cards }
  }

  // ---------- 播放器 ----------

  /**
   * @param {any} row @param {number} ci @param {number} ri
   * @param {{apply:()=>void}} rerender
   */
  function playerPanel(row, ci, ri, rerender) {
    const path = row.params.path || ''
    const vol = Number(row.params.volume_db) || 0
    const status = el('div', 'media-status')
    let durSec = 0

    const send = (/** @type {any} */ cmd) => invoke('media_command', { col: ci, row: ri, cmd })

    // 换曲 = 结构性变更（要重开解码线程）；清曲同理
    const setTrack = (/** @type {string} */ p) => {
      row.params.path = p
      row.params.resume_sec = '0'
      rerender.apply()
    }

    // ── 曲目信息
    const top = el('div', 'player-top')
    const art = el('div', 'player-art', path ? '♪' : '—')
    const info = el('div', 'player-info')
    const title = el('div', 'player-title', path ? baseName(path) : T('未选曲目'))
    const sub = el('div', 'player-sub', T('曲目按首尾相连的环流播放，到尾自动回到开头。'))
    info.append(title, sub)
    top.append(art, info)

    // ── 进度：左当前时间 / 中滑杆 / 右总长
    const seekRow = el('div', 'player-seek')
    const tNow = el('span', 't', '0:00')
    const seek = /** @type {HTMLInputElement} */ (document.createElement('input'))
    seek.type = 'range'
    seek.min = '0'
    seek.max = '1000'
    seek.step = '1'
    seek.value = '0'
    seek.disabled = true
    const tDur = el('span', 't r', '0:00')
    seekRow.append(tNow, seek, tDur)
    let dragging = false
    seek.addEventListener('pointerdown', () => {
      dragging = true
    })
    seek.addEventListener('input', () => {
      if (!dragging) return
      tNow.textContent = fmt((Number(seek.value) / 1000) * durSec)
    })
    seek.addEventListener('change', () => {
      dragging = false
      if (durSec > 0) send({ PlayerSeek: (Number(seek.value) / 1000) * durSec })
    })

    // ── 走带控制
    const ctl = el('div', 'player-ctl')
    const play = btn('▶', T('播放 / 继续'), () => send({ PlayerPlay: null }), 'btn-play')
    const pause = btn('❚❚', T('暂停'), () => send({ PlayerPause: null }))
    const loopBadge = el('span', 'player-badge', T('单曲循环'))
    ctl.append(
      btn(T('选择曲目'), T('导入音频文件（自动转 48 kHz 单声道入库）'), () => doImport()),
      play,
      pause,
      path
        ? btn('✕', T('清空曲目'), () => {
            setTrack('')
            title.textContent = T('未选曲目')
            art.textContent = '—'
          }, 'icon')
        : el('span'),
      loopBadge,
      volumeSlider(
        () => Number(row.params.volume_db) || 0,
        (v) => {
          row.params.volume_db = String(v)
          send({ Volume: { index: null, db: v } })
        },
        () => rerender.apply() // 落盘，不重建
      )
    )

    const doImport = () => {
      status.className = 'media-status'
      status.textContent = T('正在解码入库…')
      pickAndImport('music', false)
        .then((infos) => {
          status.textContent = ''
          if (!infos.length) return
          setTrack(infos[0].path)
          title.textContent = baseName(infos[0].path)
          art.textContent = '♪'
          window.__pvToast?.(T('曲目已导入'), 'ok')
        })
        .catch((e) => {
          status.className = 'media-status'
          status.textContent = ''
          window.__pvToast?.(T('导入失败：') + String(e), 'error', 7000)
        })
    }

    const wrap = el('div', 'player')
    wrap.append(top, seekRow, ctl, status)
    return {
      wrap,
      status,
      seek,
      setDur: (/** @type {number} */ d) => {
        durSec = d
        tDur.textContent = fmt(d)
        seek.disabled = !(d > 0)
      },
      setPos: (/** @type {number} */ p) => {
        if (!dragging) {
          seek.value = String(durSec > 0 ? Math.round((p / durSec) * 1000) : 0)
          tNow.textContent = fmt(p)
        }
      },
      setPlaying: (/** @type {boolean} */ on) => {
        play.classList.toggle('on', on)
        play.textContent = on ? '❚❚' : '▶'
      },
    }
  }

  // ---------- 状态轮询（所有挂载中的媒体行共用一个 2 Hz 定时器）----------

  async function tick() {
    for (const m of Array.from(mounted)) {
      if (!document.body.contains(m.rowEl)) {
        mounted.delete(m)
        continue
      }
      try {
        const st = /** @type {any} */ (await invoke('media_status', { col: m.ci, row: m.ri }))
        paintStatus(m, st)
      } catch (_) {
        // 会话没跑 / 行不是媒体行：显示「未运行」，不算错误
        m.status.textContent = T('会话未运行')
      }
    }
  }

  /** @param {Mounted} m @param {any} st */
  function paintStatus(m, st) {
    if (!st) {
      m.status.textContent = T('会话未运行')
      return
    }
    if (m.ptype === 'soundpad') {
      let s = T('音效 {n} 个，在播 {m}', {
        n: st.pads.length,
        m: st.pads.filter((/** @type {any} */ p) => p.playing).length,
      })
      m.status.className = 'media-status' + (st.error ? ' err' : '')
      if (st.error) s += T('（{e}）', { e: st.error })
      m.status.textContent = s
      for (const [i, p] of st.pads.entries()) {
        const b = /** @type {HTMLElement|null} */ (
          m.cards ? m.cards()[i] : null
        )
        if (b) {
          b.classList.toggle('on', !!p.playing)
          b.textContent = p.playing ? '❚❚' : '▶'
        }
      }
      return
    }
    // 播放器
    const state = st.playing ? T('播放中') : T('已暂停')
    if (st.error) {
      m.status.className = 'media-status err'
      m.status.textContent = T('播放出错：{e}', { e: st.error })
    } else if (st.dur > 0) {
      m.status.className = 'media-status'
      m.status.textContent = T('{state} {pos} / {dur}', { state, pos: fmt(st.pos), dur: fmt(st.dur) })
    } else if (st.playing) {
      m.status.className = 'media-status'
      m.status.textContent = state
    } else {
      m.status.className = 'media-status'
      m.status.textContent = T('未选曲目')
    }
    m.setDur && m.setDur(st.dur)
    m.setPos && m.setPos(st.pos)
    m.setPlaying && m.setPlaying(st.playing)
  }

  // ---------- 对外接口（columns.js 在渲染行时调用）----------

  window.__pvMedia = {
    /**
     * 给一行媒体处理行补上控制区；返回 false 表示不是媒体行。
     * @param {HTMLElement} rowEl 行元素（columns.js 建好）
     * @param {number} ci 列号 @param {number} ri 行号
     * @param {any} row 计划行
     * @param {{ apply: () => void }} rerender
     */
    attach(rowEl, ci, ri, row, rerender) {
      if (row.ptype === 'soundpad') {
        const p = soundpadPanel(row, ci, ri, rerender)
        rowEl.classList.add('media')
        const body = el('div', 'media-body')
        body.append(p.grid, p.foot)
        rowEl.appendChild(body)
        mounted.add({
          ci, ri, ptype: 'soundpad', rowEl,
          status: p.status, seek: null, cards: p.cards,
        })
        return true
      }
      if (row.ptype === 'player') {
        const p = playerPanel(row, ci, ri, rerender)
        rowEl.classList.add('media')
        rowEl.appendChild(p.wrap)
        mounted.add({
          ci, ri, ptype: 'player', rowEl,
          status: p.status, seek: p.seek,
          setDur: p.setDur, setPos: p.setPos, setPlaying: p.setPlaying,
        })
        return true
      }
      return false
    },

    /** 清空挂载表（整表重渲染时由 columns.js 调，避免引用已丢弃的 DOM）。 */
    reset() {
      mounted.clear()
    },
  }

  document.addEventListener('DOMContentLoaded', () => {
    ticker = window.setInterval(tick, 500)
    window.addEventListener('beforeunload', () => {
      if (ticker !== null) window.clearInterval(ticker)
    })
  })
  // 语言切换：状态文字与标签是动态写入的，重新拉一次状态即可按新语言重排
  document.addEventListener('pv-langchange', () => tick())
})()