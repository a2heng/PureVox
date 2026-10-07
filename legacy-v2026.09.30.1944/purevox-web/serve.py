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

"""PureVox Web 的本地服务器（运行入口）。

它只当"服务器"，一行音频逻辑都不做：

  1. 伺服 dist/ 的静态页面（HTTPS）
  2. GET /api/lan   —— 报本机局域网网卡列表（页面据此渲染 IP 下拉；
                       浏览器没有任何 API 能枚举本机局域网 IP，只能服务端给）
  3. WS  /ws/up     —— 输入端推上来的音频字节，原样入队
  4. WS  /ws/down   —— 输出端订阅，把队列里的字节原样推给它

**不解码、不重采样、不混音、不降噪**：解码与降噪全在网页里（out.html），
Python 只做「把浏览器的字节从 A 端倒到 B 端」。这条链路上流动的字节是
**小端 Int16 / 单声道 / 48kHz / 每帧 480 样本（10ms）**——局域网带宽不要钱，
换来的是输入端页面零编解码依赖（不要 Opus wasm、不要 base64 封装）。

为什么必须有这个进程：浏览器页面不能监听 TCP 端口，这是浏览器的硬限制。
所以「输出端在哪台机器上」就等于「那台机器上有个进程在 listen」。

安全上下文：浏览器只在 HTTPS 或 localhost 里给麦克风权限，因此默认起 HTTPS，
证书复用桌面端那份 PureVox Local CA（server/tls_manager.py，缓存在 ~/.purevox/ca/），
整个产品只需信任一次自签证书。

用法：
    python purevox-web/build/build_web.py            # 先打包
    python purevox-web/serve.py                      # 起服务器（默认 59124）
    python purevox-web/serve.py --port 8443 --open   # 换端口并自动开浏览器
    python purevox-web/serve.py --no-tls             # 仅本机调试用（http://127.0.0.1
                                                      #  本身也是安全上下文，局域网不行）
"""

import argparse
import base64
import collections
import hashlib
import http.server
import json
import os
import socket
import socketserver
import struct
import sys
import threading
import time
import webbrowser

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DIST = os.path.join(HERE, "dist")
DEFAULT_PORT = 59124          # 与桌面端 WSS 的 59123 错开，避免同机互撞

# 音频帧契约：与引擎 hop 对齐，10ms @48kHz 单声道 Int16
FRAME_SAMPLES = 480
FRAME_BYTES = FRAME_SAMPLES * 2          # Int16
SUB_QUEUE_HOPS = 30                      # 每个订阅端的有界队列（300ms），满则丢最旧
WS_IDLE_TIMEOUT = 1.0                    # 订阅端轮询间隔（秒）

PAGES = [
    ("mic.html", "麦克风降噪（对应 lite_mic，物理设备）"),
    ("out.html", "网络降噪 · 输出端（接收 + 降噪 + 播放）"),
    ("in.html", "网络降噪 · 输入端（麦克风采集 + 发送）"),
]

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

OP_CONT, OP_TEXT, OP_BIN, OP_CLOSE, OP_PING, OP_PONG = 0x0, 0x1, 0x2, 0x8, 0x9, 0xA


def lan_ips():
    """本机局域网 IPv4（复用 pvplatform.netinfo，与桌面端同一份网卡枚举）"""
    sys.path.insert(0, ROOT)
    from pvplatform import netinfo
    try:
        return list(netinfo.list_lan_ips())
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


# ── 最小 WebSocket（RFC 6455）服务端实现 ──────────────────────────────
# 只需要三件事：握手应答、把客户端的掩码帧解出来、把不掩码的帧发出去。
# 不引第三方依赖（本文件要能只靠标准库跑起来），也不碰 TLS 之外的任何协议。

def _ws_accept_key(key):
    digest = hashlib.sha1((key + WS_GUID).encode("ascii")).digest()
    return base64.b64encode(digest).decode("ascii")


