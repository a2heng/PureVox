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

"""PureVox Web 打包（唯一构建入口）。

把 Lite 两个最简档移植成浏览器页面，产出可直接部署到 GitHub Pages 的**瘦**产物：

  dist/index.html               入口（选 flavor）
  dist/mic.html                 lite_mic：麦克风 → 降噪 → 本地输出
  dist/net.html                 lite_net：网络输入（WebRTC）→ 降噪 → 本地输出
  dist/assets/ort/*.mjs|*.wasm  ONNX Runtime Web 运行时（重资源，两个页面共用一份）
  dist/assets/models/*.onnx     降噪模型（重资源，两个页面共用一份）
  dist/build.json               产物清单（版本 / 字节数 / sha256）

设计要点：

- **页面本体只留应用代码**（CSS / JS / AudioWorklet / Worker 源码，约 60KB）。
  重资源不内联：base64 内联凭空多 33% 体积，且每次改页面都要重传 20MB。
- **ORT 与模型各只存一份，mic / net 共用**：页面按 URL 引用，谁先加载谁填缓存，
  另一页直接命中缓存，不会重复下载。
- **URL 稳定不带版本查询串**：内容指纹放在清单 build.json 里，不塞进 URL。
  这样重资源能被浏览器 / CDN 按 URL 正常长期缓存（URL 不变 = 缓存不失效）；
  真要换内容就换文件名（模型名自带 ep 编号，天然版本化）或换 --base-url 前缀。
- 换 CDN：--base-url 指定重资源前缀（需带 CORS 头，jsDelivr/unpkg 自带）。

⚠️ 运行必须走安全上下文（浏览器只在安全上下文里给麦克风权限、才允许
WebRTC）：用 purevox-web/serve.py 起本地 HTTPS，或部署到 GitHub Pages。

产物与第三方运行时都不入版本库（见 .gitignore）；入库的只有
src/ workers/ build/ serve.py。

用法：
    python purevox-web/build/build_web.py                  # 默认 b 档（与桌面一致）
    python purevox-web/build/build_web.py --model a        # 小号（更快）
    python purevox-web/build/build_web.py --model c        # 大号（最强）
    python purevox-web/build/build_web.py --base-url https://cdn.example.com/pv/assets
    python purevox-web/build/build_web.py --list-models
    python purevox-web/build/build_web.py --offline        # 缺运行时直接报错，不联网

产物运行：
    python purevox-web/serve.py --open     # 本地 HTTPS，打印本机与局域网地址
"""

import argparse
import base64
import hashlib
import io
import json
import os
import re
import shutil
import sys
import tarfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
WEB_DIR = os.path.dirname(HERE)
ROOT = os.path.dirname(WEB_DIR)
SRC_DIR = os.path.join(WEB_DIR, "src")
LOCK_PATH = os.path.join(HERE, "ort.lock.json")
DEFAULT_VENDOR = os.path.join(WEB_DIR, "vendor")
DEFAULT_OUT = os.path.join(WEB_DIR, "dist")
DEFAULT_BASE_URL = "assets"       # 相对路径；同源部署即 Pages 自己

# 共用 JS 段（拼接顺序即依赖顺序：util → ui → ort → pipeline → session）
COMMON_JS = [
    "js/00-util.js",
    "js/10-ui.js",
    "js/20-ort.js",
    "js/30-pipeline.js",
    "js/35-session.js",
]

# 两个 flavor：模板占位符 + 各自的装配脚本
FLAVORS = {
    "mic": {
        "app_js": "js/40-mic-app.js",
        "out": "mic.html",
        "title": "PureVox Web · 麦克风降噪",
        "subtitle": "Lite Mic · 浏览器版",
        "idle_status": "未启动",
        "run_text": "启动",
    },
    "net": {
        "app_js": "js/40-net-app.js",
        "out": "net.html",
        "title": "PureVox Web · 网络降噪",
        "subtitle": "Lite Net · 浏览器版",
        "idle_status": "未连接",
        "run_text": "连接",
    },
}

# vendor 文件 → dist 里的子目录（ort 与 model 各一份，两个页面共用）
VENDOR_ASSETS = {
    "ortGlueUrl": "ort/ort.wasm.bundle.min.mjs",
    "ortWasmUrl": "ort/ort-wasm-simd-threaded.wasm",
}
# 内联（base64）的源码：体积小到不值得多发请求，且能避开 blob/CORS 麻烦
SOURCE_ASSETS = {
    "workerOrtB64": os.path.join("workers", "ort-worker.js"),
    "workerCaptureB64": os.path.join("workers", "capture-worklet.js"),
    "workerPlaybackB64": os.path.join("workers", "playback-worklet.js"),
}


