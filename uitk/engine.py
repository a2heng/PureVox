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

"""uitk 引擎控制器：链文档 → 会话计划 → 音频流启停（对照 ui_pyside6 主路径精简）。

UI 只调 start(chain_cfg)/stop()，返回错误文案；不碰 Qt。
"""

import sys
from typing import Optional


def enum_io_devices():
    """枚举输入/输出设备 → {"inputs": [(显示,值)], "outputs": [(显示,值)],
    "speakers": 物理扬声器候选, "defaults": 各方向默认值}。

    值一律为真实设备名：Linux = PipeWire node.name（PwBridge 直接用它建流），
    Windows = PortAudio 设备名（显示=值）。输入/输出均为完整列表：
    Linux 输出含 purevox_out（默认即它）、Windows 输出含 CABLE Input，
    不再有独立虚拟设备节点。监听/桌面输入（speakers）列物理扬声器
    （Linux 排除 purevox_out）。
    """
    if sys.platform.startswith("linux"):
        from pvplatform.audio.pwpipe_client import (
            list_sources, list_destinations, source_label, dest_label,
            default_mic_name, default_sink_name, speaker_sink_name)
        from pvplatform.system._posix import VIRTUAL_MIC_SINK
        src = [(source_label(p), p) for p in list_sources()]
        dst = [(dest_label(p), p) for p in list_destinations()]
        speakers = [(t, v) for t, v in dst if v != VIRTUAL_MIC_SINK]
        return {"inputs": src, "outputs": dst, "speakers": speakers,
                "defaults": {"inputs": default_mic_name(),
                             "outputs": default_sink_name(),
                             "speakers": speaker_sink_name()}}
    from audio_processor import get_device_names, default_api_type
    inp, out = get_device_names(api_type=default_api_type())
    in_pairs = [(n, n) for n in inp]
    out_pairs = [(n, n) for n in out]
    return {"inputs": in_pairs, "outputs": out_pairs, "speakers": out_pairs,
            "defaults": {"inputs": inp[0] if inp else "",
                         "outputs": out[0] if out else "",
                         "speakers": out[0] if out else ""}}


def _chain_enabled(chain, ptype):
    return any(e.get("type") == ptype and e.get("enabled", True)
               for e in chain)


