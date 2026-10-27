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
"""网络回路测试：测试音 → 网络扬声器 → 本地 WSS 客户端，量收到的 1 kHz tone。

它做的是**整条下行链路**：启动一个隔离的 PureVox（临时 HOME + 临时端口，不碰用户的
`~/.purevox`），计划固定为 ``tone → remote_speaker``，等 HTTPS `/health` 就绪后连
`/ws`、订阅、收 Opus 包、解码成 PCM，输出电平 / 频率 / 谐波 / 断点 / 静音段 / 包节奏。

用法：
    python tools/automation/net_loopback.py                    # 自动找 exe / 模型 / 空闲端口
    python tools/automation/net_loopback.py --seconds 15 --json out.json --out out.wav
    python tools/automation/net_loopback.py --reuse --port 59123  # 不启动，连已有实例
    python tools/automation/net_loopback.py --from-dump received.wav  # 离线复算
    python tools/automation/net_loopback.py --list              # 列出候选 exe / opus / 模型

输出两层（AGENTS.md §1.4）：
    stdout 给人看的一行结论 + 关键数字；``--json PATH`` 写结构化结果（失败也写）。
    ``--out PATH`` 落一份可听的 WAV（16-bit）+ 同名 `.f32` 原始样本。

依赖：numpy（与 `aec_probe.py` 同一取舍）；Opus 解码走运行时的 `opus.dll` /
`libopus.so.0`（与 App 同一份库），没有时只量包节奏、不假装解出了波形。
不依赖 websockets / scipy：WS 客户端是标准库实现，频谱用 numpy FFT。
"""

from __future__ import annotations

import argparse
import base64
import ctypes
import ctypes.util
import json
import math
import os
import platform as _platform
import shutil
import signal
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import wave
from pathlib import Path

try:
    import numpy as np
except ImportError:  # pragma: no cover - 工具入口
    sys.exit("缺少 numpy：python -m pip install numpy（与 tools/automation/aec_probe.py 相同）")

SCHEMA = 1


def client_us() -> int:
    """本进程的单调时钟（µs，u32 回绕）——与客户端的帧头同一口径。"""
    return int(time.perf_counter() * 1_000_000) & 0xFFFFFFFF


# Windows 的 sleep 粒度默认 15.6 ms：不提升的话，**测试发送端自己**就会把 10 ms 序列
# 变成「憋 30 ms 再连发 3 包」，量到的 gap 全是假象（实测 34~41 ms，而链路真实值是 1~2 ms）。
# 只影响本进程，退出时恢复（`finally`）。
_TIMER_RAISED = False


def _raise_timer() -> None:
    global _TIMER_RAISED
    if sys.platform.startswith("win") and not _TIMER_RAISED:
        try:
            import ctypes

            ctypes.windll.winmm.timeBeginPeriod(1)
            _TIMER_RAISED = True
        except Exception:  # noqa: BLE001
            pass


def _lower_timer() -> None:
    global _TIMER_RAISED
    if _TIMER_RAISED:
        try:
            import ctypes

            ctypes.windll.winmm.timeEndPeriod(1)
        except Exception:  # noqa: BLE001
            pass
        _TIMER_RAISED = False


SR = 48000
HOP = 480
PROTO = 3
HDR = 8  # 音频帧头：seq u32 + 发送方 µs u32（大端），见 src-tauri/src/net/rt.rs
TONE_HZ = 1000.0
TONE_AMP = 0.1  # -20 dBFS
TONE_THEORY = 2 * math.pi * TONE_HZ / SR * TONE_AMP
REPO = Path(__file__).resolve().parents[2]
DEFAULT_HOME = Path(tempfile.gettempdir()) / "purevox-net-loopback"
PLAN = {
    "columns": [
        {
            "enabled": True,
            "rows": [
                {"kind": "input", "ptype": "tone", "device": None, "enabled": True, "params": {}},
                {"kind": "output", "ptype": "remote_speaker", "device": None, "enabled": True, "params": {}},
            ],
        }
    ]
}
# 上行回环：客户端推 Opus → `remote_mic` 行 → 引擎 → `remote_speaker` 行 → 客户端订阅收回。
# 两个方向共用同一套 hub 队列/10 ms 节拍，只是 tap 的位置不同（DESIGN.md §4.1）。
PLAN_ROUNDTRIP = {
    "columns": [
        {
            "enabled": True,
            "rows": [
                {"kind": "input", "ptype": "remote_mic", "device": None, "enabled": True, "params": {}},
                {"kind": "output", "ptype": "remote_speaker", "device": None, "enabled": True, "params": {}},
            ],
        }
    ]
}


# ---------------------------------------------------------------- 平台 / 资源


def os_name(platform_arg: str) -> str:
    if platform_arg != "auto":
        return platform_arg
    if sys.platform.startswith("win"):
        return "windows"
    if sys.platform.startswith("linux"):
        return "linux"
    if sys.platform == "darwin":
        return "macos"
    return "other"


def default_exe(osn: str) -> Path | None:
    env = os.environ.get("PUREVOX_EXE")
    if env:
        return Path(env)
    names = ["purevox.exe", "purevox"]
    dirs = [
        REPO / "src-tauri" / "target" / "release",
        REPO / "src-tauri" / "target" / "debug",
        Path("/usr/bin"),
        Path("/usr/local/bin"),
    ]
    for d in dirs:
        for n in names:
            p = d / n
            if p.is_file():
                return p
    return None


def default_models() -> Path | None:
    env = os.environ.get("PUREVOX_MODEL_DIR")
    if env:
        return Path(env)
    p = REPO / "models"
    return p if p.is_dir() else None