def log(msg):
    print(msg, flush=True)


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def b64_of(path):
    with open(path, "rb") as f:
        return base64.b64encode(f.read()).decode("ascii")


def read_text(path):
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def load_lock():
    with open(LOCK_PATH, "r", encoding="utf-8") as f:
        return json.load(f)


def integrity_ok(data, integrity):
    algo, _, want = integrity.partition("-")
    if not want:
        return False
    got = base64.b64encode(hashlib.new(algo, data).digest()).decode("ascii")
    return got == want


def ensure_vendor(lock, vendor_dir, offline):
    """确保 vendor 下有锁定的两个 ORT 运行时文件；缺则按锁下载并校验。"""
    target = os.path.join(vendor_dir, "onnxruntime-web-%s" % lock["version"])
    wanted = [os.path.basename(p) for p in lock["files"]]
    if all(os.path.isfile(os.path.join(target, n)) for n in wanted):
        log("运行时：复用 %s" % target)
        return target

    os.makedirs(target, exist_ok=True)
    if offline:
        raise SystemExit("缺少 ONNX 运行时且指定了 --offline：%s" % target)
    log("运行时：下载 %s（%s）" % (lock["package"], lock["version"]))
    with urllib.request.urlopen(lock["tarball"], timeout=300) as resp:
        data = resp.read()
    if not integrity_ok(data, lock["integrity"]):
        raise SystemExit("tarball 校验失败（integrity 不符），已中止")
    log("运行时：tarball 校验通过（%s，%.1f MiB）"
        % (lock["integrity"].split("-")[0], len(data) / 1048576.0))
    tf = tarfile.open(fileobj=io.BytesIO(data))
    for rel in lock["files"]:
        member = tf.getmember("package/" + rel)
        dst = os.path.join(target, os.path.basename(rel))
        with open(dst, "wb") as f:
            f.write(tf.extractfile(member).read())
    return target


def resolve_model(key):
    """模型路径与显示名取自仓库根 model_config（不硬编码模型文件名）。"""
    sys.path.insert(0, ROOT)
    from model_config import DENOISE_MODELS, DENOISE_MODEL_DEFAULT  # noqa: E402

    # 允许用末位字母当简写（a / b / c），完整 key 亦可
    aliases = {k[-1]: k for k in DENOISE_MODELS}
    if key in aliases:
        key = aliases[key]
    if not key:
        key = DENOISE_MODEL_DEFAULT
    if key not in DENOISE_MODELS:
        raise SystemExit("未知模型档位 %r，可选：%s"
                         % (key, ", ".join(sorted(DENOISE_MODELS))))
    rel, label = DENOISE_MODELS[key]
    path = os.path.join(ROOT, rel)
    if not os.path.isfile(path):
        raise SystemExit("模型文件缺失：%s" % path)
    return key, label, path, os.path.basename(rel)


def flavor_block(html, flavor):
    """按 <!--FLAVOR:xxx--> ... <!--/FLAVOR:xxx--> 抽取本 flavor 的片段。

    块外的内容（页面骨架）无条件保留；非本 flavor 的块整段丢弃。
    """
    out = []
    keep = True          # 块外默认保留
    for line in html.splitlines(keepends=True):
        m = re.match(r"\s*<!--(/?)FLAVOR:([a-z]+)-->\s*$", line)
        if m:
            if m.group(1) == "/":
                keep = True           # 结束标记：回到块外
            else:
                keep = (m.group(2) == flavor)   # 进入标记：只留本 flavor
            continue
        if keep:
            out.append(line)
    return "".join(out)


def build_js(flavor):
    parts = []
    for rel in COMMON_JS + [FLAVORS[flavor]["app_js"]]:
        src = read_text(os.path.join(SRC_DIR, rel))
        parts.append("/* ==== %s ==== */\n;(function (PV) {\n%s\n})(window.PV = window.PV || {});"
                     % (rel, src.rstrip("\n")))
    return "\n".join(parts)


