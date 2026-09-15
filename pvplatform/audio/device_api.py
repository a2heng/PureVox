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

"""
平台感知的音频设备 API 枚举。

PortAudio 的 host API 类型编号因平台而异（WASAPI=13、PulseAudio=15、
ALSA=8、JACK=12、Core Audio=5…），且同一数值在另一平台毫无意义。
为避免把 Windows 的 13 硬编码到 Linux，本模块提供：

    resolve_api_names(api_type)  配置值 → 名字列表（不含平台回退）
    get_host_api_indices(p, api_type)  分级匹配 host API 索引（配置名 → 平台默认 → 全枚举）
    platform_default_api_type()  当前平台默认 API 数值
    get_api_options()            UI 下拉选项 [(label, type), ...]
    get_api_name(api_type)       类型 → 显示名

核心策略：配置存的 api_type 若在本机不存在（如 Windows 的 WASAPI=13
在 Linux 上），自动回退到平台默认。Linux 的采集/输出实际走本模块之外的
pipewire-pulse / libpulse（pwpipe_client），本模块的 host-API 分级仅用于
Windows WASAPI/MME。
"""

from .. import IS_WINDOWS, IS_LINUX, IS_MACOS

# 端口音频 host API 类型编号（PortAudio PaHostApiTypeId，跨平台固定不变）。
# 出处：PortAudio 官方 include/portaudio.h 的 PaHostApiTypeId 枚举
# （https://github.com/PortAudio/portaudio，master 及更早稳定版一致）：
#   paDirectSound=1、paMME=2、paASIO=3、paSoundManager=4、paCoreAudio=5、
#   paOSS=7、paALSA=8、paAL=9、paBeOS=10、paWDMKS=11、paJACK=12、
#   paWASAPI=13、paAudioScienceHPI=14、paAudioIO=15、paPulseAudio=16、paSndio=17
# 用法：枚举时按 dev['hostApi'] 匹配，避免把 Windows 的 13 硬编码含义串到别的平台。
API_DIRECTSOUND = 1
API_MME = 2
API_ASIO = 3
API_COREAUDIO = 5
API_OSS = 7
API_ALSA = 8
API_JACK = 12
API_WASAPI = 13
API_PULSE = 16
API_SNDIO = 17

# 网络输入模式（非 PortAudio host API）
API_NETWORK = 99

# Linux PipeWire（非 PortAudio host API；ALSA 备选接口已移除，仅保留配置键占位）
API_PIPEWIRE = 98


# 名字相似度模糊匹配的最低命中分数（best_name_match 兜底段）
FUZZY_NAME_MIN_SIM = 0.6


def normalize_device_name(name: str) -> str:
    """设备名归一化（用于比较）：转小写 + 字母数字 token 化 + 空白规整。

    PortAudio / WASAPI / PipeWire 同设备的显示名常有大小写、括号、
    厂商前缀差异（如 "Speakers (Realtek(R) Audio)" 与 "扬声器
    (Realtek(R) Audio)"）；归一化后比较只关心实质词，便于跨来源匹配。
    """
    import re
    if not name:
        return ""
    # 丢弃单字符 token（(R)/“(A)” 等注册标记噪音），避免干扰词序相似度
    toks = [t for t in re.findall(r"\w+", name.lower()) if len(t) >= 2]
    return " ".join(toks)


def name_similarity(a: str, b: str) -> float:
    """两个设备名的相似度（0~1）。综合 difflib 序列比 + token 重叠 + 前缀包含。"""
    na = normalize_device_name(a)
    nb = normalize_device_name(b)
    if not na or not nb:
        return 0.0
    if na == nb:
        return 1.0
    import difflib
    ratio = difflib.SequenceMatcher(None, na, nb).ratio()
    ta, tb = set(na.split()), set(nb.split())
    if ta or tb:
        overlap = 2.0 * len(ta & tb) / (len(ta) + len(tb))
    else:
        overlap = 0.0
    prefix = 0.9 if (na.startswith(nb) or nb.startswith(na)) else 0.0
    return max(ratio, overlap * 0.85, prefix)