def _ws_frame(payload, opcode=OP_BIN):
    """服务端 → 客户端的帧：一律不掩码。"""
    n = len(payload)
    if n < 126:
        head = bytes([0x80 | opcode, n])
    elif n < 65536:
        head = bytes([0x80 | opcode, 126]) + struct.pack(">H", n)
    else:
        head = bytes([0x80 | opcode, 127]) + struct.pack(">Q", n)
    return head + payload


class WsClosed(Exception):
    """对端关闭或链路断开。"""


class WsConn:
    """服务端侧的一条 WebSocket 连接（阻塞读写，跑在 handler 线程里）。"""

    def __init__(self, rfile, wfile, sock):
        self._rfile = rfile
        self._wfile = wfile
        self._sock = sock
        self._wlock = threading.Lock()
        self.closed = False

    # -- 低层读 ---------------------------------------------------------
    def _read(self, n):
        # 直接在阻塞 socket 上循环凑满：BufferedReader.read(n) 在 socket 上
        # 会短读（一次只返回已到的字节），循环是必须的；b"" 即真 EOF。
        if n == 0:
            return b""
        buf = b""
        while len(buf) < n:
            chunk = self._sock.recv(n - len(buf))
            if not chunk:
                raise WsClosed()
            buf += chunk
        return buf

    def _write(self, data):
        with self._wlock:
            if self.closed:
                raise WsClosed()
            self._wfile.write(data)
            self._wfile.flush()

    # -- 帧 -------------------------------------------------------------
    def recv(self):
        """读一帧（自动处理 ping / close / 分片），返回 (opcode, payload)。"""
        frags = []
        first_op = None
        while True:
            b0, b1 = self._read(2)
            fin = bool(b0 & 0x80)
            op = b0 & 0x0F
            masked = bool(b1 & 0x80)
            ln = b1 & 0x7F
            if ln == 126:
                ln = struct.unpack(">H", self._read(2))[0]
            elif ln == 127:
                ln = struct.unpack(">Q", self._read(8))[0]
            if ln > (1 << 20):            # 单帧上限 1 MiB：音频帧只有 960 B
                raise WsClosed()
            mask = self._read(4) if masked else None
            data = self._read(ln) if ln else b""
            if mask:
                data = bytes(b ^ mask[i & 3] for i, b in enumerate(data))

            if op == OP_CLOSE:
                raise WsClosed()
            if op == OP_PING:
                self._write(_ws_frame(data, OP_PONG))
                continue
            if op == OP_PONG:
                continue
            if op == OP_CONT:
                frags.append(data)
            else:
                first_op = op
                frags = [data]
            if fin:
                return first_op, b"".join(frags)

    def send_binary(self, data):
        self._write(_ws_frame(data, OP_BIN))

    def send_text(self, text):
        self._write(_ws_frame(text.encode("utf-8"), OP_TEXT))

    def close(self):
        if self.closed:
            return
        self.closed = True
        try:
            self._write(_ws_frame(b"", OP_CLOSE))
        except Exception:
            pass


class Relay:
    """单上游 → 多订阅端的字节中继。

    上游只认「最新连上的那一个」：新的推送者接管，上一位被请下去。
    这不是混音——「一个麦克风 → 一路降噪输出」就是 Lite Net 的语义，
    两个人同时推时明确地只留一个，不做叠加。
    """

    def __init__(self):
        self._lock = threading.Lock()
        self._subs = []                 # [Subscriber]
        self._up = None                 # 当前上游 Subscriber

    def add_sub(self, sub):
        with self._lock:
            self._subs.append(sub)

    def drop_sub(self, sub):
        with self._lock:
            if sub in self._subs:
                self._subs.remove(sub)
            if self._up is sub:
                self._up = None

    def set_up(self, sub):
        with self._lock:
            old = self._up
            self._up = sub
        if old is not None and old is not sub:
            old.superseded = True

    def clear_up(self, sub):
        with self._lock:
            if self._up is sub:
                self._up = None

    def has_sub(self):
        with self._lock:
            return bool(self._subs)

    def mark_eof(self):
        """上游断了：通知所有订阅端收尾（它们各自退出并关连接）。"""
        with self._lock:
            for sub in self._subs:
                sub.eof = True

    def push(self, payload):
        with self._lock:
            targets = list(self._subs)
        for sub in targets:
            sub.enqueue(payload)


