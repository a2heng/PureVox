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

"""L3 会话层：链文档 → 可执行会话计划（DESIGN.md §4）。

纯函数、无 Qt、无音频副作用——只依赖节点注册表。
UI 启动流程调用 from_chain 得到计划；ok() 为假展示 problems 并中止，
为真则把字段分发给传输层（AudioThread/PwBridge）与引擎（set_plugins）。
"""

from dataclasses import dataclass
from typing import List, Optional, Tuple

from pvengine.plugins import get_spec, MEDIA_NODE_TYPES


@dataclass(frozen=True)
class SessionPlan:
    """一次可启动音频会话的完整描述。"""

    inputs: Tuple[str, ...]              # 启用的采集设备（有序，含 AEC 行的 mic）
    outputs: Tuple[str, ...]            # 启用的播放设备（有序；首为主输出）
    remote_url: Optional[str]            # 远程推流地址；None = 无网络输入
    viz: frozenset                       # 启用的可视化节点名子集
    fx_chain: Tuple[dict, ...]           # 启用的 fx 条目（引擎就绪格式）
    aec_rows: Tuple[dict, ...] = ()      # 启用的 AEC 行（有序）：
                                         # {mic, far_gain_db, far_kind, far_device}；
                                         # mic 与 audio_input 走同一设备机制，
                                         # 同列表、同去重（输入端自适应，不门禁）
    aec_far_mics: Tuple[str, ...] = ()   # AEC far 选麦克风时的专用采集设备
                                         # （独立采集直达 AEC，不进混音）
    loopbacks: Tuple[str, ...] = ()      # 启用的回环输入（扬声器设备，有序）：
                                         # 独立回采进混音；AEC 行 far=扬声器
                                         # 与它继承同一套回环采集机制
    problems: Tuple[str, ...] = ()       # 阻断性问题（非空则不得建流）
    warnings: Tuple[str, ...] = ()       # 非阻断提示

    def ok(self) -> bool:
        return not self.problems

    @classmethod
    def from_chain(cls, chain_cfg) -> "SessionPlan":
        """校验并抽取链文档。未知 type 忽略并记 warning（向前兼容）。"""
        inputs: List[str] = []
        outputs: List[str] = []
        remote_url: Optional[str] = None
        viz: set = set()
        fx: List[dict] = []
        problems: List[str] = []
        warnings: List[str] = []

        # AEC 行的 mic 与 audio_input 继承同一设备机制：同解析、同列表；
        # 同一设备被 AEC 行接管后，普通音频输入行的重复项跳过并告警
        # （一行设备只进一次混音，可控无歧义）。预扫描先定接管集合，
        # 与链序无关（audio_input 在前也不会漏告警）。
        aec_mics: List[str] = []
        for _item in (chain_cfg or []):
            if str(_item.get("type", "")) == "echo_cancel" \
                    and bool(_item.get("enabled", True)):
                _p = _item.get("params") or {}
                _dev = str(_p.get("device", "") or "").strip()
                _far = str(_p.get("far_device", "") or "").strip()
                # mic 与 far 双全才算接管（缺 far 的行主循环会跳过，
                # 不能吞掉同设备普通输入行）
                if _dev and _far and _dev not in aec_mics:
                    aec_mics.append(_dev)
        aec_rows: List[dict] = []
        aec_far_mics: List[str] = []
        loopbacks: List[str] = []

        for item in (chain_cfg or []):
            t = str(item.get("type", ""))
            enabled = bool(item.get("enabled", True))
            params = item.get("params") or {}
            spec = get_spec(t)
            if spec is None:
                warnings.append(f"未知节点类型「{t}」已忽略")
                continue
            if not enabled:
                continue
            if spec.kind == "input":
                if t == "remote_mic":
                    url = str(params.get("url", "") or "").strip()
                    if remote_url is None:
                        remote_url = url
                    else:
                        warnings.append("多个远程推流节点，仅取第一个")
                elif t == "echo_cancel":
                    dev = str(params.get("device", "") or "").strip()
                    if not dev:
                        warnings.append("「AEC 输入」未选麦克风设备，该行已跳过")
                        continue
                    far_gain = -20.0
                    far_kind = str(params.get("far_kind", "") or "").strip()
                    far_dev = str(params.get("far_device", "") or "").strip()
                    if far_kind not in ("speaker", "mic"):
                        # UI 恒写显式 far_kind；缺省按扬声器处理。
                        far_kind = "speaker"
                    if not far_dev:
                        warnings.append("「AEC 输入」未选 far 参考设备，该行已跳过")
                        continue
                    if dev not in inputs:
                        inputs.append(dev)
                    far_delay = float(params.get("far_delay_ms", 0.0))
                    aec_rows.append({"mic": dev, "far_gain_db": far_gain,
                                     "far_kind": far_kind, "far_device": far_dev,
                                     "far_delay_ms": far_delay})
                    if far_kind == "mic" and far_dev not in aec_far_mics:
                        aec_far_mics.append(far_dev)
                elif t == "loopback":
                    dev = str(params.get("device", "") or "").strip()
                    if dev:
                        if dev not in loopbacks:
                            loopbacks.append(dev)
                    else:
                        warnings.append("「播放输入」未选扬声器设备，该行已跳过")
                else:
                    dev = str(params.get("device", "") or "").strip()
                    if dev:
                        if dev in aec_mics:
                            warnings.append(
                                f"「录音输入」{dev} 已被 AEC 输入行接管，该行已跳过")
                        elif dev not in inputs:
                            inputs.append(dev)
                    else:
                        warnings.append("「音频输入」未选设备，该行已跳过")
            elif spec.kind == "output":
                dev = str(params.get("device", "") or "").strip()
                if dev:
                    outputs.append(dev)
                else:
                    warnings.append("「音频输出」未选设备，该行已跳过")
            elif spec.kind == "viz":
                viz.add(t)
            elif spec.kind == "fx":
                fx.append({"type": t, "enabled": True,
                           "params": dict(params)})

        if remote_url is not None and not remote_url:
            problems.append("远程推流节点已启用，但地址为空")
        # 媒体源（音效板/音乐播放器/桌面声音）本身即可作为输入：
        # 无麦克风但有启用中的媒体节点 = 合法的纯媒体会话。
        # AEC 行的 mic 直接进 inputs：只有 AEC 行（无普通输入/无媒体/
        # 无推流）也是合法会话，不再报"未启用任何输入"。回环输入同理。
        has_media = any(e["type"] in MEDIA_NODE_TYPES for e in fx)
        has_input = bool(inputs or loopbacks)
        if remote_url is None and not has_input and not has_media:
            problems.append("未启用任何「音频输入」节点"
                            "（回声消除/桌面输入/媒体输入节点亦可）")
        elif remote_url is None and not has_input and has_media:
            warnings.append("未选麦克风输入：本次仅媒体源发声")
        if not outputs and not has_media:
            problems.append("未启用任何「音频输出」节点")

        return cls(inputs=tuple(inputs), outputs=tuple(outputs),
                    remote_url=remote_url, viz=frozenset(viz),
                    fx_chain=tuple(fx), aec_rows=tuple(aec_rows),
                    aec_far_mics=tuple(aec_far_mics),
                    loopbacks=tuple(loopbacks),
                    problems=tuple(problems), warnings=tuple(warnings))
