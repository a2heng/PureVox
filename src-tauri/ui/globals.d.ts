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
  // 语言切换后各模块重渲染：在 document 上监听 'pv-langchange'（i18n.js 派发）
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

/** 调试快照（结构见 Rust `DebugSnapshot`；前端只做宽松声明，字段按需取用）。 */
type PvSnapshot = Record<string, any>

