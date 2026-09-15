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

# config_manager.py
import json
import os
import sys
from dataclasses import dataclass, field
from typing import Any, Dict, List


def _default_api_type() -> int:
    """平台默认音频 API 类型（PipeWire=98 / ALSA=8 / WASAPI=13 / CoreAudio=5）。"""
    if sys.platform.startswith("linux"):
        return 98  # API_PIPEWIRE（Linux 默认原生 PipeWire，备选 ALSA=8）
    if sys.platform.startswith("darwin"):
        return 5   # API_TYPE_COREAUDIO
    return 13      # API_TYPE_WASAPI


@dataclass
class ConfigDefaults:
    """默认配置常量。

    设备键按接口隔离、显式写全（阅读直观，不做动态生成）：
    每个接口都有 input / output / monitor 三个设备键，另有 AEC far 端键。
    接口后缀与 `device_api.API_CONFIG_SUFFIX` 一致。
    """
    # 模式
    mode: str = "denoise"  # "off" / "denoise" / "tse"
    # 增益
    pre_gain_db: float = 0.0
    # AGC / VAD
    agc_enabled: bool = False
    vad_enabled: bool = False
    compressor_enabled: bool = False
    # 均衡器状态（增益/高切/低切）存各 eq 插件节点的 params（随 plugin_chain 持久化）
    # 插件链（右侧面板，全部处理以插件形式存在）：[{"type","enabled","params"}, ...]
    plugin_chain: List[dict] = field(default_factory=lambda: [
        {"type": "audio_input", "enabled": True, "params": {"device": ""}},
        {"type": "denoiser", "enabled": True, "params": {}},
        {"type": "audio_output", "enabled": True, "params": {"device": ""}},
        {"type": "vu_meter", "enabled": True, "params": {}},
        {"type": "spectrum", "enabled": True, "params": {}},
    ])
    # 接口
    api_type: int = field(default_factory=_default_api_type)
    NETWORK_input_url: str = "ws://0.0.0.0:59123/ws/audio"
    monitor_enabled: bool = False

    # ── 设备键（按接口隔离；默认全留空，UI 缺省强制选枚举列表第一个）──
    # WASAPI（Windows 默认接口）
    input_device_wasapi: str = ""
    output_device_wasapi: str = ""
    monitor_device_wasapi: str = ""
    aec_far_sink_wasapi: str = ""
    # MME（Windows 旧版接口）
    input_device_mme: str = ""
    output_device_mme: str = ""
    monitor_device_mme: str = ""
    aec_far_sink_mme: str = ""
    # PulseAudio（Linux 默认接口）
    input_device_pulse: str = ""
    output_device_pulse: str = ""
    monitor_device_pulse: str = ""
    aec_far_sink_pulse: str = ""
    # ALSA（Linux 备选接口）
    input_device_alsa: str = ""
    output_device_alsa: str = ""
    monitor_device_alsa: str = ""
    aec_far_sink_alsa: str = ""
    # DirectSound（Windows）
    input_device_directsound: str = ""
    output_device_directsound: str = ""
    monitor_device_directsound: str = ""
    aec_far_sink_directsound: str = ""
    # ASIO
    input_device_asio: str = ""
    output_device_asio: str = ""
    monitor_device_asio: str = ""
    aec_far_sink_asio: str = ""
    # Core Audio（macOS 默认接口）
    input_device_coreaudio: str = ""
    output_device_coreaudio: str = ""
    monitor_device_coreaudio: str = ""
    aec_far_sink_coreaudio: str = ""
    # OSS
    input_device_oss: str = ""
    output_device_oss: str = ""
    monitor_device_oss: str = ""
    aec_far_sink_oss: str = ""
    # JACK
    input_device_jack: str = ""
    output_device_jack: str = ""
    monitor_device_jack: str = ""
    aec_far_sink_jack: str = ""
    # Sndio
    input_device_sndio: str = ""
    output_device_sndio: str = ""
    monitor_device_sndio: str = ""
    aec_far_sink_sndio: str = ""

    # TSE 参考音频
    tse_reference_wav_path: str = ""
    # 服务器
    server_enabled: bool = False
    server_port: int = 59123
    # 启动 / 快捷键 / 提示音
    auto_start: bool = False
    registry_auto_start: bool = False
    # 启停全局热键（规范串，空串=不监听；见 uitk.hotkeys）
    hotkey_toggle: str = "Alt+."
    # 启停提示音：总开关 + 启动/停止各自预设 id（见 pvengine.cues）
    cue_enabled: bool = True
    cue_start: str = "soft"
    cue_stop: str = "soft"
    # VB-CABLE 检测（Windows 虚拟声卡）：False 表示用户勾选了"不再提示"
    vbcable_check_enabled: bool = True

    @classmethod
    def to_dict(cls) -> Dict[str, Any]:
        """将默认配置转换为字典（设备键按接口显式写全）。"""
        instance = cls()
        return {
            "mode": instance.mode,
            "pre_gain_db": instance.pre_gain_db,
            "agc_enabled": instance.agc_enabled,
            "vad_enabled": instance.vad_enabled,
            "compressor_enabled": instance.compressor_enabled,
            "api_type": instance.api_type,
            "NETWORK_input_url": instance.NETWORK_input_url,
            "monitor_enabled": instance.monitor_enabled,
            # WASAPI
            "input_device_wasapi": instance.input_device_wasapi,
            "output_device_wasapi": instance.output_device_wasapi,
            "monitor_device_wasapi": instance.monitor_device_wasapi,
            "aec_far_sink_wasapi": instance.aec_far_sink_wasapi,
            # MME
            "input_device_mme": instance.input_device_mme,
            "output_device_mme": instance.output_device_mme,
            "monitor_device_mme": instance.monitor_device_mme,
            "aec_far_sink_mme": instance.aec_far_sink_mme,
            # PulseAudio
            "input_device_pulse": instance.input_device_pulse,
            "output_device_pulse": instance.output_device_pulse,
            "monitor_device_pulse": instance.monitor_device_pulse,
            "aec_far_sink_pulse": instance.aec_far_sink_pulse,
            # ALSA
            "input_device_alsa": instance.input_device_alsa,
            "output_device_alsa": instance.output_device_alsa,
            "monitor_device_alsa": instance.monitor_device_alsa,
            "aec_far_sink_alsa": instance.aec_far_sink_alsa,
            # DirectSound
            "input_device_directsound": instance.input_device_directsound,
            "output_device_directsound": instance.output_device_directsound,
            "monitor_device_directsound": instance.monitor_device_directsound,
            "aec_far_sink_directsound": instance.aec_far_sink_directsound,
            # ASIO
            "input_device_asio": instance.input_device_asio,
            "output_device_asio": instance.output_device_asio,
            "monitor_device_asio": instance.monitor_device_asio,
            "aec_far_sink_asio": instance.aec_far_sink_asio,
            # Core Audio
            "input_device_coreaudio": instance.input_device_coreaudio,
            "output_device_coreaudio": instance.output_device_coreaudio,
            "monitor_device_coreaudio": instance.monitor_device_coreaudio,
            "aec_far_sink_coreaudio": instance.aec_far_sink_coreaudio,
            # OSS
            "input_device_oss": instance.input_device_oss,
            "output_device_oss": instance.output_device_oss,
            "monitor_device_oss": instance.monitor_device_oss,
            "aec_far_sink_oss": instance.aec_far_sink_oss,
            # JACK
            "input_device_jack": instance.input_device_jack,
            "output_device_jack": instance.output_device_jack,
            "monitor_device_jack": instance.monitor_device_jack,
            "aec_far_sink_jack": instance.aec_far_sink_jack,
            # Sndio
            "input_device_sndio": instance.input_device_sndio,
            "output_device_sndio": instance.output_device_sndio,
            "monitor_device_sndio": instance.monitor_device_sndio,
            "aec_far_sink_sndio": instance.aec_far_sink_sndio,
            "tse_reference_wav_path": instance.tse_reference_wav_path,
            "server_enabled": instance.server_enabled,
            "server_port": instance.server_port,
            "auto_start": instance.auto_start,
            "registry_auto_start": instance.registry_auto_start,
            "hotkey_toggle": instance.hotkey_toggle,
            "cue_enabled": instance.cue_enabled,
            "cue_start": instance.cue_start,
            "cue_stop": instance.cue_stop,
            "vbcable_check_enabled": instance.vbcable_check_enabled,
            "plugin_chain": instance.plugin_chain,
        }


