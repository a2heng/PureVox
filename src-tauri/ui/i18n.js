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
    更新: 'Update',
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
    '读取参考状态…': 'Reading reference…',
    '已有参考：{name}（{size} · {time} · {dur}）':
      'Reference: {name} ({size} · {time} · {dur})',
    '尚无参考音频——TSE 将直通。': 'No reference audio yet — TSE passes through.',
    '参考状态读取失败：': 'Failed to read reference status: ',
    '录的是「降噪后、TSE 前」的 10 秒信号，音量归一化到 -20 dBFS。请先启动音频处理，并对麦克风持续说话。':
      'Records 10 s of the denoised signal before TSE, normalized to -20 dBFS. Start the engine and keep speaking into the microphone.',
    '● 开始录音 (10s)': '● Record (10 s)',
    '录音中… {sec}s（请持续说话）': 'Recording… {sec}s (keep speaking)',
    '请先启动音频处理，再录制参考。': 'Start the audio engine before recording a reference.',
    '完成！参考已生效。': 'Done — the reference is active.',
    'AI 麦克风降噪': 'AI microphone noise reduction',
    会话: 'Session',
    音频链: 'Audio chain',
    截屏: 'Screenshot',
    '截图已保存：': 'Screenshot saved: ',
    '截图失败：': 'Screenshot failed: ',
    '应用计划失败：': 'Apply failed: ',
    已停止: 'Stopped',
    运行中: 'Running',
    '运行中（需注意）': 'Running (attention needed)',
    '{n} 列': '{n} columns',
    '设备 {a} 入 / {b} 出': '{a} in / {b} out',
    '设备 —': 'Devices —',
    列状态: 'Column status',
    '快捷键：空格 启动/停止 · D 调试面板 · Esc 关闭':
      'Shortcuts: Space start/stop · D debug panel · Esc close',
    '启动/停止失败：': 'Start/stop failed: ',
    噪声门: 'Noise Gate',
    压缩器: 'Compressor',
    限制器: 'Limiter',
    扩展器: 'Expander',
    向上压缩器: 'Upward Compressor',
    '3 段均衡器': '3-Band Equalizer',
    极性反转: 'Invert Polarity',
    关闭阈值: 'Close Threshold',
    打开阈值: 'Open Threshold',
    启动时间: 'Attack Time',
    保持时间: 'Hold Time',
    释放时间: 'Release Time',
    比率: 'Ratio',
    阈值: 'Threshold',
    输出增益: 'Output Gain',
    检测器: 'Detector',
    拐点: 'Knee',
    低频: 'Low',
    中频: 'Mid',
    高频: 'High',
    峰值: 'Peak',
    '固定行（不可移动 / 删除）': 'Fixed row (cannot move or delete)',
    '启用这一行': 'Enable this row',
    '启用这一列': 'Enable this column',
    '启用 / 停用这一列': 'Enable / disable this column',
    应用问题: 'Apply issues',
    '1 个问题': '1 issue',
    '{n} 个问题': '{n} issues',
    直通: 'Bypass',
    '直通（A/B 对比）': 'Bypass (A/B compare)',
    无需设备测试音: 'No device (test tone)',
    未选择设备: 'No device',
    设备不在: 'Device missing',
    已应用: 'Applied',
    正在刷新设备: 'Refreshing devices…',
    上移: 'Move up',
    下移: 'Move down',
    删除: 'Delete',
    启用: 'Enable',
    '停用这一列（保留配置，放掉设备引用）':
      'Disable this column (keeps the config, drops its device references)',
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
    // 媒体行（音效板 / 播放器；见 ui/media.js）
    添加音效: 'Add sounds',
    '导入音频文件（自动转 48 kHz 单声道入库）':
      'Import audio files (normalized to 48 kHz mono into the library)',
    播放这个音效: 'Play this sound',
    移除这个音效: 'Remove this sound',
    全部停止: 'Stop all',
    '停止所有正在播放的音效': 'Stop every playing sound',
    '移除失败：': 'Remove failed: ',
    '导入失败：': 'Import failed: ',
    '读取文件失败：': 'Failed to read file: ',
    '正在解码入库…': 'Decoding into the library…',
    '已添加 {n} 个音效': 'Added {n} sounds',
    未绑定: 'Unbound',
    无热键: 'No hotkey',
    '点一下再按组合键；Delete 清除': 'Click, then press a key combo; Delete clears it',
    按键: 'Press keys',
    '音效 {n} 个，在播 {m}': '{n} sounds, {m} playing',
    '（{e}）': ' ({e})',
    会话未运行: 'Session not running',
    未选曲目: 'No track selected',
    选择曲目: 'Choose track',
    清空曲目: 'Clear track',
    '播放 / 继续': 'Play / resume',
    暂停: 'Pause',
    '播放出错：{e}': 'Playback error: {e}',
    '播放中': 'Playing',
    已暂停: 'Paused',
    '{state} {pos} / {dur}': '{state} {pos} / {dur}',
    单曲循环: 'Loop',
    '曲目按首尾相连的环流播放，到尾自动回到开头。':
      'The track plays as an endless loop — it wraps back to the start at the end.',
    // AEC 行说明
    校准中: 'Calibrating…',
    '远端就是 AEC 要消除的那只输出；校准会向它送扫频探针，测出延时并配平两端电平。':
      'The far device is the output AEC must cancel from; calibration sends a sweep to it to measure the delay and match both ends.',
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
    网络: 'Network',
    '客户端 ⇄ 电脑': 'Client <-> PC',
    '网络麦克风': 'Network microphone',
    '网络扬声器': 'Network speaker',
    '无 Tauri 运行时': 'No Tauri runtime',
    '状态不可用：': 'Status unavailable: ',
    '重启服务': 'Restart service',
    // 网络调试面板的只读行；开关 / 二维码 / 远程输入都在网络行卡片里（netrow.js）
    远程输入: 'Remote input',
    '服务未启动': 'Service not started',
    服务: 'Service',
    '已启动': 'Running',
    '客户端 {n}，订阅 {m}': '{n} clients, {m} subscribed',
    '出站 {n} 包/s': 'Sent {n} pkt/s',
    '进站 {n} 包/s': 'Received {n} pkt/s',
    '往返时延 {n}': 'RTT {n}',
    '（未测）': '(not measured)',
    出站水位: 'Out buffer level',
    '（顶格）': ' (full)',
    没有订阅: 'No subscribers',
    '上限 {n} ms': 'cap {n} ms',
    出站丢样本: 'Out drops',
    进站丢样本: 'In drops',
    'Opus 库': 'Opus library',
    // 虚拟驱动页（drivers_linux.js）：缺 pactl 时「真源」建不出来，要说清缺哪个包
    '缺 pactl：装上 pulseaudio-utils 才有真源':
      'pactl is missing: install pulseaudio-utils to get the real source',
    // 网络行卡片（netrow.js）——手机相关的操作全在这里，顶栏不再有独立面板
    '启动中': 'Starting',
    '停止中': 'Stopping',
    '重启中': 'Restarting',
    '启动局域网服务，客户端才能连进来': 'Start the LAN service so clients can connect',
    '停止服务并断开所有客户端（远程输入同时强制关闭）':
      'Stop the service and disconnect every client (remote input is forced off as well)',
    '重开监听并重新绑定端口（换端口后用这个生效）':
      'Reopen the listener and rebind the port (use after changing the port)',
    '网络服务：': 'Network service: ',
    '网络服务失败：': 'Network service failed: ',
    '客户端地址': 'Client address',
    '取不到本机地址': 'Cannot determine this machine\'s address',
    '下载证书（手机装一次）': 'Download certificate (install once on the phone)',
    '手机浏览器打开这个地址下载 CA 证书，装到系统里，之后扫码访问就不会再报警告了':
      'Open this address on the phone to download the CA certificate and install it into the system; after that, scanning no longer shows a warning',
    '证书还没准备好：先启动服务': 'The certificate is not ready yet: start the service first',
    'CA 证书已下载': 'CA certificate downloaded',
    '取证书失败：': 'Could not read the certificate: ',
    '网卡': 'Network interface',
    '端口': 'Port',
    '自动（默认路由）': 'Automatic (default route)',
    '没有可直连局域网的网卡': 'No interface that can reach the LAN',
    '只决定二维码 / 复制的是哪个地址，服务不用重启（本来就在监听所有网卡）':
      'Only decides which address the QR code and the copy button use. The service keeps running — it already listens on every interface.',
    '改完自动重启服务；已连着的客户端需要重新连':
      'Changing it restarts the service; connected clients have to reconnect',
    '复制': 'Copy',
    '复制失败：': 'Copy failed: ',
    '已复制': 'Copied',
    '浏览器不支持剪贴板': 'Clipboard is not available in this browser',
    '服务未启动，没有可扫的地址': 'Service is not started, so there is nothing to scan',
    '客户端扫码打开网页版客户端': 'Scan to open the web client',
    '复制客户端地址': 'Copy the client address',
    '同网段无鉴权：开启后同一 Wi-Fi 下的任何设备都能向这台电脑打字和发按键':
      'No authentication on the local network: once on, any device on the same Wi-Fi can type and send keys to this PC',
    '二维码生成失败：': 'QR generation failed: ',
    'AEC 消除回声': 'AEC (echo cancellation)',
    '网络麦克风（不可更改）': 'Network microphone (not changeable)',
    '系统默认输出': 'System default output',
    '网络出站（本机刚发出的音频）': 'Network outbound (audio this PC just sent)',
    '勾上后这台网络的麦克风会先过 AEC：远端取哪一路由上面选。近端永远是这张卡的网络输入，不能改。':
      'Once on, this network microphone goes through AEC first. Pick where the reference comes from above; the near end is always this card\'s network input and cannot be changed.',
    '设备已更新（{a} 入 / {b} 出）': 'Devices updated ({a} in / {b} out)',
    '网络扬声器：把处理后的声音 Opus 编码后发给所有已订阅的客户端。客户端连上并订阅后就有声音。':
      'Network speaker: the processed audio is Opus-encoded and sent to every subscribed phone. Sound arrives once a phone opens the client and subscribes.',
    '网络麦克风：远端设备的麦克风当电脑麦克风用，照样进降噪 / TSE / AEC 全链。客户端需连接上面的地址。':
      'Network microphone: the phone microphone acts as a PC microphone and still goes through the full denoise / TSE / AEC chain. A phone client must connect to the address below.',
    '远程输入（打字 + 键盘，默认关闭）': 'Remote input (typing + keyboard, off by default)',
    '已开启': 'On',
    '已关闭': 'Off',
    '刷新设备失败：': 'Device refresh failed: ',
    '远程输入开关失败：': 'Remote input switch failed: ',
    // 更新面板（顶栏「更新」；数据源是本仓库 GitHub Releases，见 src/update/）
    当前版本: 'Current version',
    最新版本: 'Latest version',
    检查更新: 'Check for updates',
    下载更新: 'Download',
    安装更新: 'Install',
    打开发布页: 'Open release page',
    尚未检查: 'Not checked yet',
    '正在检查…': 'Checking…',
    已是最新: 'Up to date',
    有新版本: 'Update available',
    '正在下载…': 'Downloading…',
    '已下载，可以安装': 'Downloaded, ready to install',
    '正在安装…': 'Installing…',
    '已安装，重启 PureVox 生效': 'Installed — restart PureVox to apply',
    '本平台不支持自动更新': 'Automatic updates are not supported on this platform',
    失败: 'Failed',
    '下载失败：': 'Download failed: ',
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
    // 音频流字段（labels 均经 labelOf → T()）
    结论: 'Verdict',
    状态: 'State',
    设备: 'Device',
    格式: 'Format',
    信号源: 'Source',
    重采样: 'Resampler',
    设备侧速率: 'Device rate',
    引擎侧速率: 'Engine rate',
    伺服修正: 'Servo ppm',
    重采样延迟: 'Resampler delay',
    回调块: 'Callback blk',
    回调间隔: 'Callback gap',
    回调次数: 'Callbacks',
    '输入/输出帧': 'In/out frames',
    '消耗/设备帧': 'Out/dev frames',
    'hop/剩余': 'hop/pending',
    '峰值 / RMS': 'Peak / RMS',
    环形缓冲水位: 'Ring level',
    '缓冲/设备': 'Buffer / dev',
    欠载: 'Underruns',
    重同步: 'Resyncs',
    丢弃: 'Drops',
    错误: 'Errors',
    端到端延迟: 'End-to-end latency',
    '推理 均/最大': 'Inference avg/max',
    // Rust 提供的用户可见标签（注册表节点/参数名、模型表、提示音预设）：界面侧经 T()
    // 翻译（中文即 msgid），条目缺失就是漏译——lint E6 直接对 Rust 源校验覆盖。
    录音输入: 'Audio input',
    '测试音 1 kHz': 'Test tone 1 kHz',
    'AEC 回声消除': 'AEC echo cancel',
    降噪: 'Denoise',
    增益: 'Gain',
    '目标说话人 TSE': 'Target speaker TSE',
    音频输出: 'Audio output',
    电脑播放音频: 'Computer playback audio',
    音效板: 'Soundpad',
    音乐播放器: 'Music player',
    模型: 'Model',
    远端设备: 'Far device',
    远端延时: 'Far delay',
    '增益(dB)': 'Gain (dB)',
    '音量(dB)': 'Volume (dB)',
    参考语音: 'Reference speech',
    音效: 'Sound',
    曲目: 'Track',
    续播位置: 'Resume at',
    '降噪 202609c（现役）': 'Denoise 202609c (current)',
    '降噪 202609b': 'Denoise 202609b',
    '降噪 202609a': 'Denoise 202609a',
    '降噪 202606': 'Denoise 202606',
    'TSE 202609c（现役）': 'TSE 202609c (current)',
    'AEC 202610（现役）': 'AEC 202610 (current)',
    柔和: 'Soft',
    清脆: 'Crisp',
    气泡: 'Pop',
    电子: 'Blip',
    木鱼: 'Wood',
    风铃: 'Chime',
    // 驱动（按平台切换：Linux 虚拟驱动 / Windows VB-CABLE）
    驱动: 'Drivers',
    'Linux 虚拟驱动': 'Linux virtual driver',
    'VB-CABLE 驱动': 'VB-CABLE driver',
    '把 PureVox 的降噪输出送到虚拟声卡，其它软件（OBS / 浏览器 / Audacity）就能把它当麦克风用。':
      'Routes PureVox denoised output into a virtual sink so other apps (OBS / browsers / Audacity) can use it as a microphone.',
    '写入口（虚拟声卡）': 'Write-in (virtual sink)',
    '真源（PureVox mic）': 'Real source (PureVox mic)',
    '链路': 'Signal path',
    '麦克风 → PureVox → CABLE In → CABLE Out → 其它软件':
      'Mic → PureVox → CABLE In → CABLE Out → other apps',
    'Windows 需要 VB-CABLE 虚拟声卡驱动：PureVox 把降噪输出送到 CABLE Input，其它软件从 CABLE Output 当麦克风取。':
      'Windows needs the VB-CABLE virtual audio driver: PureVox sends denoised output to CABLE Input; other apps take CABLE Output as a microphone.',
    '未检测到 VB-CABLE：点「驱动下载」安装后，点顶栏「刷新设备」重新检测。':
      'VB-CABLE not detected: click "Download driver", install it, then click "Refresh devices" in the top bar.',
    '驱动下载': 'Download driver',
    '视频教程': 'Video tutorial',
    '检测中…': 'Checking…',
    已安装: 'Installed',
    未安装: 'Not installed',
    创建: 'Create',
    移除: 'Remove',
    刷新: 'Refresh',
    已创建: 'Created',
    未创建: 'Not created',
    '正在处理…': 'Working…',
    '创建后，在其它软件里把「PureVox out」或「PureVox mic」设为麦克风即可。':
      'After creating, pick "PureVox out" or "PureVox mic" as the microphone in other apps.',
    不可用: 'Unavailable',
    正常: 'OK',
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
