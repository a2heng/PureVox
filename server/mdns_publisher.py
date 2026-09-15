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
import logging
import os
import re
import socket
import subprocess
from typing import Optional, List

from zeroconf import Zeroconf, ServiceInfo

logger = logging.getLogger(__name__)


def get_all_ipv4s() -> List[str]:
    """获取所有非回环 IPv4 地址（Windows 优先）"""
    ips: List[str] = []
    if os.name == "nt":
        try:
            output = subprocess.check_output(["ipconfig"], text=True, creationflags=subprocess.CREATE_NO_WINDOW)
            for ip in re.findall(r'IPv4 Address[^:]*:\s*([0-9.]+)', output):
                if not ip.startswith("127.") and ip not in ips:
                    ips.append(ip)
        except Exception:
            pass
    else:
        try:
            hostname = socket.gethostname()
            for info in socket.getaddrinfo(hostname, None, socket.AF_INET):
                ip = info[4][0]
                if not ip.startswith("127.") and ip not in ips:
                    ips.append(ip)
        except Exception:
            pass
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
        if ip not in ips:
            ips.append(ip)
    except OSError:
        pass
    finally:
        s.close()
    if not ips:
        ips.append("127.0.0.1")
    return ips


class MdnsPublisher:
    def __init__(self, port: int, server_name: str = "PureVox"):
        self._zeroconf: Optional[Zeroconf] = None
        self._info: Optional[ServiceInfo] = None
        self._port = port
        self._server_name = server_name
        self._all_ips: List[str] = []
        self._addr: Optional[str] = None   # 指定广播网卡 IP；None = 全部网卡

    async def start(self):
        if self._zeroconf is not None:
            return
        ips = [self._addr] if self._addr else get_all_ipv4s()
        self._all_ips = ips
        addresses = [socket.inet_aton(ip) for ip in ips]
        self._info = ServiceInfo(
            "_purevox._tcp.local.",
            f"{self._server_name} Remote Mic._purevox._tcp.local.",
            addresses=addresses,
            port=self._port,
            properties={"version": "1.0", "name": self._server_name},
        )
        try:
            self._zeroconf = Zeroconf(interfaces=[self._addr]) if self._addr else Zeroconf()
            await self._zeroconf.async_wait_for_start()
            await self._zeroconf.async_register_service(self._info)
            logger.info(f"mDNS 已广播: {', '.join(self._all_ips)}:{self._port}")
        except Exception as e:
            logger.error(f"mDNS 广播失败: {e}")
            if self._zeroconf:
                await self._zeroconf.async_close()
            self._zeroconf = None
            self._info = None

    async def restart(self, addr: Optional[str] = None) -> None:
        """换网后重注册：先停再起；addr 指定时只广播该网卡 IP，None = 全部网卡。"""
        self._addr = addr or None
        await self.stop()
        await self.start()

    async def stop(self):
        if not self._zeroconf:
            return
        try:
            if hasattr(self._zeroconf, 'async_unregister_service'):
                await self._zeroconf.async_unregister_service(self._info)
            elif self._info:
                self._zeroconf.unregister_service(self._info)
        except Exception as e:
            logger.warning(f"mDNS 注销异常: {e}")
        try:
            close_fn = getattr(self._zeroconf, 'async_close', None) or self._zeroconf.close
            if asyncio.iscoroutinefunction(close_fn):
                await asyncio.wait_for(close_fn(), timeout=3.0)
            else:
                close_fn()
        except asyncio.TimeoutError:
            logger.warning("mDNS 停止超时，强制关闭")
            self._zeroconf.close()
        except Exception as e:
            logger.warning(f"mDNS 停止异常: {e}")
        self._zeroconf = None
        self._info = None

    @property
    def local_ip(self) -> str:
        return self._all_ips[0] if self._all_ips else "127.0.0.1"