def opus_candidates(osn: str) -> list[Path]:
    out: list[Path] = []
    env = os.environ.get("PUREVOX_OPUS")
    if env:
        out.append(Path(env))
    if osn == "windows":
        for base in (REPO / "src-tauri", REPO / "server", Path.cwd()):
            out.append(base / "opus.dll")
        if (exe := default_exe(osn)) is not None:
            out.append(exe.parent / "opus.dll")
    else:
        for n in ("libopus.so.0", "libopus.so", "libopus.dylib", "libopus.0.dylib"):
            out.append(Path(n))
    found = ctypes.util.find_library("opus")
    if found:
        out.append(Path(found))
    return out


class Opus:
    def __init__(self, lib_path: Path | None):
        if lib_path is None:
            raise FileNotFoundError("找不到 libopus（可用 --opus / PUREVOX_OPUS 指定）")
        self.path = str(lib_path)
        self.lib = ctypes.CDLL(self.path)
        self.lib.opus_decoder_create.restype = ctypes.c_void_p
        self.lib.opus_decoder_create.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
        self.lib.opus_decode_float.restype = ctypes.c_int
        self.lib.opus_decode_float.argtypes = [
            ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int32,
            ctypes.POINTER(ctypes.c_float), ctypes.c_int, ctypes.c_int,
        ]
        self.lib.opus_decoder_destroy.argtypes = [ctypes.c_void_p]
        err = ctypes.c_int(0)
        self.dec = self.lib.opus_decoder_create(SR, 1, ctypes.byref(err))
        if not self.dec or err.value != 0:
            raise RuntimeError(f"opus_decoder_create 失败（err={err.value}）")

    def decode(self, pkt: bytes) -> np.ndarray:
        buf = (ctypes.c_float * 5760)()
        n = self.lib.opus_decode_float(self.dec, pkt, len(pkt), buf, 5760, 0)
        if n < 0:
            raise RuntimeError(f"opus_decode_float 失败（err={n}）")
        return np.frombuffer(buf, dtype=np.float32, count=n).copy()

    def close(self) -> None:
        if getattr(self, "dec", None):
            self.lib.opus_decoder_destroy(self.dec)
            self.dec = None


class OpusEncoder:
    """只给 `--mode roundtrip` 用：模拟手机往上推 10 ms 的 Opus 包。"""

    def __init__(self, lib_path: Path | None):
        if lib_path is None:
            raise FileNotFoundError("找不到 libopus（可用 --opus / PUREVOX_OPUS 指定）")
        self.lib = ctypes.CDLL(str(lib_path))
        self.lib.opus_encoder_create.restype = ctypes.c_void_p
        self.lib.opus_encoder_create.argtypes = [
            ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int),
        ]
        self.lib.opus_encode_float.restype = ctypes.c_int
        self.lib.opus_encode_float.argtypes = [
            ctypes.c_void_p, ctypes.POINTER(ctypes.c_float), ctypes.c_int,
            ctypes.POINTER(ctypes.c_ubyte), ctypes.c_int32,
        ]
        self.lib.opus_encoder_destroy.argtypes = [ctypes.c_void_p]
        err = ctypes.c_int(0)
        # 与 PC 侧一致：VOIP（2048），10 ms 帧，32 kbps
        self.enc = self.lib.opus_encoder_create(SR, 1, 2048, ctypes.byref(err))
        if not self.enc or err.value != 0:
            raise RuntimeError(f"opus_encoder_create 失败（err={err.value}）")

    def encode(self, frame: np.ndarray) -> bytes:
        out = (ctypes.c_ubyte * 4000)()
        pcm = np.ascontiguousarray(frame, dtype=np.float32)
        n = self.lib.opus_encode_float(
            self.enc, pcm.ctypes.data_as(ctypes.POINTER(ctypes.c_float)), pcm.size,
            out, 4000,
        )
        if n < 0:
            raise RuntimeError(f"opus_encode_float 失败（err={n}）")
        return bytes(out[:n])

    def close(self) -> None:
        if getattr(self, "enc", None):
            self.lib.opus_encoder_destroy(self.enc)
            self.enc = None


# ---------------------------------------------------------------- 极简 WSS 客户端