def best_name_match(name: str, candidates) -> str:
    """在候选设备名里按名字相似度选最佳匹配（主设备选择与 AEC 校准共用）。

    顺序：归一化精确 → 前缀包含 → 相似度 ≥ FUZZY_NAME_MIN_SIM 的最高分。
    全部不命中返回 None（调用方自定回退策略，如首个设备 / 默认端点）。
    """
    if not name or not candidates:
        return None
    cands = [c for c in candidates if c]
    if not cands:
        return None
    na = normalize_device_name(name)
    if not na:
        return cands[0]
    for c in cands:
        if normalize_device_name(c) == na:
            return c
    for c in cands:
        nc = normalize_device_name(c)
        if nc.startswith(na):
            return c
    best = None
    best_s = FUZZY_NAME_MIN_SIM
    for c in cands:
        s = name_similarity(name, c)
        if s > best_s:
            best, best_s = c, s
    return best


def fix_device_name(name: str) -> str:
    """修复 PortAudio 在中文 Windows 上返回的乱码设备名。

    PortAudio 返回的设备名是 UTF-8 字节；PyAudio 按
    locale.getpreferredencoding()（中文系统是 cp936/GBK）先解码一次，
    只有解码抛异常才退回 UTF-8。UTF-8 的中文名恰能被 GBK 拼成合法字符时
    不会抛异常，于是得到「UTF-8 字节被 GBK 误读」的乱码串（如
    「线路输入」→「绾胯矾杈撳叆」）。同批设备里有的乱、有的正常，
    取决于各名字节能否拼成合法 GBK 序列。

    修复：把乱码串按 GBK 重新编码回原始 UTF-8 字节，再按 UTF-8 解码。
    对本来就是合法文本的字符串，GBK 编码再 UTF-8 解码要么抛异常、
    要么结果等于原串，故不影响正常名字。
    """
    if not name:
        return name
    try:
        fixed = name.encode("gbk").decode("utf-8")
    except (UnicodeEncodeError, UnicodeDecodeError):
        return name
    if fixed == name or "\ufffd" in fixed:
        return name
    return fixed

# 编号 → host API 显示名（仅列当前会用到的）
PTYPE_TO_NAME = {
    API_DIRECTSOUND: "DirectSound",
    API_MME: "MME",
    API_ASIO: "ASIO",
    API_COREAUDIO: "Core Audio",
    API_OSS: "OSS",
    API_ALSA: "ALSA",
    API_JACK: "JACK",
    API_WASAPI: "WASAPI",
    API_PULSE: "PulseAudio",
    API_SNDIO: "Sndio",
}

# API 类型 → 设备配置键后缀（配置 key 按接口隔离，如 input_device_wasapi）。
# Linux（pulse/alsa）与 Windows（wasapi/mme）设备名完全不一致，须分接口存。
# Linux 原生 PipeWire（API_PIPEWIRE）复用 pulse 后缀键（历史默认即存于此）。
API_CONFIG_SUFFIX = {
    API_DIRECTSOUND: "directsound",
    API_MME: "mme",
    API_ASIO: "asio",
    API_COREAUDIO: "coreaudio",
    API_OSS: "oss",
    API_ALSA: "alsa",
    API_JACK: "jack",
    API_WASAPI: "wasapi",
    API_PULSE: "pulse",
    API_SNDIO: "sndio",
    API_PIPEWIRE: "pulse",
}


def api_config_suffix(api_type: int) -> str:
    """API 类型 → 设备配置键后缀（如 wasapi / mme / pulse）。"""
    return API_CONFIG_SUFFIX.get(api_type, f"api{api_type}")


