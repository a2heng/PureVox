"""抓「突然爆增丢包」的现场：电脑 10 Hz + 手机 1 Hz，事件前后打时间线。

用法：
    python tools/automation/net_watch.py            # 90 s
    python tools/automation/net_watch.py 180        # 180 s

只读，不改任何状态。退出码 0 = 没抓到可疑事件，1 = 抓到了（方便挂到脚本里）。
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
import urllib.request

DEBUG = "http://127.0.0.1:47821/debug"
ADB = r"D:/Android/Sdk/platform-tools/adb.exe"
DEV = "UKTW9TK7I7CM8X4P"

PC_KEYS = (
    "tx_packets",
    "tx_dropped",
    "src_gap_ms",
    "src_min_gap_ms",
    "src_max_gap_ms",
    "src_bursts",
    "src_holes",
    "tx_gap_ms",
    "tx_min_gap_ms",
    "tx_max_gap_ms",
    "tx_bursts",
    "tx_holes",
    "rtt_ms",
)
PHONE_KEYS = (
    "packets",
    "hops",
    "late",
    "trims",
    "underruns",
    "gap_over_ring",
    "spliced_hops",
    "level_ms",
    "target_ms",
    "device_underruns",
    "clock_ppm",
    "one_way_ms",
)


def pc() -> dict | None:
    try:
        with urllib.request.urlopen(DEBUG, timeout=2) as r:
            d = json.load(r)
        n = d.get("net") or {}
        c = n.get("client") or {}
        out = {k: n.get(k) for k in PC_KEYS}
        # 手机上报的那一份（1 Hz 刷新在电脑侧）
        out["client_age_ms"] = (time.time() * 1000 - c.get("at_ms", 0)) if c.get("at_ms") else None
        out["client_level_ms"] = c.get("level_ms")
        out["clients"] = n.get("clients")
        return out
    except Exception:
        return None


def phone() -> dict | None:
    """读手机最新一行诊断 JSON（1 Hz）。"""
    try:
        p = subprocess.run([ADB, "-s", DEV, "logcat", "-d", "-t", "40"],
                           capture_output=True, timeout=6)
        lines = [l for l in p.stdout.decode("utf-8", "replace").splitlines()
                 if "PureVoxDiag" in l]
        if not lines:
            return None
        s = lines[-1]
        d = json.loads(s[s.find("{"):])
        out = {k: d["downlink"].get(k) for k in PHONE_KEYS}
        out["ts_ms"] = d.get("ts_ms")
        out["rtt_ms"] = d["conn"].get("rtt_ms")
        out["buffer_ms"] = d["downlink"].get("buffer_ms")
        return out
    except Exception:
        return None


def main() -> int:
    secs = float(sys.argv[1]) if len(sys.argv) > 1 else 90.0
    t0 = time.monotonic()
    pc_hist: list[tuple[float, dict]] = []
    ph_hist: list[tuple[float, dict]] = []
    prev_pc = None
    prev_ph = None
    events: list[tuple[float, str, dict]] = []
    last_phone_poll = 0.0

    while time.monotonic() - t0 < secs:
        now = time.monotonic() - t0
        a = pc()
        if a:
            pc_hist.append((now, a))
            if prev_pc:
                # 事件：源侧/发送侧出现连发或空档，或丢包计数跳变
                d_burst = (a.get("src_bursts") or 0) - (prev_pc.get("src_bursts") or 0)
                d_hole = (a.get("src_holes") or 0) - (prev_pc.get("src_holes") or 0)
                d_drop = (a.get("tx_dropped") or 0) - (prev_pc.get("tx_dropped") or 0)
                if d_burst or d_hole or d_drop:
                    events.append((now, "PC", {
                        "d_burst": d_burst, "d_hole": d_hole, "d_drop": d_drop,
                        "src_max": a.get("src_max_gap_ms"), "tx_max": a.get("tx_max_gap_ms"),
                    }))
            prev_pc = a
        if time.monotonic() - last_phone_poll > 1.0:
            last_phone_poll = time.monotonic()
            b = phone()
            if b:
                ph_hist.append((now, b))
                if prev_ph and b.get("ts_ms") and prev_ph.get("ts_ms"):
                    dt = (b["ts_ms"] - prev_ph["ts_ms"]) / 1000.0
                    jumps = {}
                    for k in ("late", "trims", "underruns", "gap_over_ring", "spliced_hops"):
                        d = (b.get(k) or 0) - (prev_ph.get(k) or 0)
                        if d:
                            jumps[k] = d
                    if jumps:
                        events.append((now, "PHONE", {"dt": round(dt, 1), **jumps,
                                                      "level": b.get("level_ms")}))
                prev_ph = b
        time.sleep(0.1)

    print(f"采集 {len(pc_hist)} 个电脑样本 / {len(ph_hist)} 个手机样本，事件 {len(events)} 个")
    if events:
        for t, side, info in events[:40]:
            print(f"  t={t:6.1f}s {side:5s} {info}")
    else:
        print("  没有抓到事件")

    # 汇总：两端的关键分布
    if pc_hist:
        last = pc_hist[-1][1]
        print("\n电脑侧（末尾）:")
        for k in ("src_gap_ms", "src_min_gap_ms", "src_max_gap_ms", "src_bursts", "src_holes",
                  "tx_gap_ms", "tx_min_gap_ms", "tx_max_gap_ms", "tx_bursts", "tx_holes",
                  "tx_dropped", "client_age_ms"):
            print(f"  {k:16s} {last.get(k)}")
    if ph_hist:
        last = ph_hist[-1][1]
        print("手机侧（末尾）:")
        for k in PHONE_KEYS:
            print(f"  {k:16s} {last.get(k)}")
    return 1 if events else 0


if __name__ == "__main__":
    sys.exit(main())