class Wss:
    """标准库 WebSocket 客户端（只用于本工具：文本 / 二进制、ping / pong、分片）。"""

    def __init__(self, host: str, port: int, tls: bool = True, timeout: float = 10.0,
                 path: str = "/ws"):
        self.host = host
        self.port = port
        raw = socket.create_connection((host, port), timeout=timeout)
        if tls:
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
            raw = ctx.wrap_socket(raw, server_hostname=host)
        self.sock = raw
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode("ascii"))
        head = b""
        while b"\r\n\r\n" not in head:
            c = self.sock.recv(1)
            if not c:
                raise ConnectionError("WebSocket 握手时连接被关闭")
            head += c
            if len(head) > 65536:
                raise ConnectionError("WebSocket 握手响应过长")
        status = int(head.split(b" ", 2)[1]) if head.startswith(b"HTTP/") else 0
        if status != 101:
            raise ConnectionError(f"WebSocket 握手失败：{head.split(b'\r\n', 1)[0].decode('latin1')}")
        self.buf = b""

    def _exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            d = self.sock.recv(65536)
            if not d:
                raise EOFError("WebSocket 连接已关闭")
            self.buf += d
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _send(self, op: int, payload: bytes) -> None:
        mask = os.urandom(4)
        n = len(payload)
        hdr = bytearray([0x80 | op])
        if n < 126:
            hdr.append(0x80 | n)
        elif n < 65536:
            hdr.append(0x80 | 126)
            hdr += struct.pack(">H", n)
        else:
            hdr.append(0x80 | 127)
            hdr += struct.pack(">Q", n)
        hdr += mask
        hdr += bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(hdr)

    def send_text(self, text: str) -> None:
        self._send(0x1, text.encode("utf-8"))

    def handle_control(self, text: str) -> dict | None:
        """处理后台控制消息：**必须对 `sync` 回 `syncreply`**，否则服务端估不出时钟偏移，
        `/debug` 的 `rx_latency_ms` 永远是 null（人就会以为「单向延迟测不出来」）。"""
        try:
            m = json.loads(text)
        except Exception:  # noqa: BLE001
            return None
        if m.get("t") == "sync":
            t1 = client_us()
            self.send_text(json.dumps({
                "t": "syncreply", "id": m.get("id", 0), "t0": int(m.get("t0", 0)),
                "t1": t1, "t2": client_us(),
            }))
        return m

    def send_binary(self, data: bytes) -> None:
        self._send(0x2, data)

    def recv(self, timeout: float = 3.0) -> tuple[int, bytes]:
        end = time.time() + timeout
        frag = b""
        frag_op = 0
        while True:
            left = end - time.time()
            if left <= 0:
                raise TimeoutError("等待 WebSocket 消息超时")
            self.sock.settimeout(left)
            b0, b1 = self._exact(2)
            fin = bool(b0 & 0x80)
            op = b0 & 0x0F
            masked = bool(b1 & 0x80)
            ln = b1 & 0x7F
            if ln == 126:
                ln = struct.unpack(">H", self._exact(2))[0]
            elif ln == 127:
                ln = struct.unpack(">Q", self._exact(8))[0]
            mask = self._exact(4) if masked else None
            data = self._exact(ln) if ln else b""
            if mask:
                data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
            if op == 0x9:  # ping
                self._send(0xA, data)
                continue
            if op == 0xA:  # pong
                continue
            if op == 0x8:
                raise EOFError("服务端关闭了 WebSocket")
            if op == 0x0:
                frag += data
            else:
                frag, frag_op = data, op
            if fin:
                return frag_op, frag

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


# ---------------------------------------------------------------- App 生命周期


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def write_config(home: Path, port: int, plan: dict | None = None) -> Path:
    cfg = home / ".purevox"
    cfg.mkdir(parents=True, exist_ok=True)
    (cfg / "session.json").write_text(
        json.dumps(plan or PLAN, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    settings = {
        "lang": "zh",
        "hotkey_on": False,
        "cue_on": False,
        "start_hidden": True,
        "net_port": port,
        "net_ip": "",
        "window": None,
    }
    (cfg / "settings.json").write_text(json.dumps(settings, ensure_ascii=False, indent=2), encoding="utf-8")
    return cfg


def launch(exe: Path, home: Path, models: Path | None, log_path: Path) -> subprocess.Popen:
    env = os.environ.copy()
    # Windows 用 USERPROFILE，POSIX 用 HOME；两份都设，跨平台同一条路径。
    env["USERPROFILE"] = str(home)
    env["HOME"] = str(home)
    if models is not None:
        env["PUREVOX_MODEL_DIR"] = str(models)
    env["PUREVOX_AUTONET"] = "1"
    log = open(log_path, "wb")
    kwargs: dict = {"stdout": log, "stderr": subprocess.STDOUT, "env": env}
    if os.name == "nt":
        kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW
    else:
        kwargs["start_new_session"] = True
    p = subprocess.Popen([str(exe)], **kwargs)
    p._purevox_log = log  # type: ignore[attr-defined]
    return p


def terminate(proc: subprocess.Popen) -> None:
    try:
        if os.name == "nt":
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(proc.pid)],
                           capture_output=True, timeout=10)
        else:
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    except (ProcessLookupError, OSError, subprocess.SubprocessError):
        pass
    log = getattr(proc, "_purevox_log", None)
    if log:
        try:
            log.close()
        except OSError:
            pass


def log_tail(path: Path, n: int = 40) -> str:
    try:
        lines = path.read_text("utf-8", errors="replace").splitlines()
        return "\n".join(lines[-n:])
    except OSError:
        return ""


