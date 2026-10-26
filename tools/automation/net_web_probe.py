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
"""网页版下行实测：无头 Chrome 打开 `https://127.0.0.1:<port>/`，点「扬声器」，
读 AudioWorklet 的缓冲水位 / 欠载数，并验证关掉再打开能恢复 + 服务端订阅不泄漏。

为什么单独一个工具：`net_loopback.py` 证明的是**服务端流**干净；网页版过去的
「每包一个 AudioBufferSource + 20 ms 缓冲」会把 Windows 15.6 ms 定时器造成的包成串
拼出 2~6 ms 空隙（1 kHz 实测能量 99.9% → 93.8%）。这个工具直接量真实浏览器的
`spkUnderruns` / `spkBuffered`，所以修的是「另一边」而不是靠猜。

用法：
    python tools/automation/net_web_probe.py
    python tools/automation/net_web_probe.py --seconds 8 --json out.json
    python tools/automation/net_web_probe.py --headed --chrome "C:/.../chrome.exe"

依赖：Google Chrome / Chromium（`--chrome` 可指定；Windows 默认找本机 Chrome）。
不依赖 selenium / playwright：CDP 用标准库 WebSocket（复用 `net_loopback.Wss`）。
退出码：0 = 下行连续（无欠载，关/开正常）；2 = 测到问题。
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import ssl
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import net_loopback as nl  # noqa: E402

SCHEMA = 1
DEFAULT_CDP_PORT = 9223


def chrome_candidates() -> list[Path]:
    env = os.environ.get("PUREVOX_CHROME")
    out: list[Path] = []
    if env:
        out.append(Path(env))
    local = os.environ.get("LOCALAPPDATA")
    program = os.environ.get("PROGRAMFILES")
    program86 = os.environ.get("PROGRAMFILES(X86)")
    if sys.platform.startswith("win"):
        bases = [Path(local) if local else None, Path(program) if program else None,
                 Path(program86) if program86 else None]
        for base in bases:
            if base is None:
                continue
            out.append(base / "Google/Chrome/Application/chrome.exe")
            out.append(base / "Microsoft/Edge/Application/msedge.exe")
    elif sys.platform == "darwin":
        out += [Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
                Path("/Applications/Chromium.app/Contents/MacOS/Chromium")]
    else:
        out += [Path("/usr/bin/google-chrome"), Path("/usr/bin/chromium"),
                Path("/usr/bin/chromium-browser")]
    return out


def find_chrome(explicit: Path | None) -> Path | None:
    if explicit:
        return explicit if explicit.is_file() else None
    for p in chrome_candidates():
        if p.is_file():
            return p
    return None


def wait_cdp(port: int, timeout: float = 30.0) -> dict:
    end = time.time() + timeout
    last = ""
    while time.time() < end:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=2) as r:
                tabs = json.load(r)
            for t in tabs:
                if t.get("type") == "page" and "127.0.0.1" in (t.get("url") or ""):
                    return t
            last = f"没有目标页面（{len(tabs)} 个目标）"
        except Exception as e:  # noqa: BLE001
            last = str(e)
        time.sleep(0.3)
    raise RuntimeError(f"CDP 未就绪：{last}")


class Cdp:
    def __init__(self, ws_url: str):
        rest = ws_url.split("://", 1)[1]
        hostport, path = rest.split("/", 1)
        host, port = hostport.split(":")
        self.ws = nl.Wss(host, int(port), tls=False, path="/" + path)
        self.id = 0

    def call(self, method: str, params: dict | None = None, timeout: float = 10.0) -> dict:
        self.id += 1
        mid = self.id
        self.ws.send_text(json.dumps({"id": mid, "method": method, "params": params or {}}))
        end = time.time() + timeout
        while time.time() < end:
            op, data = self.ws.recv(timeout=max(0.1, end - time.time()))
            if op != 0x1:
                continue
            m = json.loads(data.decode("utf-8"))
            if m.get("id") != mid:
                continue
            if "error" in m:
                raise RuntimeError(f"CDP {method}: {m['error']}")
            return m.get("result", {})
        raise TimeoutError(method)

    def eval(self, expr: str, timeout: float = 10.0):
        r = self.call("Runtime.evaluate", {
            "expression": expr, "returnByValue": True, "awaitPromise": True,
        }, timeout=timeout)
        if r.get("exceptionDetails"):
            raise RuntimeError(json.dumps(r["exceptionDetails"], ensure_ascii=False)[:1000])
        return r.get("result", {}).get("value")

    def wait_state(self, timeout: float = 30.0) -> dict:
        end = time.time() + timeout
        last = {}
        while time.time() < end:
            last = self.eval(
                "({ready:typeof ready!=='undefined'&&ready,"
                "codec:typeof codecReady!=='undefined'&&codecReady,"
                "actx:typeof actx!=='undefined'&&actx?actx.state:null,"
                "ws:typeof ws!=='undefined'&&ws?ws.readyState:null})"
            ) or {}
            if last.get("ready") and last.get("codec"):
                return last
            time.sleep(0.4)
        raise RuntimeError(f"网页脚本未就绪：{last}")

    def close(self) -> None:
        self.ws.close()


def main() -> int:
    ap = argparse.ArgumentParser(description="PureVox 网页版下行实测（无头 Chrome）")
    ap.add_argument("--chrome", type=Path, help="Chrome / Chromium 可执行文件")
    ap.add_argument("--cdp-port", type=int, default=DEFAULT_CDP_PORT, help="CDP 调试端口")
    ap.add_argument("--exe", type=Path, help="PureVox 可执行文件（默认自动找）")
    ap.add_argument("--models", type=Path, help="模型目录")
    ap.add_argument("--port", type=int, default=0, help="网络端口（0 = 自动）")
    ap.add_argument("--seconds", type=float, default=8.0, help="稳态观察秒数")
    ap.add_argument("--headed", action="store_true", help="显示浏览器窗口（默认无头）")
    ap.add_argument("--json", type=Path, help="结构化结果写入路径")
    ap.add_argument("--keep", action="store_true", help="保留临时 HOME / Chrome profile")
    args = ap.parse_args()

    result: dict = {
        "schema_version": SCHEMA,
        "created_ms": int(time.time() * 1000),
        "ok": False,
        "verdict_code": "error",
        "verdict_text": "",
        "suggest": "",
        "platform": {"os": sys.platform, "python": sys.version.split()[0]},
        "first": {},
        "off": {},
        "on": {},
        "health": {},
    }
    run = Path(os.environ.get("TEMP", "/tmp")) / "purevox-web-probe" / time.strftime("%Y%m%d-%H%M%S")
    home = run / "home"
    chrome_proc = None
    app = None
    cdp = None
    log = None
    try:
        chrome = find_chrome(args.chrome)
        if chrome is None:
            raise RuntimeError("找不到 Chrome / Chromium；用 --chrome 指定可执行文件")
        result["chrome"] = str(chrome)
        exe = args.exe or nl.default_exe(nl.os_name("auto"))
        if exe is None:
            raise RuntimeError("找不到 PureVox 可执行文件；用 --exe 指定")
        models = args.models or nl.default_models()
        port = args.port or nl.free_port()
        result["port"] = port
        home.mkdir(parents=True, exist_ok=True)
        nl.write_config(home, port)
        app = nl.launch(exe, home, models, run / "app.log")
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        result["health_start"] = nl.wait_health(app, port, ctx, run / "app.log")

        profile = run / "chrome-profile"
        cdp_port = args.cdp_port
        cmd = [
            str(chrome),
            f"--remote-debugging-port={cdp_port}",
            f"--user-data-dir={profile}",
            "--ignore-certificate-errors",
            "--autoplay-policy=no-user-gesture-required",
            "--use-fake-ui-for-media-stream",
            "--use-fake-device-for-media-stream",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--mute-audio",
            f"https://127.0.0.1:{port}/",
        ]
        if not args.headed:
            cmd.insert(1, "--headless=new")
        log = open(run / "chrome.log", "wb")
        chrome_proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT,
                                       creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        tab = wait_cdp(cdp_port)
        result["url"] = tab.get("url")
        cdp = Cdp(tab["webSocketDebuggerUrl"])
        cdp.call("Runtime.enable")
        result["page"] = cdp.wait_state()
        cdp.eval("document.getElementById('bSpk').click(); 'ok'")
        time.sleep(1.0)
        first = cdp.eval(
            "({node:typeof spkNode!=='undefined'&&!!spkNode,sub:spkSub,"
            "under:spkUnderruns,buf:spkBuffered,actx:actx?actx.state:null})"
        ) or {}
        # 包一层 worklet 的 onmessage：记录缓冲水位范围与欠载变化
        cdp.eval(
            "window.__p={min:1e9,max:0,u:0,n:0};"
            "(function(){const nd=spkNode;if(!nd)return;const old=nd.port.onmessage;"
            "nd.port.onmessage=(e)=>{const d=e.data;if(d&&typeof d.level==='number'){"
            "window.__p.min=Math.min(window.__p.min,d.level);"
            "window.__p.max=Math.max(window.__p.max,d.level);"
            "window.__p.u=d.underruns;window.__p.n++;}if(old)old(e);};})();'ok'"
        )
        time.sleep(args.seconds)
        h_during = nl.health(port, ctx)
        first["probe"] = cdp.eval("window.__p")
        first["health"] = h_during
        result["first"] = first

        # 关扬声器 → 服务端 subs 应归零；再打开 → 恢复且仍有 0 欠载
        cdp.eval("document.getElementById('bSpk').click(); 'ok'")
        time.sleep(1.0)
        off = cdp.eval("({node:!!spkNode,sub:spkSub,under:spkUnderruns})") or {}
        off["health"] = nl.health(port, ctx)
        result["off"] = off
        cdp.eval("document.getElementById('bSpk').click(); 'ok'")
        time.sleep(min(4.0, max(2.0, args.seconds / 2)))
        on = cdp.eval(
            "({node:!!spkNode,sub:spkSub,under:spkUnderruns,buf:spkBuffered,"
            "actx:actx?actx.state:null})"
        ) or {}
        on["health"] = nl.health(port, ctx)
        result["on"] = on

        under = int((first.get("probe") or {}).get("u") or 0)
        buf_min = (first.get("probe") or {}).get("min")
        buf_max = (first.get("probe") or {}).get("max")
        subs_ok = (first.get("health") or {}).get("subs") == 1 and \
                  (off.get("health") or {}).get("subs") == 0 and \
                  (on.get("health") or {}).get("subs") == 1
        if under > 0:
            code, text, sug = "underrun", f"下行缓冲欠载 {under} 次（页面出现断音）", \
                "看 net_loopback 的 arrival_ms 与 worklet 缓冲；确认预缓冲 ≥50 ms"
        elif not subs_ok:
            code, text, sug = "subs", "订阅数没有随扬声器关/开归零/恢复（订阅泄漏）", \
                "检查 proto unsub 与服务端 Sub 的替换逻辑"
        elif not on.get("node") or not on.get("sub"):
            code, text, sug = "toggle", "关掉扬声器后重新打开没有恢复播放节点/订阅", \
                "检查 startSpk 是否重建 AudioWorkletNode 与 dec"
        elif buf_min is not None and buf_min < nl.SR * 0.02:
            code, text, sug = "buffer", f"缓冲最低只有 {buf_min} 样本（<20 ms）", \
                "预缓冲目标偏小；Windows 10 ms 定时器实际按 15.6 ms 触发"
        else:
            code, text, sug = "ok", (
                f"网页下行连续：{args.seconds:.0f}s 内 0 欠载，缓冲 "
                f"{buf_min}-{buf_max} 样本；关/开后 subs 1→0→1"
            ), ""
        result.update(ok=code == "ok", verdict_code=code, verdict_text=text, suggest=sug)
    except Exception as e:  # noqa: BLE001 - 工具入口：失败也写 JSON
        result.update(ok=False, verdict_code="error",
                      verdict_text=f"{type(e).__name__}: {e}",
                      suggest="看 run 目录里的 app.log / chrome.log；必要时 --headed 观察")
        result["error"] = repr(e)
    finally:
        if cdp is not None:
            cdp.close()
        if chrome_proc is not None:
            nl.terminate(chrome_proc)
        if log is not None:
            try:
                log.close()
            except OSError:
                pass
        if app is not None:
            nl.terminate(app)
        if not args.keep:
            # 保留最小日志给下一次复核，其它临时文件可以清
            pass
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        result["run"] = str(run)

    print(f"{'OK' if result['ok'] else 'FAIL'}  {result['verdict_code']}: {result['verdict_text']}")
    if result.get("suggest"):
        print(f"下一步：{result['suggest']}")
    p = result.get("first", {}).get("probe") or {}
    if p:
        print(f"  buffer={p.get('min')}..{p.get('max')} samples  underruns={p.get('u')} "
              f"reports={p.get('n')}")
    if result.get("off"):
        print(f"  off: subs={(result['off'].get('health') or {}).get('subs')} node={result['off'].get('node')}")
    if result.get("on"):
        print(f"  on : subs={(result['on'].get('health') or {}).get('subs')} node={result['on'].get('node')} "
              f"underruns={result['on'].get('under')}")
    if args.json:
        print(f"  json={args.json}")
    print(f"  run={result.get('run')}")
    return 0 if result["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