class ConfigManager:
    """应用配置管理器。"""

    def __init__(self, config_path: str) -> None:
        """使用配置文件路径初始化配置管理器。

        参数:
            config_path: 配置文件路径。
        """
        self._config_path: str = config_path
        self._config: Dict[str, Any] = ConfigDefaults.to_dict()
        self.load_config()

    def load_config(self) -> None:
        """从文件加载配置；文件缺失/损坏/为空时使用默认值。
        强配置：不做旧配置迁移，只保留已知配置键，不认识的一律删除并回退默认。"""
        if not os.path.exists(self._config_path):
            return
        try:
            with open(self._config_path, "r", encoding="utf-8") as f:
                content = f.read().strip()
                if not content:
                    return
                loaded_config: Dict[str, Any] = json.loads(content)

            defaults = ConfigDefaults.to_dict()
            # 强配置：不做任何旧配置迁移，只保留 defaults 中的已知键，
            # 未知/缺失键一律丢弃并回退默认值
            cleaned = {k: v for k, v in loaded_config.items() if k in defaults}
            self._config = {**defaults, **cleaned}
        except (json.JSONDecodeError, OSError):
            return

    # 键输出顺序：设备在前，EQ 在后（设备键按接口隔离，显式写全）
    _KEY_ORDER = [
        "mode",
        "pre_gain_db",
        "agc_enabled", "vad_enabled", "compressor_enabled",
        "api_type",
        "NETWORK_input_url", "monitor_enabled",
        # WASAPI
        "input_device_wasapi", "output_device_wasapi",
        "monitor_device_wasapi", "aec_far_sink_wasapi",
        # MME
        "input_device_mme", "output_device_mme",
        "monitor_device_mme", "aec_far_sink_mme",
        # PulseAudio
        "input_device_pulse", "output_device_pulse",
        "monitor_device_pulse", "aec_far_sink_pulse",
        # ALSA
        "input_device_alsa", "output_device_alsa",
        "monitor_device_alsa", "aec_far_sink_alsa",
        # DirectSound
        "input_device_directsound", "output_device_directsound",
        "monitor_device_directsound", "aec_far_sink_directsound",
        # ASIO
        "input_device_asio", "output_device_asio",
        "monitor_device_asio", "aec_far_sink_asio",
        # Core Audio
        "input_device_coreaudio", "output_device_coreaudio",
        "monitor_device_coreaudio", "aec_far_sink_coreaudio",
        # OSS
        "input_device_oss", "output_device_oss",
        "monitor_device_oss", "aec_far_sink_oss",
        # JACK
        "input_device_jack", "output_device_jack",
        "monitor_device_jack", "aec_far_sink_jack",
        # Sndio
        "input_device_sndio", "output_device_sndio",
        "monitor_device_sndio", "aec_far_sink_sndio",
        "tse_reference_wav_path",
        "server_enabled", "server_port",
        "auto_start", "registry_auto_start",
        "hotkey_toggle", "cue_enabled", "cue_start", "cue_stop",
        "vbcable_check_enabled",
        "plugin_chain",
    ]

    def save_config(self) -> None:
        """保存配置到文件。门卫模式：只写入已知键。"""
        dir_path = os.path.dirname(self._config_path)
        if dir_path and not os.path.exists(dir_path):
            os.makedirs(dir_path, exist_ok=True)

        defaults = ConfigDefaults.to_dict()
        # 白名单：只保留 defaults 中的键
        allowed = set(defaults.keys())

        ordered: Dict[str, Any] = {}
        for k in self._KEY_ORDER:
            if k in self._config and k in allowed:
                ordered[k] = self._config[k]

        with open(self._config_path, "w", encoding="utf-8") as f:
            json.dump(ordered, f, ensure_ascii=False, indent=2)

    def get(self, key: str, default: Any = None) -> Any:
        """按键获取配置值。

        参数:
            key: 配置键名。
            default: 键不存在时返回的默认值。

        返回:
            配置值或默认值。
        """
        return self._config.get(key, default)

    def set(self, key: str, value: Any) -> None:
        """设置配置值。

        参数:
            key: 配置键名。
            value: 要设置的值。
        """
        self._config[key] = value

    def get_all(self) -> Dict[str, Any]:
        """获取全部配置的副本。

        返回:
            全部配置值的副本。
        """
        return self._config.copy()