def debug_net(timeout: float = 2.0) -> dict | None:
    """读 `/debug` 的 `net` 段（AGENTS §1.2 的本机只读接口，明文 47821）。

    音频指标（脚本自己量的）与端点计数（App 自己量的）**不是同一份数据**：
    两边一起落进 JSON，出问题时才能分清「是链路不好」还是「我们哪里算错了」。
    """
    try:
        with urllib.request.urlopen("http://127.0.0.1:47821/debug", timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8")).get("net")
    except Exception:  # noqa: BLE001 - 端口被别的实例占着 / 已退出都算「拿不到」
        return None


def health(port: int, ctx: ssl.SSLContext, timeout: float = 2.0) -> dict:
    url = f"https://127.0.0.1:{port}/health"
    with urllib.request.urlopen(url, timeout=timeout, context=ctx) as r:
        return json.loads(r.read().decode("utf-8"))


def wait_health(proc: subprocess.Popen, port: int, ctx: ssl.SSLContext,
                log_path: Path, timeout: float = 40.0) -> dict:
    end = time.time() + timeout
    last = "unknown"
    while time.time() < end:
        if proc.poll() is not None:
            raise RuntimeError(f"应用启动后退出（code={proc.returncode}）：\n{log_tail(log_path)}")
        try:
            d = health(port, ctx)
            if d.get("app") == "PureVox":
                return d
            last = f"health 不是 PureVox：{d}"
        except Exception as e:  # noqa: BLE001 - 启动期连接失败是常态
            last = str(e)
        time.sleep(0.3)
    raise RuntimeError(f"{timeout:.0f}s 内 /health 未就绪：{last}\n{log_tail(log_path)}")


# ---------------------------------------------------------------- 采集 / 分析


def collect(port: int, seconds: float, opus_path: Path | None) -> dict:
    t0 = time.time()
    ws = Wss("127.0.0.1", port, tls=True)
    dec = None
    opus_err = ""
    if opus_path is not None:
        try:
            dec = Opus(opus_path)
        except Exception as e:  # noqa: BLE001
            opus_err = str(e)
    pkts: list[bytes] = []
    arrivals: list[float] = []
    pcm: list[np.ndarray] = []
    plens: list[int] = []
    texts: list[str] = []
    seqs: list[int] = []
    off_us = None
    live = None
    first_pkt = None
    try:
        ws.send_text(json.dumps({"t": "hello", "proto": PROTO}))
        ws.send_text(json.dumps({"t": "sub"}))
        t_sub = time.time()
        while True:
            op, data = ws.recv(timeout=3)
            now = time.time()
            if op == 0x1:
                t = data.decode("utf-8", "replace")
                texts.append(t)
                m = ws.handle_control(t)
                if m and m.get("t") == "offset":
                    off_us = m.get("us")
                continue
            if first_pkt is None:
                first_pkt = now
            if now - t_sub >= seconds:
                break
            if len(data) <= HDR:
                continue
            hdr_seq = int.from_bytes(data[0:4], "big")
            seqs.append(hdr_seq)
            pkts.append(data[HDR:])
            arrivals.append(now)
            if dec is not None:
                d = dec.decode(data[HDR:])
                pcm.append(d)
                plens.append(int(d.size))
        # 断开之前抓一次端点状态：断开会清空接收环，之后再读就是全 0
        live = debug_net()
    finally:
        if dec is not None:
            dec.close()
        ws.close()
    samples = np.concatenate(pcm) if pcm else np.zeros(0, dtype=np.float32)
    return {
        "packets": pkts,
        "arrivals": arrivals,
        "texts": texts,
        "samples": samples,
        "packet_samples": plens,
        "seqs": seqs,
        "clock_offset_us": off_us,
        "live_net": live,
        "opus_error": opus_err,
        "first_packet_ms": round((first_pkt - t0) * 1000.0, 1) if first_pkt else None,
        "elapsed_s": round(time.time() - t0, 3),
    }


def collect_roundtrip(port: int, seconds: float, opus_path: Path | None) -> dict:
    """上行回环：一个连接推 Opus（模拟手机麦克风），另一个连接订阅收回。"""
    if opus_path is None:
        raise RuntimeError("roundtrip 模式需要 libopus（--opus / PUREVOX_OPUS）")
    t0 = time.time()
    rx = Wss("127.0.0.1", port, tls=True)
    tx = Wss("127.0.0.1", port, tls=True)
    recv_dec = Opus(opus_path)
    tx_enc = OpusEncoder(opus_path)
    stop = threading.Event()
    sent = [0]
    send_err = [""]
    step = 2.0 * math.pi * TONE_HZ / SR
    phase = [0.0]

    def sender() -> None:
        frame = np.empty(HOP, dtype=np.float32)
        nxt = time.perf_counter()
        seq = 0
        try:
            while not stop.is_set():
                for i in range(HOP):
                    frame[i] = TONE_AMP * math.sin(phase[0])
                    phase[0] = (phase[0] + step) % (2.0 * math.pi)
                pkt = tx_enc.encode(frame)
                # 8 字节帧头（seq + 采集时刻 µs）：与服务端同一口径
                ts = int(time.perf_counter() * 1_000_000) & 0xFFFFFFFF
                tx.send_binary(seq.to_bytes(4, "big") + ts.to_bytes(4, "big") + pkt)
                seq += 1
                sent[0] += 1
                nxt += 0.010
                # **细粒度等待**：直接 sleep(剩余) 在 Windows 上会被抬到 15.6 ms 粒度，
                # 于是发送端自己就把 10 ms 序列变成「憋 30 ms 再连发 3 包」——实测到的
                # gap 36~41 ms 就是这么来的，会把链路本身的抖动判得比实际差得多。
                while True:
                    d = nxt - time.perf_counter()
                    if d <= 0:
                        break
                    time.sleep(min(d, 0.001))
                if time.perf_counter() - nxt > 0.2:
                    nxt = time.perf_counter()
        except Exception as e:  # noqa: BLE001
            send_err[0] = f"{type(e).__name__}: {e}"

    _raise_timer()
    try:
        rx.send_text(json.dumps({"t": "hello", "proto": PROTO}))
        rx.send_text(json.dumps({"t": "sub"}))
        tx.send_text(json.dumps({"t": "hello", "proto": PROTO}))
        thread = threading.Thread(target=sender, name="pv-test-tx", daemon=True)
        thread.start()
        pkts: list[bytes] = []
        arrivals: list[float] = []
        pcm: list[np.ndarray] = []
        plens: list[int] = []
        texts: list[str] = []
        off_us = None
        live = None
        first_pkt = None
        while True:
            op, data = rx.recv(timeout=3)
            now = time.time()
            if op == 0x1:
                t = data.decode("utf-8", "replace")
                texts.append(t)
                m = rx.handle_control(t)
                if m and m.get("t") == "offset":
                    off_us = m.get("us")
                continue
            if first_pkt is None:
                first_pkt = now
            if now - t0 >= seconds:
                break
            if len(data) <= HDR:
                continue
            pkts.append(data[HDR:])
            arrivals.append(now)
            d = recv_dec.decode(data[HDR:])
            pcm.append(d)
            plens.append(int(d.size))
        live = debug_net()
    finally:
        _lower_timer()
        stop.set()
        try:
            if "thread" in locals():
                thread.join(1.0)
        except Exception:  # noqa: BLE001
            pass
        recv_dec.close()
        tx_enc.close()
        rx.close()
        tx.close()
    samples = np.concatenate(pcm) if pcm else np.zeros(0, dtype=np.float32)
    return {
        "packets": pkts,
        "arrivals": arrivals,
        "texts": texts,
        "samples": samples,
        "packet_samples": plens,
        "clock_offset_us": off_us,
        "live_net": live,
        "opus_error": "",
        "first_packet_ms": round((first_pkt - t0) * 1000.0, 1) if first_pkt else None,
        "elapsed_s": round(time.time() - t0, 3),
        "sent_packets": sent[0],
        "send_error": send_err[0],
    }


def db(x: float) -> float:
    return 20.0 * math.log10(max(float(x), 1e-12))


def band_pct(freqs: np.ndarray, power: np.ndarray, lo: float, hi: float, total: float) -> float:
    m = (freqs >= lo) & (freqs < hi)
    return 100.0 * float(power[m].sum()) / max(total, 1e-30)


def schedule_sim(samples: np.ndarray, packet_samples: list[int], arrivals: list[float]) -> dict | None:
    """复现网页版 `playPacket` 的调度：每包 `max(playAt, now+20ms)` 起播。

    返回把各包按调度时刻拼进时间轴后的结果（空隙 = 浏览器实际听到的静音）。
    这能区分「服务端流本身坏了」和「客户端调度把好流拼坏了」。
    """
    if not packet_samples or len(arrivals) != len(packet_samples) or not arrivals:
        return None
    t0 = arrivals[0]
    play = t0 + 0.02
    starts: list[float] = []
    gaps: list[float] = []
    pos = 0
    for a, n in zip(arrivals, packet_samples):
        at = max(play, a + 0.02)
        if at > play + 1e-9:
            gaps.append(at - play)
        starts.append(at)
        play = at + n / SR
        pos += n
    end = play
    total = int(round((end - starts[0]) * SR)) + 1
    timeline = np.zeros(total, dtype=np.float32)
    pos = 0
    for at, n in zip(starts, packet_samples):
        i = int(round((at - starts[0]) * SR))
        seg = samples[pos:pos + n]
        pos += n
        timeline[i:i + seg.size] += seg[: max(0, total - i)]
    out = {
        "packets": len(starts),
        "gap_count": len(gaps),
        "gap_ms_total": round(sum(gaps) * 1000.0, 3),
        "gap_ms_max": round(max(gaps) * 1000.0, 3) if gaps else 0.0,
        "lead_ms_max": round(max((starts[i] + packet_samples[i] / SR) - arrivals[i]
                                 for i in range(len(starts))) * 1000.0, 3),
        "lead_ms_final": round((end - arrivals[-1]) * 1000.0, 3),
    }
    # 时间轴上重新做一次「1 kHz 是否还连续」的判断（跳过 200 ms 起步）
    skip = min(timeline.size, SR // 5)
    y = timeline[skip:]
    out["rms_dbfs"] = round(db(float(np.sqrt(np.mean(y * y)))), 2)
    out["peak_dbfs"] = round(db(float(np.max(np.abs(y)))), 2)
    d = np.abs(np.diff(y))
    out["max_step"] = round(float(d.max()), 6) if d.size else 0.0
    out["steps_gt_3x"] = int(np.sum(d > 3 * TONE_THEORY)) if d.size else 0
    z = np.abs(y) < 1e-7
    runs, cur = [], 0
    for v in z:
        if v:
            cur += 1
        elif cur:
            runs.append(cur)
            cur = 0
    if cur:
        runs.append(cur)
    out["silence_runs_gt_1ms"] = int(sum(1 for r in runs if r > SR // 1000))
    out["longest_silence_ms"] = round(max(runs) / SR * 1000.0, 2) if runs else 0.0
    n = min(y.size, SR)
    n = 1 << int(math.log2(n)) if n >= 1024 else y.size
    if n >= 1024:
        seg = y[:n] * np.hanning(n)
        power = np.abs(np.fft.rfft(seg)) ** 2
        freqs = np.fft.rfftfreq(n, 1.0 / SR)
        total_p = float(power.sum())
        out["peak_hz"] = round(float(freqs[int(np.argmax(power[1:])) + 1]), 3)
        out["tone_pct"] = round(band_pct(freqs, power, 990, 1010, total_p), 3)
    return out


def analyze(samples: np.ndarray, packet_samples: list[int], arrivals: list[float],
            packets: list[bytes]) -> dict:
    out: dict = {
        "samples": int(samples.size),
        "duration_s": round(samples.size / SR, 3),
        "packets": len(packets) if packets else None,
        "packet_samples": None,
        "packet_bytes": None,
        "arrival_ms": None,
        "packet_rate_hz": None,
    }
    if packets:
        lens = [len(p) for p in packets]
        out["packet_bytes"] = {"min": min(lens), "max": max(lens),
                               "mean": round(sum(lens) / len(lens), 2)}
        if packet_samples:
            out["packet_samples"] = {"min": min(packet_samples), "max": max(packet_samples),
                                     "mean": round(sum(packet_samples) / len(packet_samples), 2)}
        if len(arrivals) >= 2:
            dl = np.diff(np.asarray(arrivals, dtype=np.float64))
            out["arrival_ms"] = {
                "mean": round(float(dl.mean()) * 1000.0, 3),
                "p50": round(float(np.percentile(dl, 50)) * 1000.0, 3),
                "p95": round(float(np.percentile(dl, 95)) * 1000.0, 3),
                "min": round(float(dl.min()) * 1000.0, 3),
                "max": round(float(dl.max()) * 1000.0, 3),
            }
            span = arrivals[-1] - arrivals[0]
            if span > 0:
                out["packet_rate_hz"] = round((len(arrivals) - 1) / span, 2)
    if samples.size == 0:
        return out

    # 跳过**起播瞬态**：流刚开始时接收环还没填满（服务端在补 PLC、客户端在预缓冲），
    # 那几百毫秒的样本本来就不是稳态。以前只跳 0.2 s，频谱窗口正好压在瞬态上，
    # 于是把正常的起播判成「频率错 / 99% 噪声」（2026-10-27 用二分法定位）。
    skip = min(samples.size, int(SR * max(1.5, min(3.0, samples.size / SR * 0.2))))
    x = samples[skip:]
    out["skip_ms"] = round(skip / SR * 1000.0, 1)
    out["rms_dbfs"] = round(db(float(np.sqrt(np.mean(x * x)))), 2)
    out["peak_dbfs"] = round(db(float(np.max(np.abs(x)))), 2)
    out["dc"] = round(float(np.mean(x)), 7)

    # 1 s 窗做频谱（足够分辨 1 kHz 与谐波；不引 scipy）
    n = min(x.size, SR)
    n = 1 << int(math.log2(n)) if n >= 1024 else x.size
    if n >= 1024:
        seg = x[:n] * np.hanning(n)
        power = np.abs(np.fft.rfft(seg)) ** 2
        freqs = np.fft.rfftfreq(n, 1.0 / SR)
        total = float(power.sum())
        lo = int(np.searchsorted(freqs, 20.0))
        hi = int(np.searchsorted(freqs, 20000.0))
        if hi > lo:
            k = lo + int(np.argmax(power[lo:hi]))
            out["peak_hz"] = round(float(freqs[k]), 3)
        out["tone_pct"] = round(band_pct(freqs, power, 990, 1010, total), 3)
        out["harm2_pct"] = round(band_pct(freqs, power, 1980, 2020, total), 4)
        out["harm3_pct"] = round(band_pct(freqs, power, 2970, 3030, total), 4)
        out["noise_pct"] = round(100.0 - out["tone_pct"], 3)

    # 断点：1 kHz @ -20 dBFS 的相邻样点最大步进是确定的，明显超出就是相位跳变 / 拼接 / 削波
    d = np.abs(np.diff(x))
    out["max_step"] = round(float(d.max()), 6) if d.size else 0.0
    out["max_step_theory"] = round(TONE_THEORY, 6)
    out["steps_gt_3x"] = int(np.sum(d > 3 * TONE_THEORY)) if d.size else 0

    # 每包 RMS / 静音
    if packet_samples:
        pos = skip
        prms: list[float] = []
        for n in packet_samples:
            seg = samples[pos:pos + n]
            pos += n
            if seg.size:
                prms.append(db(float(np.sqrt(np.mean(seg * seg)))))
        if prms:
            out["packet_rms_dbfs"] = {"min": round(min(prms), 2), "max": round(max(prms), 2),
                                      "mean": round(float(np.mean(prms)), 2)}
            out["silent_packets"] = int(sum(1 for v in prms if v < -100))

    z = np.abs(x) < 1e-7
    runs: list[int] = []
    cur = 0
    for v in z:
        if v:
            cur += 1
        elif cur:
            runs.append(cur)
            cur = 0
    if cur:
        runs.append(cur)
    out["silence_runs_gt_1ms"] = int(sum(1 for r in runs if r > SR // 1000))
    out["longest_silence_ms"] = round(max(runs) / SR * 1000.0, 2) if runs else 0.0
    if arrivals and packet_samples:
        out["schedule"] = schedule_sim(samples, packet_samples, arrivals)
        out["arrivals_ms"] = [round((a - arrivals[0]) * 1000.0, 2) for a in arrivals]
    return out


def verdict(stream: dict, decode_ok: bool, reuse: bool) -> tuple[bool, str, str, str]:
    """返回 (ok, code, 人话结论, 下一步建议)。"""
    if stream.get("send_error"):
        return False, "send_error", f"上行发送失败：{stream['send_error']}", \
            "看 --mode roundtrip 的发送线程；确认端口 / 连接没被服务端断开"
    if stream.get("sent_packets") == 0:
        return False, "no_send", "一个上行包都没发出去", "检查 libopus 编码器与发送连接"
    if stream.get("packets") is None:
        hint = "上行包发出去了但没回来" if stream.get("sent_packets") else "没有收到任何音频包"
        return False, "no_stream", f"{hint}（对方没订阅 / 没有网络扬声器行 / 服务未起）", \
            "确认计划里有 remote_mic / remote_speaker 行、服务已启动，客户端已发 {\"t\":\"sub\"}"
    if not decode_ok:
        return False, "no_decoder", "收到了包，但没有可用的 libopus，无法解码判断音质", \
            "安装 libopus（Windows 随包 opus.dll；Linux libopus0）或用 --opus 指定"
    rate = stream.get("packet_rate_hz")
    if rate is None or abs(rate - 100.0) > 5.0:
        return False, "packet_rate", f"包节奏不对：{rate} 包/s（应 ≈100）", \
            "看出站定时器是否被阻塞；/debug 的 net.tx_packets_per_s 应 ≈100"
    samples = stream.get("packet_samples") or {}
    if samples and (samples.get("min") != HOP or samples.get("max") != HOP):
        return False, "packet_size", f"每包解码样本数不是 480：min={samples.get('min')} max={samples.get('max')}", \
            "客户端/服务端的 Opus 帧长或解码帧长不一致"
    peak = stream.get("peak_hz")
    if peak is None or abs(peak - TONE_HZ) > 5.0:
        return False, "wrong_freq", f"主频不是 1 kHz：{peak} Hz", \
            "检查输入是不是测试音、是否有重采样比例被拉到 ±3% 之外"
    tone = stream.get("tone_pct") or 0.0
    silent = stream.get("silent_packets") or 0
    longest = stream.get("longest_silence_ms") or 0.0
    steps = stream.get("steps_gt_3x") or 0
    if silent > 0 or longest >= 5.0:
        return False, "gaps", f"有 {silent} 个静音包 / 最长静音 {longest} ms", \
            "看 /debug 的 net.tx_subs.level_ms 与 tx_dropped；客户端播放调度出现欠载"
    if tone < 80.0:
        return False, "distorted", f"1 kHz 只占 {tone:.2f}% 能量（谐波/噪声过高）", \
            "先排除 Opus 比特率过低或客户端解码/重采样问题；对比收到的 WAV 与源测试音"
    if steps > 20:
        return False, "click", f"检测到 {steps} 个明显断点（最大步进 {stream.get('max_step')}）", \
            "这是拼接/丢包/播放调度欠载；看 net 的 tx_dropped 与客户端缓冲"
    return True, "ok", f"1 kHz 正常：{peak:.1f} Hz，{tone:.2f}% 能量，RMS {stream.get('rms_dbfs')} dBFS", ""


def write_outputs(out: Path | None, samples: np.ndarray, rate: int = SR) -> dict:
    if out is None:
        return {}
    out.parent.mkdir(parents=True, exist_ok=True)
    if out.suffix.lower() != ".wav":
        out = out.with_suffix(".wav")
    pcm = np.clip(samples, -1.0, 1.0)
    with wave.open(str(out), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes((pcm * 32767.0).astype("<i2").tobytes())
    f32 = out.with_suffix(".f32")
    samples.astype("<f4").tofile(str(f32))
    return {"wav": str(out), "f32": str(f32)}


def load_dump(path: Path, rate: int) -> np.ndarray:
    if path.suffix.lower() == ".wav":
        with wave.open(str(path), "rb") as w:
            ch, sw, sr, n = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
            raw = w.readframes(n)
        if sr != rate:
            raise SystemExit(f"dump 采样率 {sr} != {rate}（用 --rate 指定）")
        if ch != 1:
            a = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
            a = a.reshape(-1, ch).mean(axis=1)
        elif sw == 2:
            a = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
        elif sw == 4:
            a = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
        else:
            raise SystemExit(f"不支持的 WAV 位深：{sw * 8}-bit")
        return a
    return np.fromfile(str(path), dtype="<f4")


# ---------------------------------------------------------------- CLI


def list_resources(osn: str) -> None:
    exe = default_exe(osn)
    models = default_models()
    print(f"platform : {osn} ({_platform.platform()})")
    print(f"exe      : {exe or '（没找到；用 --exe）'}")
    print(f"models   : {models or '（没找到；用 --models）'}")
    print("opus     :")
    for p in opus_candidates(osn):
        if p.is_file() or p.name.startswith("libopus"):
            print(f"  - {p}")
    print(f"repo     : {REPO}")


def main() -> int:
    ap = argparse.ArgumentParser(description="PureVox 网络下行回路测试（tone → remote_speaker）")
    ap.add_argument("--platform", default="auto", choices=["auto", "windows", "linux", "macos", "other"])
    ap.add_argument("--exe", type=Path, help="PureVox 可执行文件")
    ap.add_argument("--models", type=Path, help="模型目录（PUREVOX_MODEL_DIR）")
    ap.add_argument("--opus", type=Path, help="libopus 路径（默认自动找 opus.dll/libopus.so.0）")
    ap.add_argument("--port", type=int, default=0, help="网络端口（0 = 自动选空闲端口）")
    ap.add_argument("--seconds", type=float, default=10.0, help="采集秒数")
    ap.add_argument("--mode", default="speaker", choices=["speaker", "roundtrip"],
                    help="speaker=测试音→网络扬声器（默认）；roundtrip=网络麦克风→网络扬声器（上下行都走一遍）")
    ap.add_argument("--home", type=Path, help="隔离 HOME（默认临时目录；测试完保留）")
    ap.add_argument("--reuse", action="store_true", help="不启动应用，直接连 --port 上已有的实例")
    ap.add_argument("--from-dump", type=Path, help="离线分析已有 WAV / .f32，不启动应用")
    ap.add_argument("--rate", type=int, default=SR, help="--from-dump 的采样率")
    ap.add_argument("--json", type=Path, help="结构化结果写入路径（失败也写）")
    ap.add_argument("--out", type=Path, help="收到的音频写成 WAV（默认临时目录下 received.wav）")
    ap.add_argument("--list", action="store_true", help="列出候选 exe / opus / 模型后退出")
    ap.add_argument("--keep", action="store_true", help="保留临时 HOME 与应用日志")
    args = ap.parse_args()

    osn = os_name(args.platform)
    if args.list:
        list_resources(osn)
        return 0

    result: dict = {
        "schema_version": SCHEMA,
        "created_ms": int(time.time() * 1000),
        "ok": False,
        "verdict_code": "error",
        "verdict_text": "",
        "suggest": "",
        "platform": {"os": osn, "python": sys.version.split()[0], "machine": _platform.machine()},
        "input": {"seconds": args.seconds, "port": args.port, "reuse": args.reuse,
                  "mode": args.mode},
        "stream": {},
        "dump": {},
    }
    proc: subprocess.Popen | None = None
    home = args.home or (DEFAULT_HOME / time.strftime("%Y%m%d-%H%M%S"))
    log_path = home / "app.log"
    try:
        if args.from_dump:
            samples = load_dump(args.from_dump, args.rate)
            stream = analyze(samples, [], [], [])
            ok, code, text, suggest = verdict(stream, True, True)
            result.update(ok=ok, verdict_code=code, verdict_text=text, suggest=suggest,
                          stream=stream, input={"from_dump": str(args.from_dump), "rate": args.rate})
            out = args.out or (home / "received.wav")
            # from-dump 不重写输入本身
            if args.out:
                result["dump"] = write_outputs(args.out, samples, args.rate)
        else:
            port = args.port or free_port()
            result["input"]["port"] = port
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
            if args.reuse:
                h = health(port, ctx)
                result["health"] = h
            else:
                exe = args.exe or default_exe(osn)
                if exe is None:
                    raise SystemExit("找不到 PureVox 可执行文件，用 --exe 指定")
                models = args.models or default_models()
                home.mkdir(parents=True, exist_ok=True)
                write_config(home, port, PLAN_ROUNDTRIP if args.mode == "roundtrip" else PLAN)
                if not args.keep:
                    result["home"] = str(home)
                proc = launch(exe, home, models, log_path)
                h = wait_health(proc, port, ctx, log_path)
                result["health"] = h
                result["platform"]["exe"] = str(exe)
            opus = args.opus
            if opus is None:
                for cand in opus_candidates(osn):
                    try:
                        if cand.is_file():
                            opus = cand
                            break
                    except OSError:
                        continue
            result["opus"] = str(opus) if opus else None
            got = (collect_roundtrip(port, args.seconds, opus) if args.mode == "roundtrip"
                   else collect(port, args.seconds, opus))
            stream = analyze(got["samples"], got["packet_samples"], got["arrivals"], got["packets"])
            stream["first_packet_ms"] = got["first_packet_ms"]
            stream["texts"] = got["texts"]
            # 客户端与服务端的时钟偏移（`sync`/`syncreply` 估出来的）：单向延迟靠它
            stream["clock_offset_us"] = got.get("clock_offset_us")
            stream["opus_error"] = got["opus_error"]
            if args.mode == "roundtrip":
                stream["sent_packets"] = got.get("sent_packets")
                stream["send_error"] = got.get("send_error")
            decode_ok = got["samples"].size > 0 and not got["opus_error"]
            ok, code, text, suggest = verdict(stream, decode_ok, args.reuse)
            try:
                result["server"] = health(port, ctx)
            except Exception as e:  # noqa: BLE001
                result["server"] = {"error": str(e)}
            # 端点全量计数（App 自己量的那一份）；音频指标是脚本量的，两边都落盘
            result["endpoint"] = {"net": got.get("live_net") or debug_net()}
            srv = result.get("server") or {}
            # 只看**出站**：`rx_dropped` 现在是「环溢出裁剪的样本数」，启动时裁一格是
            # 设计行为（DESIGN.md §4.1.4），不该判失败；真正要红的是「发不出去」。
            tx_drops = int(srv.get("tx_dropped") or 0)
            if ok and tx_drops > 0:
                ok, code = False, "dropped"
                text = f"服务端出站丢 hop：tx_dropped={tx_drops}"
                suggest = "看 /debug 的 net.tx_subs；发送任务被调度慢或有客户端不取数据"
            out = args.out or (home / "received.wav")
            if got["samples"].size:
                result["dump"] = write_outputs(out, got["samples"])
            elif args.out:
                result["dump"] = {}
            result.update(ok=ok, verdict_code=code, verdict_text=text, suggest=suggest,
                          stream=stream, input={**result["input"],
                                                "exe": result["platform"].get("exe"),
                                                "home": str(home)})
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001 - 工具入口：一定写结构化失败
        result.update(ok=False, verdict_code="error", verdict_text=f"{type(e).__name__}: {e}",
                      suggest="看 app.log / health / 端口占用；必要时 --reuse 连现有实例")
        result["error"] = repr(e)
    finally:
        if proc is not None:
            terminate(proc)
        if not args.keep and proc is not None and args.home is None:
            # 日志留一份最小的，方便 AI 复核；这是工具产物不是用户数据
            pass
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        elif args.from_dump:
            pass

    print(f"{'OK' if result['ok'] else 'FAIL'}  {result['verdict_code']}: {result['verdict_text']}")
    if result.get("suggest"):
        print(f"下一步：{result['suggest']}")
    s = result.get("stream") or {}
    if s:
        keys = ["duration_s", "packets", "packet_rate_hz", "rms_dbfs", "peak_dbfs", "peak_hz",
                "tone_pct", "harm2_pct", "harm3_pct", "noise_pct", "max_step", "steps_gt_3x",
                "silent_packets", "longest_silence_ms"]
        print("  " + "  ".join(f"{k}={s.get(k)}" for k in keys if s.get(k) is not None))
    if result.get("dump"):
        print("  dump=" + ", ".join(f"{k}:{v}" for k, v in result["dump"].items()))
    if args.json:
        print(f"  json={args.json}")
    return 0 if result["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