def platform_default_api_type() -> int:
    """返回当前平台默认的音频 API 类型编号。"""
    if IS_WINDOWS:
        return API_WASAPI
    if IS_LINUX:
        return API_PIPEWIRE
    if IS_MACOS:
        return API_COREAUDIO
    return API_ALSA


def platform_api_names() -> list:
    """当前平台候选 host API 名字（按优先级排序）。"""
    if IS_WINDOWS:
        return ["WASAPI"]
    if IS_LINUX:
        return ["PulseAudio", "ALSA"]
    if IS_MACOS:
        return ["Core Audio"]
    return ["ALSA"]


def get_api_name(api_type: int) -> str:
    """类型编号 → 显示名。网络模式返回 'NETWORK'。"""
    if api_type == API_NETWORK:
        return "NETWORK"
    if api_type == API_PIPEWIRE:
        return "PipeWire"
    return PTYPE_TO_NAME.get(api_type, f"API({api_type})")


def get_api_options() -> list:
    """返回 UI 下拉选项 [(label, type), ...]：本地接口 + 网络。

    Linux 单一本地接口 PipeWire（原生）；Windows 提供 WASAPI（默认）与
    MME（旧版备选）；macOS 仍单一本地接口 + 网络。
    """
    opts = []
    if IS_WINDOWS:
        opts.append(("本地接口 WASAPI（默认）", API_WASAPI))
        opts.append(("本地接口 MME", API_MME))
    elif IS_LINUX:
        opts.append(("本地接口 PipeWire（默认）", API_PIPEWIRE))
    else:
        opts.append(("本地设备", platform_default_api_type()))
    opts.append(("网络(API)", API_NETWORK))
    return opts


def resolve_api_names(api_type: int) -> list:
    """把配置的 api_type 解析为实际 host API 名字列表（不含平台回退）。

    规则：
      - 网络模式（99）→ 空列表（不走 PortAudio host API）。
      - 否则返回该 API 在 PortAudio 中的名字（如 WASAPI → ["WASAPI"]）。

    平台回退（配置的 API 在本机不存在时改用平台默认）由
    `get_host_api_indices` 分级处理：先匹配配置的 API 名，匹配不到再试
    `platform_api_names()`，最后全枚举兜底。
    """
    if api_type == API_NETWORK:
        return []
    if api_type in PTYPE_TO_NAME:
        return [PTYPE_TO_NAME[api_type]]
    return []


def get_host_api_indices(p, api_type: int) -> list:
    """按名字匹配 host API 索引列表（跨平台）。

    名字用大小写不敏感的子串匹配：PyAudio 的 host API 名带厂商前缀
    （WASAPI 实为 "Windows WASAPI"、"Windows DirectSound"、"Windows WDM-KS"），
    精确相等会匹配不到而误触兜底枚举全部。

    分级匹配：
      1. 配置的 API 名在本机存在 → 只返回该 API 的索引（选 MME 就只列 MME，
         不会混入 WASAPI 设备）；
      2. 配置的 API 不存在（如 Windows 的 WASAPI=13 在 Linux 上）→ 回退到
         平台默认候选（PulseAudio → ALSA）；
      3. 仍无匹配 → 全枚举兜底（保证虚拟 sink 等挂在任意 API 下的设备可枚举）。
    """
    def _match(names: list) -> list:
        needle_tokens = [n.lower() for n in names]
        indices = []
        for i in range(p.get_host_api_count()):
            try:
                info = p.get_host_api_info_by_index(i)
            except Exception:
                continue
            hay = (info.get('name') or '').lower()
            if any(tok in hay for tok in needle_tokens):
                indices.append(i)
        return indices

    indices = _match(resolve_api_names(api_type))
    if indices:
        return indices
    indices = _match(platform_api_names())
    if indices:
        return indices
    # 全部匹配不到 → 全枚举兜底（虚拟 sink 等跨 API 设备）
    return list(range(p.get_host_api_count()))