class EngineController:
    """音频引擎启停封装。start 返回 None=成功，否则错误文案。"""

    def __init__(self, log, config=None):
        self.log = log
        self.config = config
        self.processor = None
        self.thread = None
        self._media = None
        self.running = False
        self._chain_cfg = []    # 最近一次 start 的链文档（行级实时参数路由用）
        self._server = None     # 网络输入模式：PureVoxServer
        self._server_loop = None

    def start(self, chain_cfg) -> Optional[str]:
        if self.running:
            return None
        self.stop()
        self._chain_cfg = [dict(e) for e in (chain_cfg or [])]
        log = self.log
        try:
            from session_plan import SessionPlan
            plan = SessionPlan.from_chain(chain_cfg)
            for w in plan.warnings:
                log.warn(f"[节点] {w}")
            if not plan.ok():
                return "；".join(plan.problems)

            from pvplatform.audio.backends import select_backend
            required = set()
            if len(plan.inputs) > 1:
                required.add("multi_input")
            if len(plan.outputs) > 1:
                required.add("multi_output")
            if plan.loopbacks or \
                    any(r["far_kind"] == "speaker" for r in plan.aec_rows):
                required.add("loopback_far")
            backend = select_backend(frozenset(required))
            if backend is None:
                return "当前平台没有可用的音频传输后端"
            log.msg(f"[后端] {backend.label} ({backend.name})")
            use_pw = backend.name == "pipewire"

            err = self._check_48k(chain_cfg, plan, use_pw) if not use_pw else None
            if err:
                return err

            from audio_processor import create_audio_processor, \
                start_audio_stream, HOP_LENGTH, get_device_id, default_api_type
            proc = create_audio_processor()
            self.processor = proc
            if getattr(proc, "plugin_errors", None):
                for perr in proc.plugin_errors:
                    log.warn(f"[插件] {perr}")
            proc.set_plugins([dict(e) for e in chain_cfg])

            # ── 纯媒体会话（无设备输入、无网络输入）：MediaSession 独立播放，
            # 不启动 AudioThread——播放库（miniaudio PlaybackDevice）拉模型
            # 直出，设备时钟即节拍；跨时钟域由注入的 PlaybackSink 消化；
            # 确定性本地播放与不确定实时流（网络）互不掺杂 ──
            network = plan.remote_url is not None
            if not plan.inputs and not network:
                from pvplatform.audio.media_session import MediaSession
                from pvengine import PlaybackSink
                self._media = MediaSession(
                    lambda n: proc.media_read(n), list(plan.outputs),
                    make_sink=lambda: PlaybackSink(hop=HOP_LENGTH))
                if not self._media.start():
                    err = self._media.error or "媒体输出设备打开失败"
                    self._media = None
                    return err
                self.running = True
                log.msg("[启动] 纯媒体会话（miniaudio 播放设备直出）")
                return None

            if plan.remote_url is not None and plan.aec_rows:
                return "网络输入模式不支持回声消除行（AEC 需要本地麦克风）"
            if not use_pw and plan.aec_rows:
                # Windows 单输入后端：AEC 行的 mic 须是主输入（同一设备）
                main_in = plan.inputs[0] if plan.inputs else ""
                bad = [r["mic"] for r in plan.aec_rows if r["mic"] != main_in]
                if bad:
                    return "Windows 单输入后端：回声消除的麦克风须与音频输入为同一设备，" \
                        f"请改选 {main_in or '主输入设备'}（{ '、'.join(bad)} 不符）"

            # 网络输入：启动 WSS 服务器，把 RemoteAudioSource 作为输入源；
            # 本地输入为空，输出走同一套 sinks（Linux PwBridge / Windows PaBridge）
            network_source = None
            if network:
                server = self._ensure_network_server()
                if server is None:
                    return "网络服务器启动失败"
                network_source = server.audio_source

            pw_ports = ([], [])
            inp = out = None
            extra_out = []
            if use_pw:
                pw_ports = (list(plan.inputs), list(plan.outputs))
                log.msg(f"[启动] PipeWire 输入x{len(pw_ports[0])} "
                        f"输出x{len(pw_ports[1])}")
            else:
                api_type = default_api_type()
                inp = get_device_id(plan.inputs[0], True, api_type=api_type) \
                    if plan.inputs else None
                out = get_device_id(plan.outputs[0], False, api_type=api_type) \
                    if plan.outputs else None
                # 多路输出扇出：首输出走主流，其余设备经 extra 输出流写入
                extra_out = [get_device_id(n, False, api_type=api_type)
                             for n in plan.outputs[1:]]
                extra_out = [e for e in extra_out if e is not None]
                if extra_out:
                    log.msg(f"[启动] 多路输出 +{len(extra_out)}")

            active = [e.get("type") for e in chain_cfg if e.get("enabled", True)]
            ready_msg = "+".join(active) if active else "空链"
            self.thread = start_audio_stream(
                inp, out, proc, HOP_LENGTH,
                network_source=network_source,
                api_type=default_api_type(), ready_msg=ready_msg,
                extra_output_ids=extra_out, pw_ports=pw_ports,
                aec_rows=list(plan.aec_rows), aec_inputs=list(plan.inputs),
                loopbacks=list(plan.loopbacks))
            if self.thread and not self.thread.wait_ready(timeout=3.0):
                err = getattr(self.thread, "_start_error", None) or "音频流创建超时"
                self.stop()
                return f"音频流创建失败: {err}"

            if plan.aec_rows:
                log.msg(f"[AEC] 行级回声消除 x{len(plan.aec_rows)} "
                        f"({'; '.join(r['mic'] + '<-' + r['far_device'] for r in plan.aec_rows)})")

            self.running = True

            # TSE：挂录音钩子 + 加载已保存参考（无参考时插件直通）
            try:
                from audio_processor import register_tse_audio_hook, \
                    load_tse_reference, CFG_REF_WAV_PATH
                register_tse_audio_hook(self.thread, log.msg)
                if _chain_enabled(chain_cfg, "tse"):
                    wav = (self.config.get(CFG_REF_WAV_PATH, "")
                           if self.config else "")
                    if wav:
                        load_tse_reference(proc, wav)
            except Exception as e:
                log.warn(f"[TSE] 初始化失败: {e}")
            return None
        except Exception as e:
            import traceback
            log.err(f"启动失败: {e}\n{traceback.format_exc()}")
            self.stop()
            return str(e)

    def _check_48k(self, chain_cfg, plan, use_pw) -> Optional[str]:
        """Windows PortAudio：逐设备 48k 打开检测（WASAPI 严格/MME 宽松为既定行为）。"""
        if use_pw:
            return None
        try:
            import pyaudio
        except ImportError:
            return None
        from audio_processor import get_device_id, default_api_type, HOP_LENGTH
        api_type = default_api_type()
        failed = []
        p = pyaudio.PyAudio()
        try:
            checks = [(True, n) for n in plan.inputs] + \
                     [(False, n) for n in plan.outputs]
            # AEC far 选麦克风时是独立采集流，同样逐设备 48k 门禁；
            # far 选扬声器走 loopback，按既定行为免检。
            checks += [(True, n) for n in plan.aec_far_mics
                       if n not in plan.inputs]
            for is_in, name in checks:
                dev = get_device_id(name, is_in, api_type=api_type)
                if dev is None:
                    continue
                try:
                    if is_in:
                        s = p.open(format=pyaudio.paFloat32, channels=1,
                                   rate=48000, input=True,
                                   input_device_index=dev, frames_per_buffer=HOP_LENGTH)
                    else:
                        s = p.open(format=pyaudio.paFloat32, channels=1,
                                   rate=48000, output=True,
                                   output_device_index=dev, frames_per_buffer=HOP_LENGTH)
                    s.close()
                except Exception as e:
                    failed.append(f"{name or '系统默认'} ({e})")
        finally:
            p.terminate()
        if failed:
            return "以下设备不支持 48kHz，已阻止启动: " + "、".join(failed)
        return None

    def set_live_param(self, index, key, value):
        """滑杆实时生效：直接更新运行中处理器的插件参数（不重建链）。

        AEC 行的参考音量直达运行中的行（AudioThread.set_aec_far_gain），
        处理器内 AEC 行只是占位，推给它没有效果。
        """
        if not (self.processor and self.running):
            return
        try:
            entry = self._chain_cfg[index] if 0 <= index < len(self._chain_cfg) \
                else {}
            if entry.get("type") == "echo_cancel" and key == "far_gain_db" \
                    and self.thread is not None:
                mic = str((entry.get("params") or {}).get("device", ""))
                if self.thread.set_aec_far_gain(mic, float(value)):
                    return
            self.processor.update_plugin_param(index, key, value)
        except Exception as e:
            self.log.warn(f"[参数] 实时更新失败 ({key}): {e}")

    def set_plugin_enabled(self, index: int, enabled: bool):
        """fx 行勾选热更：只翻 Stage/插件 enabled（不重启音频流）。

        DESIGN §7 热更矩阵——复选框开/关由运行中处理器原地生效；
        媒体源类插件（FADE_THROUGH）自行淡出/淡入，衔接无缝。
        TSE 例外：运行中启用时即时加载已存参考（参考只在 start 载入，
        不补这一步就是"勾选了也没效果，只能重启"，见 _hot_toggle 修复）。
        """
        if self.processor and self.running:
            try:
                self.processor.set_plugin_enabled(index, enabled)
                if enabled:
                    entry = self._chain_cfg[index] \
                        if 0 <= index < len(self._chain_cfg) else {}
                    if entry.get("type") == "tse" and \
                            not self.processor.is_tse_reference_loaded():
                        from audio_processor import load_tse_reference, \
                            CFG_REF_WAV_PATH
                        wav = (self.config.get(CFG_REF_WAV_PATH, "")
                               if self.config else "")
                        if wav and load_tse_reference(self.processor, wav):
                            self.log.msg("[TSE] 运行中启用，已载入参考音频")
            except Exception as e:
                self.log.warn(f"[节点] 启停热更失败: {e}")

    def soundpad_play(self, index: int):
        if self.processor and self.running:
            try:
                self.processor.soundpad_play(index)
            except Exception as e:
                self.log.warn(f"[音效板] 播放失败: {e}")

    def soundpad_stop(self, index: int):
        if self.processor and self.running:
            try:
                self.processor.soundpad_stop(index)
            except Exception:
                pass

    def soundpad_stop_all(self):
        if self.processor and self.running:
            try:
                self.processor.soundpad_stop_all()
            except Exception:
                pass

    def music_status(self, index: int) -> dict:
        """音乐播放器状态（未运行返回默认零值）。"""
        if self.processor and self.running:
            try:
                return self.processor.music_status(index)
            except Exception:
                pass
        return {"playing": False, "pos": 0.0, "dur": 0.0}

    def calibrate_aec_delay(self, mic_dev: str, far_dev: str,
                            far_kind: str = "speaker") -> Optional[float]:
        """离线校准 AEC far 延迟（须在音频停止时安静测一次，硬件不变则恒定）。

        返回「扬声器端点 → 麦克风回声」唯一延迟路径值（无隐含叠加），
        即滑杆要用的值；失败返回 None（UI 保留原值）。
        """
        from audio_processor import AudioThread
        return AudioThread.calibrate_aec_delay(mic_dev, far_dev, far_kind)

    def stop(self):
        if self._media is not None:
            try:
                self._media.stop()
            except Exception:
                pass
            self._media = None
        if self.thread:
            try:
                self.thread.stop()
            except Exception:
                pass
            self.thread = None
        if self.processor:
            try:
                self.processor.cleanup()
            except Exception:
                pass
            self.processor = None
        self._stop_network_server()
        self.running = False

    # ── 网络输入：PureVox WSS 服务器生命周期 ───────────────────────

    def _ensure_network_server(self):
        """启动 WSS 服务器（幂等）；返回实例或 None。平台无关。"""
        if self._server is not None:
            return self._server
        import asyncio
        import threading
        try:
            from server.https_server import PureVoxServer
        except Exception as e:
            self.log.err(f"[服务器] 导入失败: {e}")
            return None
        port = 59123
        if self.config:
            try:
                port = int(self.config.get("server_port", 59123) or 59123)
            except Exception:
                port = 59123
        try:
            from pvplatform.system import add_firewall_rule
            add_firewall_rule(self.log)
        except Exception:
            pass
        server = PureVoxServer(port=port)
        server.set_logger(self.log.msg)
        loop = asyncio.new_event_loop()
        self._server = server
        self._server_loop = loop
        threading.Thread(target=self._run_server_loop, args=(server, loop),
                         daemon=True).start()
        self.log.msg(f"[服务器] 已启动 (端口 {port})")
        return server

    def _run_server_loop(self, server, loop):
        """后台线程：运行 PureVoxServer 的 asyncio 事件循环。"""
        import asyncio
        asyncio.set_event_loop(loop)
        try:
            loop.run_until_complete(server.start())
            loop.run_forever()
        except Exception as e:
            self.log.err(f"[服务器] 事件循环异常: {e}")
        finally:
            try:
                for task in asyncio.all_tasks(loop):
                    task.cancel()
            except Exception:
                pass
            try:
                if not loop.is_closed():
                    loop.close()
            except Exception:
                pass

    def _stop_network_server(self):
        """停止网络服务器（幂等）。"""
        server, loop = self._server, self._server_loop
        self._server = None
        self._server_loop = None
        if server is None or loop is None:
            return
        import asyncio
        try:
            async def _stop():
                await server.stop()
            fut = asyncio.run_coroutine_threadsafe(_stop(), loop)
            fut.result(timeout=5)
            loop.call_soon_threadsafe(loop.stop)
        except Exception as e:
            self.log.warn(f"[服务器] 停止异常: {e}")
