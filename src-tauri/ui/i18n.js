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

// 统一字符串管理（CONTRIBUTING.md）：本文件是唯一字符串源，中文即 msgid（zh 恒等返回，
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
    开发构建: 'dev build',
    发布日期: 'Published',
    安装包: 'Installer',
    更新说明: 'Release notes',
    重新下载: 'Download again',
    '{n} 行': '{n} lines',
    '这个 release 没有更新说明': 'This release has no notes',
    没有匹配到本机安装方式可用的包: 'No package matched this install kind',
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
    开机自启已开启: 'Start with Windows enabled',
    开机自启已关闭: 'Start with Windows disabled',
    '开机自启设置失败：': 'Failed to change start with Windows: ',
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
    // AEC 校准结果面板（calib.js；数据来自快照 calib_report）
    校准结果: 'Calibration result',
    '麦克风（近端，1.5–2.5 kHz 包络）': 'Microphone (near, 1.5–2.5 kHz envelope)',
    '远端参考（回环）': 'Far reference (loopback)',
    回声路径: 'Echo path',
    近端电平: 'Near level',
    远端电平: 'Far level',
    下一步: 'Next step',
    '复制 JSON': 'Copy JSON',
    结果: 'Result',
    '查看最近一次校准结果（含曲线与结论）': 'Show the latest calibration result (curves + verdict)',
    成功: 'Success',
    '成功：峰值匹配完成，延时与增益已应用。':
      'Success: peak match done, delay and gains applied.',
    '失败：麦克风没有听到探针。': 'Failed: the microphone did not hear the probe.',
    '失败：远端参考没有数据。': 'Failed: the far reference produced no data.',
    '失败：采集数据太短。': 'Failed: capture too short.',
    '没有曲线数据（采集超时或数据太短）': 'No curve data (capture timed out or too short)',
    '采集时两路峰值差 {raw} ms': 'peak difference {raw} ms at capture',
    '比底噪高 {snr} dB': '{snr} dB above its own floor',
    '已应用 {g} dB': 'applied {g} dB',
    '远端 → 麦克风': 'far → mic',
    '麦克风峰值 {t} ms': 'Mic peak {t} ms',
    '远端峰值 {t} ms': 'Far peak {t} ms',
    '麦克风峰值 {db} dBFS': 'Mic peak {db} dBFS',
    '远端峰值 {db} dBFS': 'Far peak {db} dBFS',
    '延时差 {d} ms': 'Δ {d} ms',
    '第 {c} 列第 {r} 行': 'column {c}, row {r}',
    '麦克风 {mic} ｜ 远端 {far}': 'Mic {mic} | far {far}',
    '探针带 {lo}–{hi} Hz；曲线 = 21 ms 帧能量的 13 帧（约 35 ms）平滑（不是原始波形）':
      'Probe band {lo}–{hi} Hz; curve = 13-frame (~35 ms) smoothing of 21 ms frame energy (not the raw waveform)',
    '文件：{path}': 'Files: {path}',
    '已复制 JSON': 'JSON copied',
    '读取参考状态…': 'Reading reference…',
    '参考语音：录制于 {time}': 'Reference recorded at {time}',
    '尚无参考语音（TSE 直通）': 'No reference yet (TSE passes through)',
    '参考状态读取失败：': 'Failed to read reference status: ',
    'TSE 介绍：只保留目标说话人的声音。先录 10 秒自己的说话声作为参考，之后按这个声音过滤其他人；录制前请先启动音频处理，并对着麦克风持续说话。':
      'TSE: keeps only the target speaker. Record 10 s of your own voice as the reference; TSE then filters out other voices. Start the audio engine and keep speaking into the microphone while recording.',
    '● 开始录音 (10s)': '● Record (10 s)',
    '打开参考语音文件夹': 'Open reference folder',
    '已打开参考语音文件夹': 'Reference folder opened',
    '打开参考语音文件夹失败：': 'Could not open the reference folder: ',
    '参考语音已录制，TSE 已生效': 'Reference recorded — TSE is active',
    '录音中… {sec}s（请持续说话）': 'Recording… {sec}s (keep speaking)',
    '请先启动音频处理，再录制参考。': 'Start the audio engine before recording a reference.',
    '校准完成：延时 {d} ms': 'Calibration done — delay {d} ms',
    '曲目已导入': 'Track imported',
    '网络服务已启动': 'Network service started',
    '网络服务已停止': 'Network service stopped',
    '网络服务已重启': 'Network service restarted',
    '已复制客户端地址': 'Client address copied',
    '切换网卡失败：': 'Could not switch network interface: ',
    '端口已改为 {p}': 'Port changed to {p}',
    '端口要在 1024~65535 之间': 'Port must be between 1024 and 65535',
    '改端口失败：': 'Could not change the port: ',
    '虚拟麦克风已创建': 'Virtual microphone created',
    '虚拟麦克风已移除': 'Virtual microphone removed',
    '虚拟驱动操作失败：': 'Virtual driver operation failed: ',
    '设置已保存': 'Settings saved',
    '检查更新失败：': 'Update check failed: ',
    '下载更新失败：': 'Update download failed: ',
    '安装更新失败：': 'Update install failed: ',
    '打开链接失败：': 'Could not open the link: ',
    'AI 麦克风降噪': 'AI MIC noise reduction',
    会话: 'Session',
    音频链: 'Audio chain',
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
    '送扫频探针：测延时并自动配平近端 / 远端': 'Sweep probe: measure delay and auto-match near / far levels',
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
    '消除麦克风中的扬声器声音。校准时请先关闭音频播放，并把扬声器音量调到正常使用时的音量。':
      'Removes the speaker sound picked up by the microphone. Before calibrating, stop any playback and set the speaker volume to the level you normally use.',
    // 调试列（静态小节）
    调试: 'Debug',
    应用: 'App',
    版本: 'Version',
    调试接口: 'Debug HTTP',
    内存: 'Memory',
    逻辑核数: 'Logical cores',
    适配器: 'Adapter',
    整卡占用: 'GPU util',
    本进程占用: 'GPU proc',
    本进程显存: 'VRAM proc',
    音频: 'Audio',
    引擎: 'Engine',
    延时校准: 'Delay calib',
    网络: 'Network',
    '网络麦克风': 'Network microphone',
    '网络扬声器': 'Network speaker',
    '无 Tauri 运行时': 'No Tauri runtime',
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
    没有订阅: 'No subscribers',
    '待发 {n} hop': '{n} hop pending',
    '（丢 {n}）': ' ({n} dropped)',
    上行: 'Uplink',
    '（测时延中）': '(measuring latency)',
    '水位 {a}/{b} ms': 'level {a}/{b} ms',
    '丢包 {n}': '{n} lost',
    手机环: 'Client ring',
    '最大间隔 {n} ms': 'max gap {n} ms',
    '裁 {n} 格': '{n} trimmed',
    '空环到达 {n}': '{n} into empty ring',
    时钟偏差测时中: 'clock drift measuring',
    '时钟 {n} ppm': 'clock {n} ppm',
    'PLC {n}': 'PLC {n}',
    '下行 {n} ms': 'downlink {n} ms',
    '欠载 {n}': '{n} underruns',
    '掩盖 {n}': '{n} concealed',
    '（客户端未上报）': '(no client report)',
    // 虚拟驱动页（drivers_linux.js）：缺 pactl 时「真源」建不出来，要说清缺哪个包
    '缺 pactl：装上 pulseaudio-utils 才有真源':
      'pactl is missing: install pulseaudio-utils to get the real source',
    // 网络行卡片（netrow.js）——手机相关的操作全在这里，顶栏不再有独立面板
    '启动中': 'Starting',
    '停止中': 'Stopping',
    '重启中': 'Restarting',
    // 卡片顶栏那一句短状态（全文在 `/debug` 的 `net` 段，不塞进卡片）
    就绪: 'Ready',
    未启动: 'Not running',
    '状态不可用': 'Status unavailable',
    '编解码器不可用': 'Codec unavailable',
    '客户端': 'Clients',
    '订阅': 'Subscribed',
    '{n} 客户端': '{n} client(s)',
    '{n} 订阅': '{n} subscribed',
    // 二维码右边的紧凑控件（2026-10-27：右边只放能点的东西）
    证书: 'Certificate',
    AEC: 'AEC',
    缓冲: 'Buffer',
    '{ms} ms': '{ms} ms',
    '远程输入在这个平台不可用：': 'Remote input is not available on this platform: ',
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
    '抖动缓冲（手机缓冲帧数，1 帧 = 10 ms）': 'Jitter buffer (frames on the phone, 1 frame = 10 ms)',
    '端口被占用': 'Port already in use',
    '向远端送扫频探针，测出延时与两端电平并回填': 'Send a sweep to the far end; measures delay and both levels, then fills them in',
    '参数控件未就绪': 'Parameter controls not ready',
    '模型': 'Model',
    '抖动缓冲已改为 {ms} ms': 'Jitter buffer set to {ms} ms',
    '改抖动缓冲失败：': 'Failed to set jitter buffer: ',
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
    '系统默认输出': 'System default output',
    '网络出站（本机刚发出的音频）': 'Network outbound (audio this PC just sent)',
    '设备已更新（{a} 入 / {b} 出）': 'Devices updated ({a} in / {b} out)',
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
    // 调试列（动态值 / 提示）
    '（无数据）': '(no data)',
    '采集中…': 'Collecting…',
    '不可用：': 'Unavailable: ',
    '采样于 {time}': 'Sampled at {time}',
    '最近 {last}  最小 {min}  最大 {max}': 'last {last}  min {min}  max {max}',
    '波形（48 kHz，最近 50 ms）': 'Waveform (48 kHz, last 50 ms)',
    '波形（48 kHz，最近 50 ms，送入重采样前）': 'Waveform (48 kHz, last 50 ms, into resampler)',
    '输入：{name}': 'Input: {name}',
    '输出：{name}': 'Output: {name}',
    '取快照失败：': 'Snapshot failed: ',
    '渲染快照失败：': 'Failed to render the snapshot: ',
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
    麦克风: 'mic', // 左上角标签列只有几 em 宽，'Microphone' 会把整列撑开
    // 「设备」在调试列已有同一条（第 331 行），这里不重复，引用同一条翻译。
    '测试音（1kHz 测试用）': 'Test tone (1 kHz test)',
    'AEC（回声消除）': 'AEC (echo cancel)',
    // 可视化插件（处理行 viz）与调试面板的 VU / 频谱（ui/viz.js）
    // 31 段均衡 / AGC（处理行；Rust 注册表标签，界面经 T() 翻译）
    '31 段均衡': '31-band EQ',
    '预设 {n}': 'Preset {n}',
    '目标电平：输出响度（-20 正常 / -14 更响）': 'Target level: output loudness (-20 normal / -14 louder)',
    '跟随速度：0.001 稳 / 0.01 快': 'Follow speed: 0.001 stable / 0.01 fast',
    '底噪门限：-55 默认（环境吵用 -45）': 'Noise floor: -55 default (-45 in noisy rooms)',
    '长按这个按钮保存当前设置': 'Long-press this button to save the current settings',
    '单击应用；长按覆盖保存': 'Click to apply; long-press to overwrite',
    '长按保存当前设置': 'Long-press to save the current settings',
    '已保存到自定义 {n}': 'Saved to custom {n}',
    '在曲线上拖动：左右选频段，上下改增益': 'Drag on the curve: left/right picks a band, up/down changes gain',
    归零: 'Reset',
    低切: 'Low cut',
    高切: 'High cut',
    'EQ 下发失败：': 'Failed to send the EQ: ',
    平坦: 'Flat',
    人声: 'Vocal',
    音乐: 'Music',
    低音: 'Bass',
    高音: 'Treble',
    播客: 'Podcast',
    均衡设置: 'EQ settings',
    '自动增益 AGC': 'Auto gain (AGC)',
    目标电平: 'Target level',
    跟随速度: 'Follow speed',
    底噪门限: 'Noise floor',
    可视化: 'Visualization',
    显示内容: 'Display',
    '电平 + 频谱': 'Level + spectrum',
    '电平（VU）': 'Level (VU)',
    频谱: 'Spectrum',
    电平: 'Level',
    已停用: 'Disabled',
    '状态不可用：': 'Status unavailable: ',
    '频谱（ISO 61 段，20 Hz ~ 20 kHz，对数轴，0 ~ -90 dBFS）':
      'Spectrum (ISO 61-band, 20 Hz ~ 20 kHz, log axis, 0 ~ -90 dBFS)',
    降噪: 'Denoise',
    增益: 'Gain',
    '目标说话人 TSE': 'Target speaker TSE',
    扬声器: 'Speaker',
    音效板: 'Soundpad',
    音乐播放器: 'Music player',
    远端设备: 'Far device',
    远端延时: 'Far delay',
    '增益(dB)': 'Gain (dB)',
    '音量(dB)': 'Volume (dB)',
    音效: 'Sound',
    曲目: 'Track',
    续播位置: 'Resume at',
    '降噪 202609b（现役）': 'Denoise 202609b (current)',
    '降噪 202606（旧版）': 'Denoise 202606 (legacy)',
    'TSE 202609c（现役）': 'TSE 202609c (current)',
    'AEC 202610（现役）': 'AEC 202610 (current)',
    柔和: 'Soft',
    清脆: 'Crisp',
    气泡: 'Pop',
    电子: 'Blip',
    木鱼: 'Wood',
    风铃: 'Chime',
    // 驱动（按平台切换：Linux 虚拟驱动 / Windows VB-CABLE）
    虚拟驱动: 'Virtual driver',
    驱动: 'Drivers',
    系统声音: 'System sound',
    '已打开系统声音面板': 'Opened the system sound panel',
    '打开系统声音面板失败：': 'Failed to open the system sound panel: ',
    'VB 控制面板': 'VB control panel',
    '已请求管理员权限打开 VB-CABLE 控制面板':
      'Requested admin rights to open the VB-CABLE control panel',
    '打开 VB-CABLE 控制面板失败：': 'Failed to open the VB-CABLE control panel: ',
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
      zh: '完成：参考语音已保存（{a} s，RMS {b} dBFS，峰值 {c} dBFS）',
      en: 'Done: reference saved ({a} s, RMS {b} dBFS, peak {c} dBFS)',
    },
    { id: 'cal.collect', zh: '采集中 {a}/{b} s', en: 'Capturing {a}/{b} s' },
    { id: 'cal.compute', zh: '计算中…', en: 'Computing…' },
    {
      id: 'cal.result',
      // 与 `engine/calib.rs` 的完成串**逐字同构**：只多了「峰值匹配频窗」这个占位。
      // 骨架对不上时前端拿不到 `cal.result`，也就不会回填延时/增益（2026-10-26 踩过：
      // 改了文案忘了这里，界面上结果出得来、滑块却一动不动）。
      zh: '延时 {d} ms（峰值匹配 {b0}–{b1} Hz，麦克风窗内 +{s} dB）｜近端 {m} dB｜远端 {f} dB',
      en: 'Delay {d} ms (peak match {b0}–{b1} Hz, mic +{s} dB in band), near {m} dB, far {f} dB',
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
