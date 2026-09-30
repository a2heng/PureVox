# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""i18n 字符串表：中文字面量即 msgid。

- `zh` 下 `T(x)` 恒等返回 x（中文即原文，零开销）；
- `en` 下查 `_TABLES["en"]`，缺失时回退中文（缺失即漏译，review 一眼可见）；
- 插值串用命名占位符模板：`T("端口 {port}").format(port=…)`，占位符集合
  中英必须一致（tests/test_i18n.py 做奇偶校验）。

本模块是跨层共享的叶子模块（与 logger/config_manager 同类）：
不 import 任何项目内代码，不违反 L0–L4 分层。范围仅桌面 UI
（uitk/、经 UI 显示的 spec.label 与错误串）；日志串、注释、
pvengine/pvplatform 常量不翻译。
"""

# 支持的语言（code → 显示名；显示名即菜单项文案，天然双语不需要翻译）
LANGUAGES = {"zh": "中文", "en": "English"}

_LANG = "zh"

# 英文表：中文 msgid → English。占位符 {name} 集合必须与 msgid 一致。
_TABLES = {
    "en": {
        # ── 主窗 ──
        "（开发版）": " (dev build)",
        "启动音频处理": "Start Audio Processing",
        "停止音频处理": "Stop Audio Processing",
        "退出": "Quit",
        "添加 ▾": "Add ▾",
        "设置 ▾": "Settings ▾",
        "输入：未启动": "Input: not started",
        "输入": "Input",
        "输出": "Output",
        "处理": "FX",
        "可视化": "Visualizer",
        "（默认）": "(Default)",
        "模型输入音量在 VU 电平表黄色区效果最佳；\n"
        "输入音量过小会被当做噪音过滤。":
            "Best results when the model input level sits in the yellow "
            "zone of the VU meter;\na too-quiet input may be filtered out "
            "as noise.",
        "扬声器 | ": "Speaker | ",
        "麦克风 | ": "Mic | ",
        "扬声器": "Speaker",
        "麦克风": "Mic",
        "Far 延迟": "Far delay",
        "校准": "Calibrate",
        "校准中…": "Calibrating…",
        "系统声音": "System Sounds",
        "虚拟声卡": "Virtual Sound Card",
        "快捷键与提示音": "Hotkeys & Cues",
        "关于": "About",
        "启动时自动运行": "Auto-start processing on launch",
        "开机自启": "Start with OS",
        "已创建，请重启音频处理生效。":
            "Created. Restart audio processing to apply.",
        "已清理。": "Cleaned up.",
        "输入：{ch}ch {sr}Hz → 48kHz（自适应重采样）":
            "Input: {ch}ch {sr}Hz → 48kHz (adaptive resample)",
        "输入：48kHz（直通）": "Input: 48kHz (passthrough)",
        "输入：网络推流": "Input: network stream",
        "输入：无设备输入": "Input: no device",
        "输出：{ch}ch 48kHz → {sr}Hz（自适应重采样）":
            "Output: {ch}ch 48kHz → {sr}Hz (adaptive resample)",
        "输出：48kHz（直通）": "Output: 48kHz (passthrough)",
        "AEC far（{kind}）：{sr}Hz → 48kHz（自适应）":
            "AEC far ({kind}): {sr}Hz → 48kHz (adaptive)",
        "AEC far（{kind}）：48kHz（直通）":
            "AEC far ({kind}): 48kHz (passthrough)",
        "链已更新，但重启失败：\n{err}":
            "Chain updated, but restart failed:\n{err}",
        "请先关闭音频处理（并保持环境安静），再点击自动校准。":
            "Stop audio processing first (keep the room quiet), "
            "then click auto-calibrate.",
        "延迟校准失败，已保留原值。\n"
        "请保持环境安静并确认麦克风能听到扬声器测试音后重试。":
            "Calibration failed; the previous value was kept.\n"
            "Keep the room quiet, make sure the mic can hear the speaker "
            "test tone, then retry.",
        "以下快捷键已被系统或其他程序占用，未能生效：\n"
        "{keys}\n\n请在「设置 → 快捷键与提示音」中换一组。":
            "These shortcuts are taken by the system or another app and "
            "were not applied:\n{keys}\n\nPlease pick another combination "
            "in \"Settings → Hotkeys & Cues\".",
        # ── 添加菜单 ──
        "设备": "Devices",
        "媒体输入": "Media Input",
        "音效板": "Soundpad",
        "音乐播放器": "Music Player",
        "桌面声音输入": "Desktop Audio Input",
        # ── 节点 label（pvengine spec.label 中文常量，渲染点翻译）──
        "录音输入": "Microphone",
        "网络输入": "Network Input",
        "播放输入": "Playback Input",
        "AEC 输入": "AEC Input",
        "音频输出": "Audio Output",
        "VU 电平表": "VU Meter",
        "频谱图": "Spectrum",
        "增益": "Gain",
        "噪声门 VAD": "Noise Gate VAD",
        "均衡器": "Equalizer",
        "均衡器 EQ · 10 段": "EQ · 10 Band",
        "均衡器 EQ · 31 段": "EQ · 31 Band",
        "均衡器 EQ · 61 段": "EQ · 61 Band",
        "压缩器": "Compressor",
        "AI 降噪 · 小号": "AI Denoise · S",
        "AI 降噪 · 中号": "AI Denoise · M",
        "AI 降噪 · 大号": "AI Denoise · L",
        "AI 降噪 · 旧版 v6": "AI Denoise · Legacy v6",
        "目标说话人 TSE": "Target Speaker TSE",
        "自动增益 AGC": "Auto Gain AGC",
        # ── 参数滑杆 label（spec.params）──
        "音量 dB": "Volume dB",
        "最大增益 dB": "Max gain dB",
        "门限 dBFS": "Threshold dBFS",
        "开启 ms": "Open ms",
        "保持 ms": "Hold ms",
        "关闭 ms": "Close ms",
        "阈值 dB": "Threshold dB",
        "压缩比": "Ratio",
        "补偿 dB": "Makeup dB",
        # ── 行内按钮 / 卡片 ──
        "均衡器编辑": "Edit EQ",
        "参考录音": "Reference Audio",
        "添加音效": "Add Sound",
        "未命名": "Untitled",
        "音频/容器": "Audio/Container",
        "全部文件": "All Files",
        "该文件无法解码：\n{err}": "Cannot decode this file:\n{err}",
        "选择音乐/媒体文件": "Choose a music/media file",
        "导入中…": "Importing…",
        "（未选择曲目）": "(No track selected)",
        "选择曲目": "Choose Track",
        "播放": "Play",
        "暂停": "Pause",
        "捕获默认输出设备的系统混音（loopback），"
        "音量滑杆实时生效；随引擎启停自动开关。":
            "Captures the system mix of the default output device "
            "(loopback); the volume slider applies live; opens and closes "
            "with the engine.",
        "网络": "Network",
        "服务未启动（启动后显示状态）":
            "Service not started (status appears after start)",
        "手机/浏览器扫码或访问该地址推流（HTTPS，首次需信任自签证书）；"
        "实际监听端口 = 设置中的服务器端口。":
            "Scan the QR code or open this URL in a phone/browser to stream "
            "(HTTPS; trust the self-signed certificate on first use). "
            "Actual listening port = the server port in Settings.",
        "服务器启动中…": "Starting server…",
        "端口 {port} · 客户端 {clients} 个":
            "Port {port} · {clients} client(s)",
        "二维码\n不可用（{reason}）": "QR code\nunavailable ({reason})",
        "待检测 —— 启动或停止音频处理时自动检测":
            "Pending — checked automatically when audio processing "
            "starts or stops",
        "OBS/会议/语音等软件": "OBS / meetings / voice apps",
        "未检测到驱动：点「驱动下载」安装后启动即可识别。":
            "Driver not detected: click \"Download Driver\", install, "
            "then start audio processing to detect it.",
        " VB-CABLE 驱动 ": " VB-CABLE Driver ",
        "控制面板": "Control Panel",
        "驱动下载": "Download Driver",
        "视频教程": "Video Guide",
        "启动时检测 VB-CABLE 驱动安装（取消勾选不再弹框）":
            "Check VB-CABLE driver on launch (uncheck to stop asking)",
        "已安装": "Installed",
        "未安装": "Not installed",
        "未检测到 VB-CABLE 驱动：请先下载官方驱动包并安装，"
        "装好后点击「启动/停止音频处理」即可识别。":
            "VB-CABLE driver not detected: download and install the "
            "official driver package first; once installed, click "
            "Start/Stop Audio Processing to detect it.",
        "创建": "Create",
        "清理": "Remove",
        "已创建": "Created",
        "未创建": "Not created",
        "创建后，其它软件把「PureVox 虚拟麦克风」设为麦克风"
        "即可收到降噪声音。创建/清理均幂等。":
            "After creation, set \"PureVox Virtual Microphone\" as the "
            "microphone in other apps to receive the denoised sound. "
            "Create/remove are both idempotent.",
        "峰值 {pk} dB": "peak {pk} dB",
        "无 AGC 节点": "No AGC node",
        # ── 引擎/会话错误串 ──
        "当前平台没有可用的音频传输后端（所需能力: {caps}）":
            "No available audio transport backend on this platform "
            "(required capabilities: {caps})",
        "媒体输出设备打开失败": "Failed to open the media output device",
        "网络输入模式不支持回声消除行（AEC 需要本地麦克风）":
            "Network input mode does not support echo-cancel rows "
            "(AEC needs a local microphone)",
        "Windows 单输入后端：回声消除的麦克风须与音频输入为同一设备，"
        "请改选 {main_in}（{bad} 不符）":
            "Windows single-input backend: the AEC microphone must be the "
            "same device as the audio input; please switch to {main_in} "
            "({bad} mismatched)",
        "主输入设备": "the main input device",
        "网络服务器启动失败（服务端依赖缺失或加载失败，详见日志）":
            "Network server failed to start (server dependencies missing "
            "or failed to load; see the log)",
        "音频流创建超时": "Audio stream creation timed out",
        "音频流创建失败: {err}": "Audio stream creation failed: {err}",
        "远程推流节点已启用，但地址为空":
            "Network input node is enabled but its URL is empty",
        "未启用任何「音频输入」节点（回声消除/桌面输入/媒体输入节点亦可）":
            "No \"Audio Input\" node enabled (AEC / Desktop Input / "
            "media input nodes also count)",
        "未启用任何「音频输出」节点": "No \"Audio Output\" node enabled",
        # ── 对话框 ──
        "确定": "OK",
        "关于 {app}": "About {app}",
        "开发版": "dev build",
        "Windows 使用": "Windows Guide",
        "Linux 使用": "Linux Guide",
        "更新日志": "Changelog",
        "许可证": "License",
        "均衡器": "Equalizer",
        "低切": "Low-cut",
        "高切": "High-cut",
        "平直": "Flat",
        "低音增强": "Bass Boost",
        "人声增强": "Vocal Boost",
        "高音增强": "Treble Boost",
        "启停快捷键": "Start/Stop Hotkey",
        "启停提示音": "Start/Stop Cues",
        "启动提示音": "Start Cue",
        "停止提示音": "Stop Cue",
        "目标说话人 TSE · 参考音频":
            "Target Speaker TSE · Reference Audio",
        "已有参考：{name}\n{kb} KB · {mt}":
            "Existing reference: {name}\n{kb} KB · {mt}",
        "尚无参考音频——TSE 插件将直通。\n"
        "启动音频处理后点「开始录音」，对麦克风说 10 秒话。":
            "No reference audio yet — the TSE plugin passes through.\n"
            "Start audio processing, click \"Start Recording\" and speak "
            "into the mic for 10 seconds.",
        "请先启动音频处理，再录制参考。":
            "Start audio processing before recording a reference.",
        "录音中… {sec}s（请持续说话）":
            "Recording… {sec}s (keep talking)",
        "录音失败：10 秒内未捕获到音频"
        "（请确认音频处理已启动且麦克风有输入）":
            "Recording failed: no audio captured within 10 seconds "
            "(make sure audio processing is running and the mic has input)",
        "保存失败: {err}": "Save failed: {err}",
        "完成！参考已生效。": "Done! The reference is now active.",
        "已保存，但加载失败（模型或参考音频不可用）——请查看日志。":
            "Saved, but loading failed (model or reference audio unusable) "
            "— check the log.",
        "● 开始录音 (10s)": "● Start Recording (10s)",
        # ── 托盘 ──
        "PureVox — 运行中": "PureVox — Running",
        "PureVox — 已停止": "PureVox — Stopped",
        "打开 PureVox": "Open PureVox",
        # ── 热键字段 ──
        "按下组合键…": "Press a key combo…",
        "未设置": "Not set",
        "需带 Ctrl/Alt/Shift（或 F1–F24）":
            "Needs Ctrl/Alt/Shift (or F1–F24)",
    },
}


def set_language(code: str) -> None:
    """设置当前语言（未知 code 回退 zh）。"""
    global _LANG
    _LANG = code if code in LANGUAGES else "zh"


def get_language() -> str:
    return _LANG


def T(msgid: str) -> str:
    """翻译：zh 恒等返回；其他语言查表，缺失回退中文（漏译显眼）。"""
    if _LANG == "zh":
        return msgid
    return _TABLES.get(_LANG, {}).get(msgid, msgid)
