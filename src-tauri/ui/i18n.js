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

// 中英文界面：中文即 msgid（zh 恒等返回，en 查表、缺失回退中文），与旧实现同思路。
// 用法：静态元素加 data-i18n="中文原文"；JS 里用 T('中文原文')。切换语言调用 __pvSetLang('en'|'zh')。
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
    按键: 'Press keys…',
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
    '无需设备（测试音）': 'No device (tone)',
    未选择设备: 'No device',
    设备不在: 'Device missing',
    输出回环默认输出: 'Output loopback (default)',
    已应用: 'Applied',
    正在刷新设备: 'Refreshing devices…',
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
  }

  /** @type {'zh'|'en'} */
  let lang = 'zh'

  /** @param {string} s */
  const T = (s) => (lang === 'en' ? EN[s] ?? s : s)

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
  /** 切换语言：应用静态串 + 通知各模块重渲染 + 持久化 + 同步到 Rust（托盘）。 */
  window.__pvSetLang = (l) => {
    lang = l === 'en' ? 'en' : 'zh'
    try {
      localStorage.setItem('pv_lang', lang)
    } catch (_) {
      /* 忽略 */
    }
    applyI18n(document)
    syncBtn()
    window.__pvOnLangChange?.()
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
