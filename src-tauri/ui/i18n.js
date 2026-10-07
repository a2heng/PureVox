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

// 统一字符串管理（AGENTS.md §4）：本文件是唯一字符串源，中文即 msgid（zh 恒等返回，
// en 查表、缺失回退中文），与旧实现 i18n.py 同思路。上屏只有三个入口：
//   ① 静态元素 data-i18n="中文原文"；② 动态 T('中文原文')，可带占位符：
//      T('列 {n}', { n: 1 })（中英占位符集合必须一致，lint 校验）；
//   ③ Rust 中文模板串用 __pvTpl(raw) 匹配 TPL 表重排（zh 恒显原文、en 重排），
//      Rust 改拼串必须同步 TPL 的 zh 骨架（i18n_lint 会比对 Rust 源码字面骨架）。
// 其余中文一律不上屏；注释、调试日志（__pvDebug / ui_report / console）与 Rust
// 诊断文本（probe reason 体）不翻译。切换语言调用 __pvSetLang('en'|'zh')。
// 门禁：tools/automation/i18n_lint.js（check.ps1 -Gate ui）查裸串 / 缺键 /
// 占位符奇偶 / 孤儿键。
;(() => {
  /** @type {Record<string, string>} */
  const EN = {
    // 顶栏
    刷新设备: 'Refresh devices',
    启动: 'Start',
    停止: 'Stop',
    设置: 'Settings',
    启用全局热键: 'Enable global hotkey',
    录制: 'Record',
    '按键…': 'Press keys…',
    启用提示音: 'Enable sounds',
    启动音: 'Start sound',
    停止音: 'Stop sound',
    启动自运行并隐藏窗口: 'Run and hide window on launch',
    关于: 'About',
    关于模型声明: 'Built-in AI models are not covered by the GPL; licensed for use with PureVox only (see MODEL-LICENSE.md).',
    保存: 'Save',
    关闭: 'Close',
    调试面板: 'Debug panel',
    开发者工具: 'DevTools',
    开机自启: 'Start with Windows',
    // 列编辑器
    输入: 'Input',
    处理: 'Process',
    输出: 'Output',
    固定: 'Fixed',
    删除列: 'Delete column',
    加一列: 'Add column',
    近端: 'Near',
    远端: 'Far',
    近端增益: 'Near gain',
    远端增益: 'Far gain',
    延时: 'Delay',
    校准: 'Calibrate',
    录制参考: 'Record ref',
    直通: 'Bypass',
    '直通（A/B 对比）': 'Bypass (A/B compare)',
    无需设备测试音: 'No device (test tone)',
    未选择设备: 'No device',
    设备不在: 'Device missing',
    输出回环默认输出: 'Output loopback (default)',
    已应用: 'Applied',
    正在刷新设备: 'Refreshing devices…',
    上移: 'Move up',
    下移: 'Move down',
    删除: 'Delete',
    启用: 'Enable',
    '列 {n}': 'Column {n}',
    至少保留一列: 'Keep at least one column',
    删除此列: 'Delete this column',
    '远端相对近端的延时（ms）': 'Delay of far relative to near (ms)',
    '送扫频探针：测延时并自动配平近端 / 远端': 'Sweep probe: measure delay and auto-match near / far levels',
    '录制 10 s「降噪后」的信号作为 TSE 参考（音量归一化）':
      'Record 10 s of the denoised signal as the TSE reference (volume normalized)',
    '默认 {path}': 'Default {path}',
    '初始化失败：': 'Initialization failed: ',
    '录制失败：': 'Recording failed: ',
    '校准失败：': 'Calibration failed: ',
    // 调试列（静态小节）
    调试: 'Debug',
    应用: 'App',
    版本: 'Version',
    运行时长: 'Uptime',
    快照时间: 'Snapshot',
    调试接口: 'Debug HTTP',
    内存: 'Memory',
    'CPU 系统': 'CPU system',
    'CPU 本进程': 'CPU process',
    逻辑核数: 'Logical cores',
    '内存 系统已用': 'Mem used',
    '内存 系统可用': 'Mem free',
    '本进程 工作集': 'Working set',
    '本进程 私有字节': 'Private bytes',
    适配器: 'Adapter',
    整卡占用: 'GPU util',
    本进程占用: 'GPU proc',
    '显存已用 / 总量': 'VRAM used/total',
    本进程显存: 'VRAM proc',
    音频: 'Audio',
    引擎: 'Engine',
    参考录制: 'Ref recording',
    延时校准: 'Delay calib',
    '本进程不含 WebView2 子进程（界面渲染在 msedgewebview2.exe 中）。':
      'Process metrics exclude the WebView2 child (rendered in msedgewebview2.exe).',
    // 调试列（动态值 / 提示）
    '（无数据）': '(no data)',
    '采集中…': 'Collecting…',
    '不可用：': 'Unavailable: ',
    '采样于 {time}': 'Sampled at {time}',
    '最近 {last}  最小 {min}  最大 {max}': 'last {last}  min {min}  max {max}',
    '原生奈奎斯特 {f}k': 'Native Nyquist {f}k',
    '波形（48 kHz，最近 50 ms）': 'Waveform (48 kHz, last 50 ms)',
    '波形（48 kHz，最近 50 ms，送入重采样前）': 'Waveform (48 kHz, last 50 ms, into resampler)',
    '平均频谱（0 ~ 24 kHz，-140 ~ 0 dBFS）': 'Avg spectrum (0 ~ 24 kHz, -140 ~ 0 dBFS)',
    '输入：{name}': 'Input: {name}',
    '输出：{name}': 'Output: {name}',
    '取快照失败：': 'Snapshot failed: ',
    // 音频流字段
    状态: 'State',
    设备: 'Device',
    设备格式: 'Device format',
    重采样: 'Resampler',
    设备侧速率: 'Device rate',
    '引擎侧速率（48k）': 'Engine rate (48k)',
    时钟伺服修正: 'Clock servo',
    重采样延迟: 'Resampler delay',
    '回调块（帧）': 'Callback frames',
    回调次数: 'Callbacks',
    '输入帧 / 输出帧': 'In frames / out frames',
    '48k 消耗帧 / 设备帧': '48k frames / device frames',
    'hop 数 / 剩余帧': 'Hops / pending',
    '峰值 / RMS': 'Peak / RMS',
    环形缓冲水位: 'Ring level',
    '48k 缓冲 / 设备缓冲': '48k buf / device buf',
    '欠载（补静音样本）': 'Underruns (silence)',
    重同步次数: 'Resyncs',
    '丢弃样本 / 流错误': 'Dropped / stream errors',
    端到端延迟: 'End-to-end latency',
    '推理耗时 均值 / 最大': 'Inference avg / max',
    // Rust 提供的用户可见标签（注册表节点/参数名、模型表、提示音预设）：界面侧经 T()
    // 翻译（中文即 msgid），条目缺失就是漏译——lint E6 直接对 Rust 源校验覆盖。
    录音输入: 'Audio input',
    '测试音 1 kHz': 'Test tone 1 kHz',
    'AEC 回声消除': 'AEC echo cancel',
    降噪: 'Denoise',
    增益: 'Gain',
    '目标说话人 TSE': 'Target speaker TSE',
    音频输出: 'Audio output',
    模型: 'Model',
    远端设备: 'Far device',
    远端延时: 'Far delay',
    '增益(dB)': 'Gain (dB)',
    参考语音: 'Reference speech',
    '降噪 202609c（现役）': 'Denoise 202609c (current)',
    '降噪 202609b': 'Denoise 202609b',
    '降噪 202609a': 'Denoise 202609a',
    '降噪 202606': 'Denoise 202606',
    'TSE 202609c（现役）': 'TSE 202609c (current)',
    'AEC 202609（现役）': 'AEC 202609 (current)',
    柔和: 'Soft',
    清脆: 'Crisp',
    气泡: 'Pop',
    电子: 'Blip',
    木鱼: 'Wood',
    风铃: 'Chime',
  }

  // Rust 中文模板串表：zh = Rust format! 产出的字面骨架（{占位} 即捕获组），en = 同构
  // 英文模板，占位符集合必须一致（i18n_lint 校验 + 比对 Rust 源字面骨架）；id 是稳定
  // 短名，代码按 id 判定/取值（不比对中文，避免裸中文字面量散落业务代码）。
  /** @type {Array<{ id: string, zh: string, en: string }>} */
  const TPL = [
    { id: 'rec.progress', zh: '录制中 {a}/{b} s', en: 'Recording {a}/{b} s' },
    {
      id: 'rec.done',
      zh: '完成：{path}（{secs} s，归一化后峰值 {peak} dBFS）',
      en: 'Done: {path} ({secs} s, normalized peak {peak} dBFS)',
    },
    { id: 'cal.collect', zh: '采集中 {a}/{b} s', en: 'Capturing {a}/{b} s' },
    { id: 'cal.compute', zh: '计算中…', en: 'Computing…' },
    {
      id: 'cal.result',
      zh: '延时 {d} ms（相关 {c}）｜近端 {m} dB｜远端 {f} dB',
      en: 'Delay {d} ms (corr {c}), near {m} dB, far {f} dB',
    },
  ]

  /** @type {'zh'|'en'} */
  let lang = 'zh'

  /**
   * 中文 msgid → 当前语言；可选占位符替换（{name}，中英同构）。
   * @param {string} s
   * @param {Record<string, string | number>} [vars]
   */
  const T = (s, vars) => {
    const out = lang === 'en' ? EN[s] ?? s : s
    return vars ? out.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : out
  }

  /** 惰性编译 TPL：zh 骨架 → 逐占位捕获的正则。 */
  let tplTable = null
  const tplCompiled = () => {
    if (tplTable) return tplTable
    tplTable = TPL.map((t) => {
      const names = []
      const src = t.zh
        .split(/(\{[^}]+\})/)
        .map((p) => {
          if (p[0] === '{') {
            names.push(p.slice(1, -1))
            return '(.+?)'
          }
          return p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
        })
        .join('')
      return { id: t.id, en: t.en, names, re: new RegExp(`^${src}$`) }
    })
    return tplTable
  }

  /**
   * 匹配 Rust 中文模板串（录制 / 校准进度与结果等）。
   * @param {string} raw Rust 产出的原始串
   * @returns {{ id: string, vars: Record<string, string>, text: string } | null}
   *   text = 当前语言显示串（zh 恒为原文）；vars = 占位符取值（参数回填也用它）。
   */
  window.__pvTpl = (raw) => {
    for (const t of tplCompiled()) {
      const m = t.re.exec(raw)
      if (!m) continue
      const vars = {}
      t.names.forEach((n, i) => {
        vars[n] = m[i + 1]
      })
      const text = lang === 'en' ? t.en.replace(/\{(\w+)\}/g, (_, k) => vars[k] ?? `{${k}}`) : raw
      return { id: t.id, vars, text }
    }
    return null
  }

  /** 把 root 下所有 [data-i18n] 元素按当前语言重写文本。 */
  const applyI18n = (root) => {
    root.querySelectorAll('[data-i18n]').forEach((el) => {
      el.textContent = T(el.dataset.i18n)
    })
  }

  const syncBtn = () => {
    const btn = document.getElementById('btn-lang')
    if (btn) btn.textContent = lang === 'en' ? '中' : 'EN'
  }

  window.__pvT = T
  window.__pvLang = () => lang
  /** 切换语言：应用静态串 + 广播事件（各模块自行监听重渲染） + 持久化 + 同步到 Rust（托盘）。 */
  window.__pvSetLang = (l) => {
    lang = l === 'en' ? 'en' : 'zh'
    try {
      localStorage.setItem('pv_lang', lang)
    } catch (_) {
      /* 忽略 */
    }
    applyI18n(document)
    syncBtn()
    document.dispatchEvent(new CustomEvent('pv-langchange'))
    try {
      window.__TAURI__.core.invoke('set_language', { lang })
    } catch (_) {
      /* 忽略 */
    }
  }

  window.addEventListener('DOMContentLoaded', () => {
    try {
      if (localStorage.getItem('pv_lang') === 'en') lang = 'en'
    } catch (_) {
      /* 忽略 */
    }
    applyI18n(document)
    syncBtn()
    document.getElementById('btn-lang')?.addEventListener('click', () => {
      window.__pvSetLang(lang === 'en' ? 'zh' : 'en')
    })
  })
})()