def build_assets(flavor, base_url, ort_version, model_key, model_label, model_name):
    """注入 PV.ASSETS：重资源给 URL，内联的 worker/worklet 源码给 base64。"""
    fields = []
    for key, rel in VENDOR_ASSETS.items():
        # URL 稳定、不带版本查询串 —— 靠 HTTP 缓存（模型名自带 ep 编号做版本化）
        fields.append("    %s: %s," % (key, json.dumps("%s/%s" % (base_url, rel))))
    fields.append('    modelUrl: %s,' % json.dumps("%s/models/%s" % (base_url, model_name)))
    for key, rel in SOURCE_ASSETS.items():
        raw = read_text(os.path.join(WEB_DIR, rel))
        fields.append('    %s: "%s",'
                      % (key, base64.b64encode(raw.encode("utf-8")).decode("ascii")))
    fields.append("    meta: {")
    fields.append("        flavor: %s," % json.dumps(flavor))
    fields.append("        modelKey: %s," % json.dumps(model_key))
    fields.append("        modelLabel: %s," % json.dumps(model_label, ensure_ascii=False))
    fields.append("        ortVersion: %s," % json.dumps(ort_version))
    fields.append("        sampleRate: 48000, hop: 480,")
    fields.append("    },")
    return ("window.PV = window.PV || {};\n"
            "window.PV.ASSETS = Object.freeze({\n%s\n});\n" % "\n".join(fields))


def build_one(flavor, template, css, base_url, ort_version,
              model_key, model_label, model_name):
    spec = FLAVORS[flavor]
    assets = build_assets(flavor, base_url, ort_version, model_key, model_label, model_name)
    js = build_js(flavor)
    for blob, where in ((css, "CSS"), (assets, "ASSETS"), (js, "JS")):
        if "</script" in blob.lower():
            raise SystemExit("注入的 %s 含 </script>，会截断内联脚本" % where)
    html = flavor_block(template, flavor)
    html = html.replace("{{CSS}}", css)
    html = html.replace("{{ASSETS}}", assets)
    html = html.replace("{{JS}}", js)
    html = html.replace("{{FLAVOR}}", flavor)
    html = html.replace("{{TITLE}}", spec["title"])
    html = html.replace("{{SUBTITLE}}", spec["subtitle"])
    html = html.replace("{{IDLE_STATUS}}", spec["idle_status"])
    html = html.replace("{{RUN_TEXT}}", spec["run_text"])
    if "{{" in html:
        raise SystemExit("模板占位符未替换完：%s" % re.findall(r"\{\{[A-Z_]+\}\}", html))
    return html


def build_index():
    """dist/index.html：极简入口页，列出两个 flavor（纯静态，无依赖）。"""
    rows = []
    for name in ("mic", "net"):
        spec = FLAVORS[name]
        rows.append('    <li><a href="%s">%s</a><span>%s</span></li>'
                    % (spec["out"], spec["title"], spec["subtitle"]))
    return (
        "<!DOCTYPE html>\n"
        '<html lang="zh-CN"><head><meta charset="UTF-8">\n'
        '<meta name="viewport" content="width=device-width, initial-scale=1.0">\n'
        "<title>PureVox Web</title>\n"
        "<style>body{font-family:'Courier New','Noto Sans SC',monospace;"
        "background:#f5f0e8;color:#3a3530;margin:0;padding:32px 16px}"
        "@media(prefers-color-scheme:dark){body{background:#0d0d0d;color:#c0c0c0}}"
        "ul{list-style:none;padding:0;max-width:420px;margin:0 auto}"
        "li{border:1px solid #8884;padding:12px 14px;margin-bottom:10px;border-radius:6px}"
        "a{color:#338855;text-decoration:none;font-size:1rem}"
        "span{display:block;font-size:.7rem;opacity:.7;margin-top:4px}"
        "p{max-width:420px;margin:16px auto;font-size:.72rem;line-height:1.6;opacity:.8}</style>\n"
        "</head><body>\n"
        "  <ul>\n%s\n  </ul>\n"
        "  <p>降噪模型与 ONNX 运行时按需下载并长期缓存（两个页面共用同一份），\n"
        "  页面本身不请求任何第三方服务。麦克风与 WebRTC 需要安全上下文。</p>\n"
        "</body></html>\n" % "\n".join(rows)
    )


def copy_assets(out_dir, vendor_dir, model_path, model_name):
    """重资源落盘到 dist/assets/（一份，供 mic/net 共用）。"""
    placed = []
    ort_dir = os.path.join(out_dir, "assets", "ort")
    os.makedirs(ort_dir, exist_ok=True)
    for rel in VENDOR_ASSETS.values():
        src_name = os.path.basename(rel)
        dst = os.path.join(ort_dir, src_name)
        if not os.path.isfile(dst) or sha256_of(dst) != sha256_of(os.path.join(vendor_dir, src_name)):
            shutil.copyfile(os.path.join(vendor_dir, src_name), dst)
        placed.append(os.path.join("assets", "ort", src_name))
    model_dir = os.path.join(out_dir, "assets", "models")
    os.makedirs(model_dir, exist_ok=True)
    dst = os.path.join(model_dir, model_name)
    if not os.path.isfile(dst) or sha256_of(dst) != sha256_of(model_path):
        shutil.copyfile(model_path, dst)
    placed.append(os.path.join("assets", "models", model_name))
    return placed


