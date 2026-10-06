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

import asyncio
import base64
import json
import os
import sys
from typing import Optional, Set

import aiohttp
from aiohttp import web

from server.opus_codec import OpusDecoder
from server.tls_manager import TlsManager



class PureVoxServer:
    def __init__(self, port: int = 8443, html_dir: str = "", audio_source=None):
        self._port = port
        if html_dir:
            self._html_dir = html_dir
        else:
            # 尝试多个可能的 html 目录位置
            script_dir = os.path.dirname(os.path.abspath(__file__))
            candidates = [
                os.path.join(script_dir, "..", "html"),           # 源码结构: server/../html/
                os.path.join(script_dir, "html"),                 # 某些打包结构: server/html/
            ]
            # PyInstaller 打包时 html 可能在根目录
            if getattr(sys, 'frozen', False):
                base = os.path.dirname(sys.executable)
                candidates.insert(0, os.path.join(base, "html"))
                candidates.append(base)  # 也可能直接在 exe 目录下
            self._html_dir = ""
            for p in candidates:
                resolved = os.path.abspath(p)
                if os.path.isdir(resolved) and os.path.isfile(os.path.join(resolved, "index.html")):
                    self._html_dir = resolved
                    break
            if not self._html_dir:
                self._html_dir = os.path.abspath(candidates[0])  # 兜底用第一个
        self._tls = TlsManager()
        if audio_source is not None:
            self._audio_source = audio_source
        else:
            # 延迟导入：缺省数据桥依赖主线 audio_processor；精简调用方注入自带实现
            from server.audio_bridge import RemoteAudioSource
            self._audio_source = RemoteAudioSource()
        self._opus_decoder = OpusDecoder()
        self._app: Optional[web.Application] = None
        self._runner: Optional[web.AppRunner] = None
        self._mdns = None
        self._active_ws: Set[web.WebSocketResponse] = set()
        self._log = print
        self._flush_last_seq: int = -1  # flush 时记录的 last_seq，用于丢弃路上旧包
        self._loop = None               # 服务器事件循环（apply_network/restart 投递用）
        self._ssl_ctx = None            # 运行中的 SSLContext（证书热加载用）

    def set_logger(self, log_func):
        self._log = log_func
        self._audio_source.set_logger(log_func)

    @property
    def audio_source(self):
        """注入的音频数据桥（主线为 RemoteAudioSource；精简调用方可自带）。"""
        return self._audio_source

    @property
    def port(self) -> int:
        return self._port

    def _setup_routes(self, app: web.Application):
        @web.middleware
        async def cors_middleware(request, handler):
            if request.method == "OPTIONS":
                resp = web.Response()
            else:
                resp = await handler(request)
            resp.headers["Access-Control-Allow-Origin"] = "*"
            resp.headers["Access-Control-Allow-Methods"] = "GET, POST, OPTIONS"
            resp.headers["Access-Control-Allow-Headers"] = "*"
            return resp
        app.middlewares.append(cors_middleware)

        app.router.add_get("/", self._handle_index)
        app.router.add_get("/api/status", self._handle_status)
        app.router.add_get("/api/ca-cert", self._handle_ca_cert)
        app.router.add_get("/ws/audio", self._handle_ws_audio)
        for subdir in ["css", "js", "wasm"]:
            full_path = os.path.join(self._html_dir, subdir)
            if os.path.isdir(full_path):
                app.router.add_static(f"/{subdir}/", full_path)

    async def _handle_index(self, request: web.Request) -> web.Response:
        index_path = os.path.join(self._html_dir, "index.html")
        if os.path.isfile(index_path):
            return web.FileResponse(index_path)
        self._log(f"[服务器] 错误: 找不到 {index_path}")
        raise web.HTTPNotFound(text=f"index.html not found (html_dir={self._html_dir})")

    async def _handle_status(self, request: web.Request) -> web.Response:
        return web.json_response({
            "name": "PureVox",
            "version": "1.0",
            "active_clients": self._audio_source.active_clients,
            "sample_rate": self._audio_source.sample_rate,
            "opus_available": self._opus_decoder.available,
        })

    async def _handle_ca_cert(self, request: web.Request) -> web.Response:
        pem = self._tls.get_ca_cert_pem()
        return web.Response(
            body=pem,
            content_type="application/x-pem-file",
            headers={"Content-Disposition": "attachment; filename=purevox-ca.crt"},
        )

    async def _handle_ws_audio(self, request: web.Request) -> web.WebSocketResponse:
        ws = web.WebSocketResponse()
        await ws.prepare(request)
        self._active_ws.add(ws)
        self._audio_source.client_connected()
        self._log("[WSS] 音频客户端已连接")

        try:
            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.TEXT:
                    try:
                        data = json.loads(msg.data)
                        msg_type = data.get("type")
                        if msg_type == "audio":
                            seq = data.get("seq", -1)
                            opus_bytes = base64.b64decode(data.get("data", ""))
                            pcm = self._opus_decoder.decode(opus_bytes)
                            if pcm:
                                self._audio_source.write_pcm(pcm)
                            else:
                                self._log(f"[WSS] Opus 解码失败: seq={seq} len={len(opus_bytes)}")

                            ack = {"type": "ack", "seq": seq}
                            await ws.send_str(json.dumps(ack))
                        elif msg_type == "flush":
                            self._audio_source.flush()
                            self._flush_last_seq = -1  # 重置过滤，后续所有包正常处理
                            self._log("[WSS] 客户端请求清空缓冲")
                        elif msg_type == "stop":
                            break
                    except Exception as e:
                        self._log(f"[WSS] 消息处理错误: {e}")
                elif msg.type == aiohttp.WSMsgType.ERROR:
                    self._log(f"[WSS] 错误: {ws.exception()}")
        except asyncio.CancelledError:
            pass
        finally:
            self._active_ws.discard(ws)
            await ws.close()
            self._audio_source.client_disconnected()
            self._log("[WSS] 音频客户端已断开")

        return ws

    async def start(self):
        if getattr(self, '_started', False):
            self._log("[服务器] 已启动，跳过")
            return
        self._started = True
        self._loop = asyncio.get_running_loop()
        self._tls.ensure_ca()
        from server.mdns_publisher import MdnsPublisher, get_all_ipv4s
        self._mdns = MdnsPublisher(self._port)
        all_ips = get_all_ipv4s()
        self._mdns._all_ips = all_ips
        local_ip = all_ips[0] if all_ips else "127.0.0.1"
        want = all_ips + ["127.0.0.1"]
        # 证书 SAN 未覆盖当前网卡（换网/首次）→ 重签
        self._tls.generate_server_cert(
            want, force=not self._tls.server_cert_covers(want))

        self._app = web.Application()
        self._setup_routes(self._app)
        self._runner = web.AppRunner(self._app)
        await self._runner.setup()

        ssl_ctx = self._tls.get_ssl_context()
        self._ssl_ctx = ssl_ctx
        site = web.TCPSite(self._runner, "0.0.0.0", self._port, ssl_context=ssl_ctx)
        try:
            await site.start()
        except OSError as e:
            self._log(f"[服务器] 端口 {self._port} 被占用，尝试清理...")
            await self._runner.cleanup()
            self._started = False
            raise

        await self._mdns.start()
        self._log(f"[服务器] HTTPS 已启动: https://{local_ip}:{self._port}")
        self._log(f"[服务器] mDNS 已广播: _purevox._tcp.local.")

    async def stop(self):
        # 先关闭所有活跃的 WebSocket 连接
        for ws in list(self._active_ws):
            try:
                await asyncio.wait_for(ws.close(), timeout=1.0)
            except Exception:
                pass
        self._active_ws.clear()

        if self._mdns:
            await self._mdns.stop()
            self._mdns = None
        if self._runner:
            try:
                await asyncio.wait_for(self._runner.cleanup(), timeout=3.0)
            except asyncio.TimeoutError:
                self._log("[服务器] 清理超时，强制停止")
            self._runner = None
        self._log("[服务器] 已停止")

    # ── 换网恢复（线程安全：可从任意线程投递到服务器事件循环）──

    def apply_network(self, ip: Optional[str] = None):
        """切网统一路径：证书 SAN 未覆盖当前网卡 IP 时重签并热加载 + mDNS 重注册。

        ip 指定时只在该网卡广播 mDNS，None = 全部网卡。
        """
        loop = self._loop
        if loop is None:
            return

        async def _run():
            try:
                from server.mdns_publisher import get_all_ipv4s
                all_ips = get_all_ipv4s()
                if not self._tls.server_cert_covers(all_ips):
                    self._tls.ensure_ca()
                    self._tls.generate_server_cert(
                        all_ips + ["127.0.0.1"], force=True)
                    if self._ssl_ctx is not None:
                        self._tls.reload_ssl_context(self._ssl_ctx)
                    self._log("[服务器] 证书已按当前网卡重签并热加载")
            except Exception as e:
                self._log(f"[服务器] 证书重签失败: {e}")
            if self._mdns is not None:
                try:
                    await self._mdns.restart(ip)
                    self._log(f"[服务器] mDNS 已重注册 ({ip or '全部网卡'})")
                except Exception as e:
                    self._log(f"[服务器] mDNS 重注册失败: {e}")

        try:
            asyncio.run_coroutine_threadsafe(_run(), loop)
        except Exception as e:
            self._log(f"[服务器] apply_network 投递失败: {e}")

    def restart(self):
        """重开监听（换网/异常恢复）：停后原端口重启，线程安全。"""
        loop = self._loop
        if loop is None:
            return

        async def _run():
            await self.stop()
            self._started = False
            await self.start()

        try:
            asyncio.run_coroutine_threadsafe(_run(), loop)
        except Exception as e:
            self._log(f"[服务器] 重启投递失败: {e}")
