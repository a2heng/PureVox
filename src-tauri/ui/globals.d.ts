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
  /** i18n（i18n.js）：中文 msgid → 当前语言 */
  __pvT?: (s: string) => string
  __pvLang?: () => string
  __pvSetLang?: (lang: string) => void
  /** 语言切换后各模块重渲染（columns.js 实现） */
  __pvOnLangChange?: () => void
}

/** 调试快照（结构见 Rust `DebugSnapshot`；前端只做宽松声明，字段按需取用）。 */
type PvSnapshot = Record<string, any>

