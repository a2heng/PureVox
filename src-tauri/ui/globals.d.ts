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

// 前端全局声明：供 tsc / LSP 对 ui/*.js 做类型检查（jsconfig.json 打开 checkJs）。

/** Tauri IPC：命令名 + 参数 -> 结果。 */
interface TauriCore {
  invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

interface Window {
  __TAURI__: { core: TauriCore }
  /** debug.js 每轮快照后回调（columns.js 实现，读取设备列表） */
  __pvOnSnapshot?: (snapshot: any) => void
  /** 前端调试桥（jsdebug.js）：把信息转发到调试接口 */
  __pvDebug?: (message: string) => void
  /** i18n（i18n.js）：中文 msgid → 当前语言；可选 {name} 占位符替换 */
  __pvT?: (s: string, vars?: Record<string, string | number>) => string
  /** i18n（i18n.js）：匹配 Rust 中文模板串（录制 / 校准等）→ id / 占位符取值 / 显示串 */
  __pvTpl?: (raw: string) => { id: string; vars: Record<string, string>; text: string } | null
  __pvLang?: () => string
  __pvSetLang?: (lang: string) => void
  /** 驱动面板平台实现（drivers_linux.js / drivers_windows.js 注册，drivers.js 按平台取用） */
  __pvDriversLinux?: PvDriversImpl
  __pvDriversWindows?: PvDriversImpl
  /** 媒体处理行界面（media.js 注册，columns.js 调用） */
  __pvTse?: {
    attach(
      el: HTMLElement,
      ci: number,
      ri: number,
      row: any,
      ctx: { apply: () => void; running: () => boolean },
    ): boolean
    onSnapshot(snapshot: any): void
    reset(): void
  }

  __pvMedia?: PvMediaImpl
  /** 二维码（qrcode.js = 内嵌的第三方库 qrcode-generator v1.4.4，MIT）。
   *  库里那份是上游老式写法、过不了 checkJs，已在 jsconfig.json 里 exclude，
   *  所以这里的类型是手写的。 */
  __pvQr?: PvQrImpl
  /** 应用外壳（shell.js）：状态栏/侧栏/Toast/快捷键 */
  __pvShell?: {
    onSnapshot(snapshot: any): void
    onPlan(plan: any): void
    showProblems(list: string[]): void
    toast(text: string, level?: 'info' | 'ok' | 'warn' | 'error', ms?: number): void
  }
  /** 统一提示（shell.js 注册；任何模块都能调用） */
  __pvToast?: (text: string, level?: 'info' | 'ok' | 'warn' | 'error', ms?: number) => void
  /** 参数控件渲染器（columns.js 注册）：网络行的 AEC 段复用同一份，避免两套漂移 */
  __pvParams?: {
    controls(
      row: any,
      ci: number,
      ri: number,
      keys?: string[]
    ): HTMLElement
  }
  /** AEC 校准结果面板（calib.js 注册）：快照里的结构化报告 → 曲线 + 结论 */
  __pvCalib?: {
    onSnapshot(probe: any): void
    open(): void
    close(): void
    hasReport(): boolean
  }
  /** TSE 参考卡片（tse.js 注册，columns.js 调用）：状态 / 10 秒录音 / 进度 */
  __pvTse?: {
    attach(
      el: HTMLElement,
      ci: number,
      ri: number,
      row: any,
      hooks?: { apply?: () => void; running?: () => boolean }
    ): boolean
    onSnapshot(snapshot: any): void
    reset(): void
  }
  /** 网络行卡片（netrow.js 注册，columns.js 调用）：状态灯 / 启停重启 / 地址 / 二维码 / AEC 段 */
  __pvNetRow?: {
    attach(
      el: HTMLElement,
      ci: number,
      ri: number,
      row: any,
      hooks?: { apply?: () => void }
    ): boolean
    reset(): void
  }
  // 语言切换后各模块重渲染：在 document 上监听 'pv-langchange'（i18n.js 派发）
}

/** 二维码封装（qrcode.js 文件末尾的 PureVox 包装，第三方库本体在它上面）。 */
interface PvQrImpl {
  /** 生成矩阵。注意库里不暴露掩码号，别去读。 */
  encode(text: string, ecl?: 'L' | 'M' | 'Q' | 'H'): { size: number; isDark: (r: number, c: number) => boolean }
  /** 渲染到 canvas（自定像素比例 / 留白 / 配色） */
  render(
    text: string,
    canvas: HTMLCanvasElement,
    opt?: { scale?: number; margin?: number; dark?: string; light?: string; ecl?: string }
  ): { size: number; isDark: (r: number, c: number) => boolean }
}

/** 「驱动」面板的平台实现（见 ui/drivers.js 外壳）。 */
interface PvDriversImpl {
  /** 内容模板 id（index.html 里的 <template>） */
  template: string
  /** 面板标题（已本地化） */
  title(): string
  /** 把模板挂进指定容器并接线 */
  mount(body: HTMLElement): void
  /** 重新拉取状态并重绘 */
  refresh(): void
}

/** 媒体处理行的界面模块（media.js 注册，columns.js 在渲染行时调用）。 */
interface PvMediaImpl {
  /**
   * 给一行媒体处理行（soundpad / player）补上控制区并登记到 2 Hz 状态轮询。
   * @returns true = 确实是媒体行（调用方据此加高行）
   */
  attach(
    rowEl: HTMLElement,
    col: number,
    row: number,
    spec: { ptype: string; params: Record<string, string> },
    rerender: { apply: () => void }
  ): boolean
  /** 清空挂载表（整表重渲染时调用，避免引用已丢弃的 DOM） */
  reset(): void
}

/** 调试快照（结构见 Rust `DebugSnapshot`；前端只做宽松声明，字段按需取用）。 */
type PvSnapshot = Record<string, any>

