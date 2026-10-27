#!/usr/bin/env python3
# PureVox network reconnect stress (TLS WebSocket, self-signed cert allowed).
#
# Simulates the phone/网页 "重新连接": do a real WS handshake, send hello + sub,
# then close the TCP socket without a WS close frame, and repeat.
#
# Usage:
#   python ws-reconnect.py 59123 20
#   python ws-reconnect.py 59123 20 --host 192.168.1.5 --json
#
# Exit code 0 = all rounds completed. It does NOT prove the UI stayed connected;
# check that the PC process is alive and its log has no "panicked" (see SKILL.md).
import argparse
import base64
import json
import os
import socket
import ssl
import struct
import sys


def recv_exact(sock, n):
    buf = b''
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError('connection closed')
        buf += chunk
    return buf


def send_text(sock, text):
    data = text.encode()
    mask = os.urandom(4)
    n = len(data)
    hdr = bytearray([0x81])
    if n < 126:
        hdr.append(0x80 | n)
    elif n < 65536:
        hdr.append(0x80 | 126)
        hdr += struct.pack('!H', n)
    else:
        hdr.append(0x80 | 127)
        hdr += struct.pack('!Q', n)
    hdr += mask
    hdr += bytes(b ^ mask[i % 4] for i, b in enumerate(data))
    sock.sendall(bytes(hdr))


def recv_frame(sock):
    b0, b1 = recv_exact(sock, 2)
    op = b0 & 0x0F
    n = b1 & 0x7F
    if n == 126:
        n = struct.unpack('!H', recv_exact(sock, 2))[0]
    elif n == 127:
        n = struct.unpack('!Q', recv_exact(sock, 8))[0]
    if b1 & 0x80:
        mask = recv_exact(sock, 4)
        data = bytes(b ^ mask[i % 4] for i, b in enumerate(recv_exact(sock, n)))
    else:
        data = recv_exact(sock, n)
    return op, data


def connect(host, port):
    ctx = ssl._create_unverified_context()  # 自签证书是设计（DESIGN.md §4.1.12）
    raw = socket.create_connection((host, port), timeout=8)
    sock = ctx.wrap_socket(raw, server_hostname=host)
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        'GET /ws HTTP/1.1\r\n'
        f'Host: {host}:{port}\r\n'
        'Upgrade: websocket\r\n'
        'Connection: Upgrade\r\n'
        f'Sec-WebSocket-Key: {key}\r\n'
        'Sec-WebSocket-Version: 13\r\n\r\n'
    )
    sock.sendall(req.encode())
    head = b''
    while b'\r\n\r\n' not in head:
        head += sock.recv(1)
    if b' 101 ' not in head:
        raise RuntimeError(head[:200])
    return sock


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('port', type=int)
    ap.add_argument('rounds', type=int, nargs='?', default=20)
    ap.add_argument('--host', default='127.0.0.1')
    ap.add_argument('--json', action='store_true', help='print a JSON summary')
    args = ap.parse_args()

    targets = []
    for i in range(args.rounds):
        sock = connect(args.host, args.port)
        send_text(sock, json.dumps({'t': 'hello', 'proto': 3}))
        _, data = recv_frame(sock)
        ready = json.loads(data)
        if ready.get('t') == 'ready':
            targets.append(ready.get('target_ms'))
        send_text(sock, json.dumps({'t': 'sub'}))
        recv_frame(sock)  # ready again
        # 模拟「重新连接」：不发 close 帧，直接断 TCP
        sock.close()
    summary = {
        'ok': True,
        'rounds': args.rounds,
        'host': args.host,
        'port': args.port,
        'target_ms': targets,
    }
    if args.json:
        print(json.dumps(summary, ensure_ascii=False))
    else:
        print(f"{args.rounds} rounds ok; ready.target_ms = {sorted(set(targets))}")
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001 - 诊断脚本，失败要人话
        print(f"reconnect test failed: {e}", file=sys.stderr)
        sys.exit(1)
