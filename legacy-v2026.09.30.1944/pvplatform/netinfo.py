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

"""本机网卡 IPv4 枚举（带网卡名、物理口优先、TUN/VPN 沉底）。

主线网络输入节点与 Lite Net 页共用同一份实现（唯一实现路径）：UI 网络下拉
与自动选择（best_lan_ip）都从这里取。网络服务/TLS/mDNS/协议不在此处。
"""

import ctypes
import ctypes.wintypes as wt
import socket
import struct

# TUN/VPN 虚拟口不硬剔除（用户可显式选择），仅在自动选择时规避
_TUN_IF_TYPES = {53, 131}  # PPP, TunnelEncapsulation
_TUN_NAME_HINTS = (
    "tun", "tap", "wintun", "wireguard", "vpn", "clash", "mihomo",
    "v2ray", "sing-box", "singbox", "zerotier", "tailscale", "openvpn",
    "hamachi", "ppp", "nordlynx", "proton", "tailscale", "loon", "shadowsocks",
)


def sys_platform_win():
    import sys
    return sys.platform.startswith("win")


def _is_tun_name(name):
    n = (name or "").lower().replace("-", "").replace("_", "").replace(" ", "")
    return any(h in n for h in _TUN_NAME_HINTS)


def list_lan_ips():
    """返回 [(ipv4, if_name), ...]，全部 Up 网卡（剔除回环/链路本地），
    物理口在前、隧道/虚拟口在后"""
    out = []  # (ip, name, iftype)
    if sys_platform_win():
        for ip, name, iftype, oper in _win_adapters():
            if oper != 1:  # IfOperStatusUp：未连接的网卡没有可用端点
                continue
            out.append((ip, name, iftype))
    else:
        try:
            import array
            import fcntl
            # POSIX: SIOCGIFCONF 枚举接口名+地址
            buf = array.array("B", b"\0" * 8192)
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            addr, ln = buf.buffer_info()
            ifreq = struct.pack("iL", len(buf.buffer_info()[1] * 8), addr)
            fcntl.ioctl(s.fileno(), 0x8912, ifreq)  # SIOCGIFCONF
            size = struct.unpack("iL", ifreq)[0]
            data = buf.tobytes()
            for i in range(0, size, 40):
                name = data[i:i + 16].split(b"\0")[0].decode("utf-8", "replace")
                ip = ".".join(str(b) for b in data[i + 20:i + 24])
                out.append((ip, name, None))
        except Exception:
            pass

    # 过滤回环/链路本地；排序：非 TUN 名 > 接口类型物理口(6=以太网/71=Wi-Fi) > IP。
    # 隧道类型（PPP/TunnelEncapsulation 等）与虚拟口即使名字不带 TUN 特征也沉底，
    # 避免通用名隧道口（如"以太网 2"）抢占自动选择
    def rank(item):
        ip, name, iftype = item
        tun = _is_tun_name(name) or (iftype is not None and iftype in _TUN_IF_TYPES)
        phys = 0 if iftype in (6, 71) else 1
        return (1 if tun else 0, phys, ip)

    out = [(ip, n, t) for ip, n, t in out
           if ip and not ip.startswith(("127.", "169.254."))]
    seen = set()
    uniq = []
    for ip, n, _t in sorted(out, key=rank):
        if ip not in seen:
            seen.add(ip)
            uniq.append((ip, n))
    return uniq


def best_lan_ip(ips=None):
    """自动选择最优网卡 IP：首个非 TUN/VPN 物理口（列表已按物理口优先排序）；
    全是虚拟口时取首项，再兜底 UDP 出口路由（可能被 TUN 抢走，仅最后手段）"""
    pairs = ips if ips is not None else list_lan_ips()
    for ip, name in pairs:
        if not _is_tun_name(name):
            return ip
    if pairs:
        return pairs[0][0]
    # 兜底：UDP 出口路由（可能被 TUN 抢走，仅最后手段）
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.settimeout(0.5)
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
        s.close()
        if ip and not ip.startswith("127."):
            return ip
    except Exception:
        pass
    return "127.0.0.1"