def out_label(path, base):
    """产物目录的展示名：仓库内显示相对路径，跨盘/站外直接给绝对路径。"""
    try:
        return os.path.relpath(path, base).replace("\\", "/")
    except ValueError:
        return os.path.abspath(path).replace("\\", "/")


def main():
    ap = argparse.ArgumentParser(description="PureVox Web 打包")
    ap.add_argument("--model", default="", help="降噪档位：a / b / c（或 202609a 等完整 key）")
    ap.add_argument("--out", default=DEFAULT_OUT, help="产物目录（默认 purevox-web/dist）")
    ap.add_argument("--vendor", default=DEFAULT_VENDOR, help="第三方运行时目录（默认 purevox-web/vendor）")
    ap.add_argument("--base-url", default=DEFAULT_BASE_URL,
                    help="重资源 URL 前缀（默认 assets，即同源相对路径；可指向 CDN，需带 CORS）")
    ap.add_argument("--offline", action="store_true", help="不联网：运行时缺失即报错")
    ap.add_argument("--list-models", action="store_true", help="列出可选模型档位后退出")
    args = ap.parse_args()

    if args.list_models:
        sys.path.insert(0, ROOT)
        from model_config import DENOISE_MODELS, DENOISE_MODEL_DEFAULT
        for k in DENOISE_MODELS:
            rel, label = DENOISE_MODELS[k]
            path = os.path.join(ROOT, rel)
            size = os.path.getsize(path) if os.path.isfile(path) else -1
            log("%-8s %-28s %6.1f MiB%s"
                % (k, label, size / 1048576.0, "  （默认）" if k == DENOISE_MODEL_DEFAULT else ""))
        return 0

    lock = load_lock()
    vendor_dir = ensure_vendor(lock, args.vendor, args.offline)
    model_key, model_label, model_path, model_name = resolve_model(args.model)
    base_url = args.base_url.rstrip("/")
    log("模型：%s（%s，%.1f MiB）"
        % (model_key, model_label, os.path.getsize(model_path) / 1048576.0))
    log("重资源前缀：%s（%s）"
        % (base_url, "同源" if not base_url.startswith(("http://", "https://", "//")) else "跨源，需 CORS"))

    template = read_text(os.path.join(SRC_DIR, "app.html"))
    css = read_text(os.path.join(SRC_DIR, "css", "app.css"))

    os.makedirs(args.out, exist_ok=True)
    placed = copy_assets(args.out, vendor_dir, model_path, model_name)

    log("")
    for flavor in ("mic", "net"):
        html = build_one(flavor, template, css, base_url, lock["version"],
                         model_key, model_label, model_name)
        dst = os.path.join(args.out, FLAVORS[flavor]["out"])
        with open(dst, "w", encoding="utf-8", newline="\n") as f:
            f.write(html)
        log("页面：%-12s %7.1f KiB" % (FLAVORS[flavor]["out"],
                                      os.path.getsize(dst) / 1024.0))
    with open(os.path.join(args.out, "index.html"), "w",
              encoding="utf-8", newline="\n") as f:
        f.write(build_index())

    log("")
    total = 0
    for rel in placed:
        full = os.path.join(args.out, rel)
        size = os.path.getsize(full)
        total += size
        log("重资源：%-42s %7.2f MiB  %s"
            % (rel.replace("\\", "/"), size / 1048576.0, "sha256:" + sha256_of(full)[:16]))
    log("        （两页共用，各存一份；URL 稳定 → 浏览器/CDN 可长期缓存）")

    manifest = {
        "flavors": {k: FLAVORS[k]["out"] for k in FLAVORS},
        "model": {"key": model_key, "label": model_label,
                  "file": "assets/models/" + model_name,
                  "bytes": os.path.getsize(model_path), "sha256": sha256_of(model_path)},
        "onnxruntimeWeb": {"version": lock["version"],
                           "glue": "assets/ort/" + os.path.basename(VENDOR_ASSETS["ortGlueUrl"]),
                           "wasm": "assets/ort/" + os.path.basename(VENDOR_ASSETS["ortWasmUrl"])},
        "baseUrl": base_url,
        "assetsBytes": total,
    }
    with open(os.path.join(args.out, "build.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
        f.write("\n")

    log("")
    log("运行（浏览器只在安全上下文给麦克风权限，必须走 HTTPS 或 localhost）：")
    log("    python purevox-web/serve.py --open")
    log("    → 打印本机与局域网地址；首次访问在证书警告里点「继续前往」即可")
    log("部署：把 %s 整个目录传到 GitHub Pages 即可" % out_label(args.out, ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main())
