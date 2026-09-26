#!/usr/bin/env python3
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

"""PureVox Web 单 HTML 的本地 HTTPS 伺服（运行入口）。

为什么必须起服务：浏览器只在**安全上下文**里给麦克风权限、才允许 WebRTC，
安全上下文 = HTTPS 或 localhost。单 HTML 直接双击（file://）拿不到麦克风，
所以这里用 HTTPS 把 purevox-web/dist/ 伺服起来，手机与电脑都能开。

证书直接复用桌面端那一份（server/tls_manager.py 的 PureVox Local CA 与
server 证书，缓存在 ~/.purevox/ca/）——整个产品只需信任一次自签证书。

用法：
    python purevox-web/build/build_web.py            # 先打包
    python purevox-web/serve.py                      # 起 HTTPS（默认 59124）
    python purevox-web/serve.py --port 8443 --open   # 换端口并自动开浏览器
    python purevox-web/serve.py --no-tls             # 仅本机调试用（http://127.0.0.1
                                                      #  本身也是安全上下文，局域网不行）
"""

import argparse
import http.server
import os
import socketserver
import sys
import threading
import webbrowser

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DIST = os.path.join(HERE, "dist")
DEFAULT_PORT = 59124          # 与桌面端 WSS 的 59123 错开，避免同机互撞

PAGES = [
    ("mic.html", "麦克风降噪（对应 lite_mic）"),
    ("net.html", "网络降噪（对应 lite_net，WebRTC 收发）"),
]


def lan_ips():
    """本机局域网 IPv4（复用 pvplatform.netinfo，与桌面端同一份网卡枚举）"""
    sys.path.insert(0, ROOT)
    from pvplatform import netinfo
    try:
        return [ip for ip, _name in netinfo.list_lan_ips()]
    except Exception:
        return []


def build_ssl_ctx(ips):
    """复用桌面端的 CA/服务器证书；SAN 没覆盖当前网卡就重签。"""
    sys.path.insert(0, ROOT)
    from server.tls_manager import TlsManager
    tls = TlsManager()
    tls.ensure_ca()
    want = list(ips) + ["127.0.0.1"]
    tls.generate_server_cert(want, force=not tls.server_cert_covers(want))
    return tls.get_ssl_context(), tls.get_ca_cert_pem()


class Handler(http.server.SimpleHTTPRequestHandler):
    """只伺服 dist/。

    重资源（assets/ 下的 ORT 运行时与模型，约 22MB）按稳定 URL 长期缓存：
    URL 不变即不失效，刷新页面/切 flavor 都不重传。页面本体给 no-cache，
    改页面后立刻能看到新版本（条件请求走 304，代价极小）。
    """

    def __init__(self, *a, **kw):
        super().__init__(*a, directory=DIST, **kw)

    def end_headers(self):
        path = self.path.split("?", 1)[0]
        if path.startswith("/assets/"):
            self.send_header("Cache-Control", "public, max-age=31536000, immutable")
        else:
            self.send_header("Cache-Control", "no-cache")
        super().end_headers()

    def log_message(self, fmt, *args):
        sys.stdout.write("[web] %s - %s\n" % (self.address_string(), fmt % args))
        sys.stdout.flush()


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    ap = argparse.ArgumentParser(description="PureVox Web 本地 HTTPS 伺服")
    ap.add_argument("--port", type=int, default=DEFAULT_PORT, help="端口（默认 %d）" % DEFAULT_PORT)
    ap.add_argument("--no-tls", action="store_true",
                    help="不启用 HTTPS（只适合 http://127.0.0.1 本机调试）")
    ap.add_argument("--open", action="store_true", help="启动后自动打开浏览器")
    ap.add_argument("--host", default="0.0.0.0", help="监听地址（默认全网卡，供手机访问）")
    args = ap.parse_args()

    if not os.path.isdir(DIST):
        raise SystemExit("没有产物目录 %s —— 先跑 python purevox-web/build/build_web.py" % DIST)
    missing = [p for p, _ in PAGES if not os.path.isfile(os.path.join(DIST, p))]
    if missing:
        raise SystemExit("产物缺失：%s —— 先跑 python purevox-web/build/build_web.py"
                         % "、".join(missing))

    ips = lan_ips()
    scheme = "http"
    ctx = None
    ca_pem = None
    if not args.no_tls:
        try:
            ctx, ca_pem = build_ssl_ctx(ips)
            scheme = "https"
        except Exception as e:
            print("[web] TLS 初始化失败（%s），退回 http" % e)
            print("[web] 注意：非 localhost 的 http 地址不是安全上下文，麦克风用不了")

    httpd = Server((args.host, args.port), Handler)
    if ctx is not None:
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)

    url = "%s://127.0.0.1:%d/" % (scheme, args.port)
    print("")
    print("  PureVox Web —— 本地%s伺服已启动" % ("HTTPS" if ctx is not None else "HTTP"))
    print("  " + "-" * 58)
    for page, desc in PAGES:
        print("    %-24s %s" % (page, desc))
    print("  " + "-" * 58)
    print("  本机：%s" % url)
    for ip in ips:
        print("  局域网：%s://%s:%d/   （手机同网段可直接开）" % (scheme, ip, args.port))
    print("")
    print("  重资源（assets/ 下约 22MB 的运行时与模型）按稳定 URL 长期缓存，")
    print("  首次下载后切 flavor / 刷新页面都不再重复下载。")
    if ctx is not None:
        print("")
        print("  首次访问：浏览器会报「证书不受信任」→ 点「高级 → 继续前往」。")
        print("  信任这一次之后麦克风与 WebRTC 才可用（浏览器只在安全上下文授权）。")
        try:
            ca_path = os.path.join(os.path.expanduser("~"), ".purevox", "ca", "ca.crt")
            print("  想彻底免警告：把该 CA 导入系统信任库 → %s" % ca_path)
        except Exception:
            pass
    print("")
    print("  Ctrl+C 停止")
    print("")

    if args.open:
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()

    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n[web] 已停止")
    finally:
        httpd.shutdown()
        httpd.server_close()


if __name__ == "__main__":
    main()