# Windows GetAdaptersAddresses (AF_INET, 无额外依赖)
def _win_adapters():
    class SOCKADDR(ctypes.Structure):
        _fields_ = [("sa_family", wt.USHORT), ("sa_data", ctypes.c_byte * 14)]

    class SOCKET_ADDRESS(ctypes.Structure):
        _fields_ = [("lpSockaddr", ctypes.c_void_p),
                    ("iSockaddrLength", ctypes.c_int)]

    class IP_ADAPTER_UNICAST_ADDRESS(ctypes.Structure):
        class _u(ctypes.Union):
            _fields_ = [("Alignment", ctypes.c_ulonglong)]
        _fields_ = [("_u", _u),
                    ("Next", ctypes.c_void_p),
                    ("Address", SOCKET_ADDRESS),
                    ("PrefixOrigin", ctypes.c_int), ("SuffixOrigin", ctypes.c_int),
                    ("DadState", ctypes.c_int), ("ValidLifetime", wt.ULONG),
                    ("PreferredLifetime", wt.ULONG), ("LeaseLifetime", wt.ULONG),
                    ("OnLinkPrefixLength", ctypes.c_ubyte)]

    class IP_ADAPTER_ADDRESSES(ctypes.Structure):
        class _u(ctypes.Union):
            # Alignment 联合体内含 Length + IfIndex（勿再单独声明 IfIndex）
            _fields_ = [("Alignment", ctypes.c_ulonglong)]
        _fields_ = [("_u", _u),
                    ("Next", ctypes.c_void_p),
                    ("AdapterName", ctypes.c_char_p),
                    ("FirstUnicastAddress", ctypes.c_void_p),
                    ("FirstAnycastAddress", ctypes.c_void_p),
                    ("FirstMulticastAddress", ctypes.c_void_p),
                    ("FirstDnsServerAddress", ctypes.c_void_p),
                    ("DnsSuffix", ctypes.c_wchar_p),
                    ("Description", ctypes.c_wchar_p),
                    ("FriendlyName", ctypes.c_wchar_p),
                    ("PhysicalAddress", ctypes.c_byte * 8),
                    ("PhysicalAddressLength", wt.ULONG),
                    ("Flags", wt.DWORD), ("Mtu", wt.ULONG), ("IfType", wt.DWORD),
                    ("OperStatus", ctypes.c_int)]

    GAA_FLAG_SKIP_ANYCAST = 2
    GAA_FLAG_SKIP_MULTICAST = 4
    AF_INET = 2
    out = []
    try:
        iphlpapi = ctypes.windll.iphlpapi
        iphlpapi.GetAdaptersAddresses.argtypes = [
            wt.ULONG, wt.ULONG, ctypes.c_void_p, ctypes.c_void_p,
            ctypes.POINTER(wt.ULONG)]
        iphlpapi.GetAdaptersAddresses.restype = wt.ULONG
        size = wt.ULONG(16384)
        buf = None
        for _ in range(3):
            buf = ctypes.create_string_buffer(size.value)
            r = iphlpapi.GetAdaptersAddresses(
                AF_INET, GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST,
                None, ctypes.byref(buf), ctypes.byref(size))
            if r == 0:
                break
            if r != 111:  # ERROR_BUFFER_OVERFLOW
                return out
        else:
            return out
        addr = ctypes.cast(ctypes.byref(buf), ctypes.POINTER(IP_ADAPTER_ADDRESSES))
        while addr:
            a = addr.contents
            uni = a.FirstUnicastAddress
            while uni:
                u = ctypes.cast(
                    uni, ctypes.POINTER(IP_ADAPTER_UNICAST_ADDRESS)).contents
                if u.Address.lpSockaddr:
                    sa = ctypes.cast(
                        u.Address.lpSockaddr, ctypes.POINTER(SOCKADDR)).contents
                    if sa.sa_family == AF_INET:
                        # sockaddr_in: family(2B) + port(2B) + addr(4B)，sa_data 从 port 起
                        ip = socket.inet_ntoa(bytes(b & 0xFF for b in sa.sa_data[2:6]))
                        out.append((ip, a.FriendlyName or "", a.IfType, a.OperStatus))
                uni = u.Next
            if not a.Next:
                break
            addr = ctypes.cast(a.Next, ctypes.POINTER(IP_ADAPTER_ADDRESSES))
        return out
    except Exception:
        return out