class Subscriber:
    """一个订阅端（输出页）的待发队列 + 生命周期标记。"""

    def __init__(self, conn):
        self.conn = conn
        self.queue = collections.deque(maxlen=SUB_QUEUE_HOPS)
        self.lock = threading.Lock()
        self.superseded = False          # 被新上游接管
        self.eof = False                 # 上游已断开

    def enqueue(self, payload):
        with self.lock:
            self.queue.append(payload)   # deque maxlen 满了自动丢最旧

    def take(self):
        with self.lock:
            return self.queue.popleft() if self.queue else None


RELAY = Relay()


class Handler(http.server.SimpleHTTPRequestHandler):
    """静态伺服 dist/ + 两个 WebSocket 端点 + /api/lan。"""

    protocol_version = "HTTP/1.1"
    server_version = "PureVoxWeb"

    def __init__(self, *a, **kw):
        super().__init__(*a, directory=DIST, **kw)

    # -- 静态资源的缓存策略（重资源按稳定 URL 长期缓存）------------------
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

    def _json(self, obj):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-cache")
        self._cors_headers()
        self.end_headers()
        self.wfile.write(body)

    def _cors_headers(self):
        # 跨站部署（页面在 github.io、中继在局域网）时浏览器要这两件：
        # CORS 放行任意源（局域网接口，无隐私数据）+ Private Network Access
        # 预检放行（公网页连私网地址，Chrome 会先发 OPTIONS 问一句）。
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Private-Network", "true")

    def do_OPTIONS(self):
        path = self.path.split("?", 1)[0]
        if path not in ("/api/lan", "/ws/up", "/ws/down"):
            self.send_error(404)
            return
        self.send_response(204)
        self._cors_headers()
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/api/lan":
            self._json({
                "port": self.server.server_address[1],
                "frameSamples": FRAME_SAMPLES,
                "frameBytes": FRAME_BYTES,
                "ips": [{"ip": ip, "name": name} for ip, name in lan_ips()],
            })
            return
        if path in ("/ws/up", "/ws/down"):
            self._websocket(path)
            return
        super().do_GET()

    # -- WebSocket -------------------------------------------------------
    def _websocket(self, path):
        key = self.headers.get("Sec-WebSocket-Key")
        upgrade = (self.headers.get("Upgrade") or "").lower()
        if not key or upgrade != "websocket":
            self.send_error(400, "expected websocket upgrade")
            return
        self.send_response(101, "Switching Protocols")
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", _ws_accept_key(key))
        self.send_header("Access-Control-Allow-Private-Network", "true")
        self.end_headers()
        # 劫持连接：后面全是 WS 帧，不再是 HTTP，不许 handle() 循环解析下一个请求
        self.close_connection = True
        try:
            self.connection.settimeout(None)
        except Exception:
            pass
        conn = WsConn(self.rfile, self.wfile, self.connection)
        try:
            if path == "/ws/up":
                self._serve_up(conn)
            else:
                self._serve_down(conn)
        except (WsClosed, OSError, ValueError):
            pass
        except Exception as e:                     # 兜底：不让一条坏连接带倒伺服
            self.log_message("ws %s 异常: %s", path, e)
        finally:
            conn.close()

    def _serve_up(self, conn):
        """输入端：收一帧就转给所有订阅端，不看内容、不做处理。"""
        sub = Subscriber(conn)
        RELAY.set_up(sub)
        self.log_message("输入端已连接（%d 个输出端在听）", 1 if RELAY.has_sub() else 0)
        frames = 0
        last = time.time()
        try:
            while True:
                op, data = conn.recv()
                if op == OP_TEXT:
                    # 唯一认的控制消息：上游主动结束
                    try:
                        msg = json.loads(data.decode("utf-8"))
                    except Exception:
                        msg = {}
                    if msg.get("type") == "stop":
                        break
                    continue
                if op != OP_BIN or not data:
                    continue
                if len(data) % FRAME_BYTES:
                    # 不是整 hop 的帧直接丢弃——不做补齐/拼接，网格必须对齐
                    continue
                RELAY.push(data)
                frames += 1
                now = time.time()
                if now - last >= 5.0:
                    last = now
                    self.log_message("输入端已推 %.1f 秒音频", frames * FRAME_SAMPLES / 48000.0)
        finally:
            RELAY.clear_up(sub)
            RELAY.mark_eof()
            self.log_message("输入端已断开")

    def _serve_down(self, conn):
        """输出端：订阅队列，把上游的字节原样推过去。"""
        sub = Subscriber(conn)
        RELAY.add_sub(sub)
        self.log_message("输出端已连接")
        try:
            conn.send_text(json.dumps({"type": "hello", "frameBytes": FRAME_BYTES,
                                       "frameSamples": FRAME_SAMPLES}))
            while True:
                if sub.superseded:
                    break
                data = sub.take()
                if data is None:
                    if sub.eof:
                        break
                    time.sleep(0.005)
                    continue
                conn.send_binary(data)
        finally:
            RELAY.drop_sub(sub)
            self.log_message("输出端已断开")


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    ap = argparse.ArgumentParser(description="PureVox Web 本地服务器")
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

    ips = [ip for ip, _name in lan_ips()]
    scheme = "http"
    ctx = None
    if not args.no_tls:
        try:
            ctx, _ca_pem = build_ssl_ctx(ips)
            scheme = "https"
        except Exception as e:
            print("[web] TLS 初始化失败（%s），退回 http" % e)
            print("[web] 注意：非 localhost 的 http 地址不是安全上下文，麦克风用不了")

    httpd = Server((args.host, args.port), Handler)
    if ctx is not None:
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)

    base = "%s://127.0.0.1:%d" % (scheme, args.port)
    print("")
    print("  PureVox Web —— 本地%s服务器已启动" % ("HTTPS" if ctx is not None else "HTTP"))
    print("  " + "-" * 58)
    for page, desc in PAGES:
        print("    %-24s %s" % (page, desc))
    print("  " + "-" * 58)
    print("  本机：%s" % base)
    for ip in ips:
        print("  局域网：%s://%s:%d/   （手机同网段可直接开）" % (scheme, ip, args.port))
    print("")
    print("  用法：在电脑上打开 out.html（选局域网 IP → 页面出二维码），")
    print("        手机扫二维码打开 in.html，两边都点「启动」即连通。")
    print("  音频格式：Int16 / 单声道 / 48kHz / 每帧 %d 样本（10ms）；"
          % FRAME_SAMPLES)
    print("  本进程不解码、不降噪，只把输入端推上来的字节转给输出端。")
    print("")
    print("  重资源（assets/ 下约 22MB 的运行时与模型）按稳定 URL 长期缓存，")
    print("  首次下载后切页面 / 刷新都不再重复下载。")
    if ctx is not None:
        print("")
        print("  首次访问：浏览器会报「证书不受信任」→ 点「高级 → 继续前往」。")
        print("  信任这一次之后麦克风才可用（浏览器只在安全上下文授权）。")
        try:
            ca_path = os.path.join(os.path.expanduser("~"), ".purevox", "ca", "ca.crt")
            print("  想彻底免警告：把该 CA 导入系统信任库 → %s" % ca_path)
        except Exception:
            pass
    print("")
    print("  Ctrl+C 停止")
    print("")

    if args.open:
        threading.Timer(0.5, lambda: webbrowser.open(base + "/out.html")).start()

    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n[web] 已停止")
    finally:
        httpd.shutdown()
        httpd.server_close()


if __name__ == "__main__":
    main()
