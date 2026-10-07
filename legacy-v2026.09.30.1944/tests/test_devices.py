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

"""设备面检测（物理设备枚举 / 虚拟设备建立与使用）：
python tests/test_devices.py

环境自适应三档（全部无崩溃、优雅降级优先）：
1. 恒跑（CI 容器/无音频环境）：枚举 API 优雅返回（无 pw-dump / 无设备
   → 空列表/空串/False，不抛异常）；设备配置键表完整（10 接口 × 4 方向）；
   SpeakerCapture 工厂生命周期安全（start 失败不残留）；
2. 有音频设备的环境（开发机/带声卡 runner）：枚举一致性、默认设备合法、
   Windows 精确名/前缀匹配 get_device_id；
3. 虚拟麦克风真实建立→使用→卸载（Linux + PipeWire）：**需显式开启**
   `PUREVOX_TEST_VIRTUAL_MIC=1`——会真实创建 purevox_out/purevox_mic，
   测毕卸载（CI 容器无 PipeWire 自动跳过，走降级档）。
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

_IS_LINUX = sys.platform.startswith("linux")
_IS_WIN = sys.platform.startswith("win")


class _Log:
    def __call__(self, m):
        print("    [设备] " + str(m))

    def msg(self, m):
        self(m)

    def warn(self, m):
        self("WARN " + str(m))

    def err(self, m):
        self("ERR " + str(m))

    def sys(self, m):
        self(m)

    def dev(self, m):
        pass


def test_enum_graceful():
    """枚举 API 在任何环境恒真：返回类型正确，无设备环境优雅为空。"""
    from pvplatform.audio.pwpipe_client import (
        list_sources, list_destinations, node_description,
        source_label, dest_label)
    src = list_sources()
    dst = list_destinations()
    assert isinstance(src, list) and all(isinstance(s, str) for s in src)
    assert isinstance(dst, list) and all(isinstance(s, str) for s in dst)
    # 枚举净化：绝不包含 PureVox 自身流与对外虚拟源（回授防护）
    assert not any(s.startswith("PureVox-") for s in src)
    assert not any(s.startswith("purevox") for s in src)
    assert not any(d.startswith("PureVox-") for d in dst)
    # 不存在的节点名 → 优雅回退为名字本身
    assert node_description("__no_such_node__") == "__no_such_node__"
    assert isinstance(source_label("x"), str)
    assert isinstance(dest_label("x"), str)
    print(f"  枚举优雅: sources={len(src)} destinations={len(dst)}  OK")


def test_defaults_coherent():
    """默认设备选取与枚举自洽（无设备时允许空串）。"""
    from pvplatform.audio.pwpipe_client import (
        list_sources, list_destinations,
        default_mic_name, default_sink_name, speaker_sink_name)
    src = list_sources()
    dst = list_destinations()
    mic = default_mic_name()
    sink = default_sink_name()
    spk = speaker_sink_name()
    assert mic == "" or mic in src
    assert sink == "" or sink in dst
    # AEC 兜底扬声器绝不能指向 PureVox 自身的虚拟输出
    assert spk == "" or not spk.startswith("purevox")
    print(f"  默认设备自洽: mic={mic!r} sink={sink!r} speaker={spk!r}  OK")


def test_config_keys_complete():
    """强配置键表完整：10 个接口后缀 × input/output/monitor/aec_far_sink 全写全。"""
    from pvplatform.audio.device_api import API_CONFIG_SUFFIX
    import config_manager as cm
    cfg = vars(cm.ConfigDefaults())
    suffixes = set(API_CONFIG_SUFFIX.values())
    for sfx in sorted(suffixes):
        for direction in ("input_device_", "output_device_",
                          "monitor_device_", "aec_far_sink_"):
            key = direction + sfx
            assert key in cfg, f"配置键缺失: {key}"
    assert API_CONFIG_SUFFIX[98] == "pulse"     # 「自动(API)」别名 → pulse
    print(f"  设备配置键完整: {len(suffixes)} 接口 × 4 方向  OK")


def test_speaker_capture_lifecycle():
    """AEC 扬声器采集工厂：创建/start/read/stop 全安全（无设备环境 start=False）。"""
    from pvplatform.audio import create_speaker_capture
    cap = create_speaker_capture()
    started = cap.start()
    assert isinstance(started, bool)
    if started:
        data = cap.read(480)
        assert data is None or isinstance(data, list)
        assert isinstance(cap.dev_sr, int) and cap.dev_sr > 0
    cap.flush()
    cap.stop()
    assert cap.active is False
    print(f"  SpeakerCapture 生命周期安全 (started={started})  OK")


def test_devices_present():
    """有音频设备环境的枚举一致性（无设备时跳过）。平台感知：
    Linux = pw-dump 节点枚举；Windows = PortAudio host API 枚举。"""
    if _IS_WIN:
        from audio_processor import get_device_names, default_api_type
        api = default_api_type()
        try:
            src, dst = get_device_names(api_type=api)
        except Exception:
            src, dst = [], []
    else:
        from pvplatform.audio.pwpipe_client import (
            list_sources as src_fn, list_destinations as dst_fn)
        src, dst = src_fn(), dst_fn()
    if not (src or dst):
        print("  跳过（本环境无音频设备）")
        return
    assert all(isinstance(s, str) and s for s in src)
    assert all(isinstance(s, str) and s for s in dst)
    print(f"  枚举一致性: {len(src)} 输入 / {len(dst)} 输出  OK")


def test_name_fuzzy_match():
    """设备名模糊匹配纯函数（跨平台恒跑）：精确/前缀/大小写与标点差异/
    相似度命中；无关名返回 None。AEC far 端点选择与 get_device_id 共用。"""
    from pvplatform.audio.device_api import (
        best_name_match, name_similarity, normalize_device_name)
    cands = ["Speakers (Realtek(R) Audio)", "麦克风 (Realtek(R) Audio)",
             "CABLE Input (VB-Audio Virtual Cable)"]
    # 归一化精确（大小写/括号差异）
    assert best_name_match("speakers (realtek audio)", cands) == \
        "Speakers (Realtek(R) Audio)"
    # 词序不同但同实质（模糊相似）
    assert best_name_match("Realtek Speakers", cands) == \
        "Speakers (Realtek(R) Audio)"
    # 前缀
    assert best_name_match("Speakers", cands) == "Speakers (Realtek(R) Audio)"
    # 无关 → None（不误配到 CABLE）
    assert best_name_match("__不存在的扬声器__", cands) is None
    assert name_similarity("Speakers", "Speakers (Realtek)") >= 0.6
    assert normalize_device_name("  Mic  (A)  ") == "mic"
    print("  名字模糊匹配: 精确/相似/前缀/无关  OK")


def test_windows_device_matching():
    """Windows PortAudio 设备匹配：精确名命中；未知名按既定兼容策略回退
    第一个可用设备（配置存留旧名时的行为，见 get_device_id 文档）。"""
    if not _IS_WIN:
        print("  跳过（非 Windows）")
        return
    from audio_processor import get_device_names, get_device_id, \
        default_api_type
    api = default_api_type()
    ins, outs = get_device_names(api_type=api)
    assert isinstance(ins, list) and isinstance(outs, list)
    if outs:
        dev = get_device_id(outs[0], False, api_type=api)
        assert dev is not None, f"精确名匹配失败: {outs[0]!r}"
        # 兼容回退：未知名 → 第一个可用输出设备（不抛异常）
        fallback = get_device_id("__不存在设备xyz__", False, api_type=api)
        assert fallback is not None
    print(f"  Windows 设备匹配: {len(ins)} 输入 / {len(outs)} 输出  OK")


def test_virtual_mic_lifecycle():
    """虚拟麦克风建立→使用→卸载（真实路径，需 PipeWire + 显式开启）。

    PUREVOX_TEST_VIRTUAL_MIC=1 时：真实创建 purevox_out（单声道 null-sink）
    与 purevox_mic（remap 真源），验证两者进入标准枚举（"使用"面），随后
    卸载并验证枚举消失。CI 容器无 PipeWire → 自动走优雅降级档。
    """
    from pvplatform import system as sysmod
    ready = sysmod.virtual_mic_ready()
    assert isinstance(ready, bool)
    log = _Log()

    want_real = os.environ.get("PUREVOX_TEST_VIRTUAL_MIC") == "1"
    if _IS_LINUX and want_real:
        from pvplatform.audio.pwpipe_client import (
            list_sources, list_destinations, _list_nodes)
        assert sysmod.ensure_virtual_mic(log) is True
        assert sysmod.virtual_mic_ready() is True
        # "使用"面：虚拟 sink 进入输出枚举；真源 purevox_mic 实际创建，
        # 但按回授防护不得进入 PureVox 自身输入枚举（只对其它软件可见）。
        assert "purevox_out" in list_destinations(), \
            "虚拟 sink 未出现在输出枚举"
        node_names = {n["name"] for n in _list_nodes()}
        assert "purevox_mic" in node_names, "虚拟真源节点未创建"
        assert "purevox_mic" not in list_sources(), \
            "虚拟源不得进入 PureVox 输入枚举（回授防护）"
        time.sleep(0.3)
        sysmod.remove_virtual_mic(log)
        time.sleep(0.5)
        assert sysmod.virtual_mic_ready() is False
        assert "purevox_out" not in list_destinations()
        assert "purevox_mic" not in {n["name"] for n in _list_nodes()}
        print("  虚拟麦克风 真实建立→进入枚举→卸载→枚举消失  OK")
    elif _IS_LINUX:
        from pvplatform.audio.pwpipe_client import pw_available
        assert ready is False or pw_available()
        print(f"  虚拟麦克风优雅降级: ready={ready} "
              f"(真实建立/卸载需 PipeWire + PUREVOX_TEST_VIRTUAL_MIC=1)")
    else:
        assert ready is False       # Windows/macOS 恒 False（Linux 专属）
        print("  虚拟麦克风非 Linux 平台恒 False  OK")


if __name__ == "__main__":
    print("设备面检测:")
    test_enum_graceful()
    test_defaults_coherent()
    test_config_keys_complete()
    test_name_fuzzy_match()
    test_speaker_capture_lifecycle()
    test_devices_present()
    test_windows_device_matching()
    test_virtual_mic_lifecycle()
    print("全部通过")
