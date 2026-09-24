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

"""SessionPlan 纯函数测试（L3 会话计划契约，无音频副作用）：
python tests/test_session_plan.py
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from session_plan import SessionPlan


def test_valid_chain():
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": "Mic"}},
        {"type": "denoiser_m", "enabled": True, "params": {}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
        {"type": "vu_meter", "enabled": True, "params": {}},
    ])
    assert plan.ok(), plan.problems
    assert plan.inputs == ("Mic",)
    assert plan.outputs == ("Spk",)
    assert plan.remote_url is None
    assert plan.viz == frozenset({"vu_meter"})
    assert plan.fx_chain == ({"type": "denoiser_m", "enabled": True, "params": {}},)
    print("  合法链 → ok、字段抽取正确  OK")


def test_no_input_blocked():
    plan = SessionPlan.from_chain([
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert not plan.ok()
    assert any("音频输入" in p for p in plan.problems)
    print("  无输入节点 → 阻断  OK")


def test_no_output_blocked():
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": "Mic"}},
    ])
    assert not plan.ok()
    assert any("音频输出" in p for p in plan.problems)
    print("  无输出节点 → 阻断  OK")


def test_empty_device_skipped_with_warning():
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": ""}},
        {"type": "denoiser_m", "enabled": True, "params": {}},
        {"type": "audio_output", "enabled": True, "params": {"device": ""}},
    ])
    assert not plan.ok()
    assert any("未选设备" in w for w in plan.warnings)
    assert plan.inputs == () and plan.outputs == ()
    print("  空 device 行跳过并告警  OK")


def test_disabled_nodes_skipped():
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": False, "params": {"device": "Mic"}},
        {"type": "denoiser_m", "enabled": False, "params": {}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert not plan.ok()          # 输入行被禁用 = 无输入 → 阻断
    assert plan.fx_chain == ()
    print("  禁用节点不参与计划  OK")


def test_unknown_type_warning():
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": "Mic"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
        {"type": "totally_new_node", "enabled": True, "params": {}},
    ])
    assert plan.ok(), plan.problems
    assert any("未知节点类型" in w for w in plan.warnings)
    print("  未知类型忽略并告警（向前兼容）  OK")


def test_remote_mic():
    plan = SessionPlan.from_chain([
        {"type": "remote_mic", "enabled": True, "params": {"url": "wss://x"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert plan.ok()
    assert plan.remote_url == "wss://x"
    assert plan.inputs == ()      # 远程源不进本地设备列表
    # url 为空 → 阻断
    plan2 = SessionPlan.from_chain([
        {"type": "remote_mic", "enabled": True, "params": {"url": "  "}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert not plan2.ok()
    assert any("地址为空" in p for p in plan2.problems)
    print("  remote_mic url 抽取/空地址阻断  OK")


def test_pure_media_session():
    plan = SessionPlan.from_chain([
        {"type": "desktop_audio", "enabled": True, "params": {}},
        {"type": "vu_meter", "enabled": True, "params": {}},
    ])
    assert plan.ok(), plan.problems
    assert plan.inputs == () and plan.outputs == ()
    assert any("仅媒体源发声" in w for w in plan.warnings)
    print("  纯媒体会话（媒体节点即输入源）合法  OK")


def test_multiple_outputs_ordered():
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": "Mic"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "A"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "B"}},
    ])
    assert plan.ok()
    assert plan.outputs == ("A", "B")
    print("  多输出按链序  OK")


def test_aec_row_is_input():
    """AEC 行即输入：只有 AEC 行 + 输出也是合法会话，不报无输入；
    mic 进 inputs，far=扬声器进 aec_rows。"""
    plan = SessionPlan.from_chain([
        {"type": "echo_cancel", "enabled": True,
         "params": {"device": "Mic",
                    "far_kind": "speaker", "far_device": "Spk"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert plan.ok(), plan.problems
    assert plan.inputs == ("Mic",)
    assert plan.aec_rows == ({"mic": "Mic", "far_gain_db": -20.0,
                              "far_kind": "speaker", "far_device": "Spk",
                              "far_delay_ms": 0.0},)
    assert plan.aec_far_mics == ()
    print("  AEC 行计入输入、行配置抽取正确  OK")


def test_aec_owns_mic_plain_duplicate_skipped():
    """AEC 行接管 mic：普通音频输入同设备行跳过并告警（与链序无关）。"""
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": "Mic"}},
        {"type": "echo_cancel", "enabled": True,
         "params": {"device": "Mic",
                    "far_kind": "speaker", "far_device": "Spk"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert plan.ok(), plan.problems
    assert plan.inputs == ("Mic",)
    assert any("接管" in w for w in plan.warnings)
    print("  AEC 接管 mic、同设备普通输入跳过告警  OK")


def test_aec_far_mic_routed():
    """AEC far=麦克风：far 设备进 aec_far_mics（专用采集，不进混音）。"""
    plan = SessionPlan.from_chain([
        {"type": "echo_cancel", "enabled": True,
         "params": {"device": "Mic",
                    "far_kind": "mic", "far_device": "Mic2"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert plan.ok(), plan.problems
    assert plan.inputs == ("Mic",)
    assert plan.aec_far_mics == ("Mic2",)
    assert plan.aec_rows[0]["far_kind"] == "mic"
    print("  AEC far 麦克风路由到专用采集  OK")


def test_aec_row_empty_far_skipped():
    """AEC 行未选 far：该行跳过（不阻断，普通输入仍可建流）。"""
    plan = SessionPlan.from_chain([
        {"type": "audio_input", "enabled": True, "params": {"device": "Mic"}},
        {"type": "echo_cancel", "enabled": True,
         "params": {"device": "Mic2",
                    "far_kind": "speaker", "far_device": ""}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert plan.ok(), plan.problems
    assert plan.aec_rows == ()
    assert plan.inputs == ("Mic",)   # 缺 far 整行跳过，Mic2 不进输入
    assert any("far" in w for w in plan.warnings)
    print("  AEC 行缺 far 跳过告警  OK")


def test_loopback_row_is_input():
    """回环输入行即输入：只有回环行 + 输出也是合法会话。"""
    plan = SessionPlan.from_chain([
        {"type": "loopback", "enabled": True, "params": {"device": "Spk"}},
        {"type": "audio_output", "enabled": True, "params": {"device": "Spk"}},
    ])
    assert plan.ok(), plan.problems
    assert plan.loopbacks == ("Spk",)
    assert plan.inputs == ()
    print("  回环输入行计入输入  OK")


if __name__ == "__main__":
    print("SessionPlan 测试:")
    test_valid_chain()
    test_no_input_blocked()
    test_no_output_blocked()
    test_empty_device_skipped_with_warning()
    test_disabled_nodes_skipped()
    test_unknown_type_warning()
    test_remote_mic()
    test_pure_media_session()
    test_multiple_outputs_ordered()
    test_aec_row_is_input()
    test_aec_owns_mic_plain_duplicate_skipped()
    test_aec_far_mic_routed()
    test_aec_row_empty_far_skipped()
    test_loopback_row_is_input()
    print("全部通过")
