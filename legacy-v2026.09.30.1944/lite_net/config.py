# PureVox Lite Net Only — 独立配置
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later

import json
import os

LITE_CONFIG_DIR = os.path.join(os.path.expanduser("~"), ".purevox")
LITE_CONFIG_PATH = os.path.join(LITE_CONFIG_DIR, "lite_net_only.json")

DEFAULTS = {
    "output_device": "",
    "port": 8765,
    # 选中的广播网卡 IP（多网卡下拉）
    "net_ip": "",
    "pre_gain_db": 0,
    "post_gain_db": 0,
    "autostart": False,
    "zoom": 100,
    # True = 按屏幕分辨率自动定挡（忽略 zoom）；手动选过百分比后为 False
    "auto_zoom": True,
}

def _load_json_compat(path):
    # 兼容 GBK/UTF-8/UTF-8-SIG，配置与 UI 显示均不乱码
    for enc in ("utf-8", "utf-8-sig", "gbk", "gb18030"):
        try:
            with open(path, "r", encoding=enc) as f:
                return json.load(f)
        except UnicodeDecodeError:
            continue
        except Exception:
            break
    return None

def load():
    data = None
    try:
        data = _load_json_compat(LITE_CONFIG_PATH)
    except Exception:
        data = None
    if data is None:
        return dict(DEFAULTS)
    out = dict(DEFAULTS)
    for k in DEFAULTS:
        if k in data:
            out[k] = data[k]
    # clamp gains int -20~30, zoom 75~200, port 1024~65535
    try:
        out["pre_gain_db"] = max(-20, min(30, int(float(out["pre_gain_db"]))))
        out["post_gain_db"] = max(-20, min(30, int(float(out["post_gain_db"]))))
        out["zoom"] = max(75, min(200, int(float(out.get("zoom", 100)))))
        out["port"] = max(1024, min(65535, int(float(out.get("port", 8765)))))
    except Exception:
        out["pre_gain_db"] = 0
        out["post_gain_db"] = 0
        out["zoom"] = 100
        out["port"] = 8765
    return out

def save(cfg):
    try:
        os.makedirs(LITE_CONFIG_DIR, exist_ok=True)
        with open(LITE_CONFIG_PATH, "w", encoding="utf-8") as f:
            json.dump(cfg, f, ensure_ascii=False, indent=2)
    except Exception:
        pass
