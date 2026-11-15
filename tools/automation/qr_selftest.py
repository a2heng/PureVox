#!/usr/bin/env python3
# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# SPDX-License-Identifier: GPL-3.0-or-later
"""自检：`ui/qrcode.js`（内嵌 qrcode-generator v1.4.4, MIT）生成的二维码能否真被扫出来。

为什么要它：手机面板的二维码扫不出来，用户只看到一个方块图，根本想不到是这边编错了；
本机「看起来像二维码」不代表手机扫得出来，肉眼更看不出来。所以这里做**端到端**验证：

    node 跑 ui/qrcode.js 出矩阵 → numpy 拼成位图 → **OpenCV 的 QRCodeDetector 真解码**
    → 断言解出来的文本 == 输入。

    解码器认不出来就一定是编错了，不用去猜是哪一环。用的是完全独立的实现
    （OpenCV 的 ZXing 移植 vs 我们用的 Kazuhiko Arase 库），不共享任何代码。

    曾手写过一版编码器，RS 纠错多项式的升/降幂、格式信息的 MSB 位序都踩过坑，
    肉眼完全看不出来——这就是拦住那类 bug 的东西。

依赖：opencv-python + numpy（仅测试用，不进发布产物）。缺任一个就跳过并提示安装。
用法：python tools/automation/qr_selftest.py
"""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
QR_JS = ROOT / "ui" / "qrcode.js"

# 覆盖：典型局域网 URL（手机面板实际会渲染的东西）、IPv6、带查询串、
# 纯 ASCII 边界（触发模式自动切换）、逐档长度（触发版本自动选择与分块/交错）。
CASES = [
    "http://192.168.1.100:59123/",
    "http://10.0.0.2:59123/",
    "http://[fe80::1]:59123/",
    "http://192.168.31.244:59123/",
    "https://example.com/a/b?c=1&d=2",
    "A",
    "1234567890",
    "HELLO WORLD",
    "x" * 60,
    "y" * 120,
    "z" * 200,
    "n" * 400,
]

NODE_TMPL = """\
import fs from 'node:fs'
const src = fs.readFileSync({qr_js}, 'utf8')
const g = {{}}
new Function('globalThis', src)(g)
const q = g.qrcode(0, 'M')
q.addData(process.argv[2])
q.make()
const n = q.getModuleCount()
const rows = []
for (let r = 0; r < n; r++) {{
  const row = []
  for (let c = 0; c < n; c++) row.push(q.isDark(r, c) ? 1 : 0)
  rows.push(row)
}}
process.stdout.write(JSON.stringify({{size: n, modules: rows}}))
"""


def node_matrix(text: str) -> dict:
    """跑 node 取回矩阵（node 直接跑 ui/qrcode.js，产物里没有 node_modules）。"""
    code = NODE_TMPL.format(qr_js=json.dumps(str(QR_JS)))
    with tempfile.NamedTemporaryFile("w", suffix=".mjs", delete=False, encoding="utf-8") as f:
        f.write(code)
        script = f.name
    try:
        out = subprocess.run(
            ["node", script, text],
            capture_output=True,
            text=True,
            check=True,
            timeout=60,
        )
        return json.loads(out.stdout)
    finally:
        Path(script).unlink(missing_ok=True)


def to_image(modules, scale: int = 6, margin: int = 4):
    """矩阵 → numpy 位图（含留白）。模块之间不插缝，QR 要靠模块本身的黑白。"""
    import numpy as np

    n = len(modules)
    px = np.ones((n + margin * 2, n + margin * 2), dtype=np.uint8) * 255
    for r, row in enumerate(modules):
        for c, v in enumerate(row):
            if v:
                px[r + margin, c + margin] = 0
    return np.kron(px, np.ones((scale, scale), dtype=np.uint8))


def main() -> int:
    try:
        import cv2
        import numpy  # noqa: F401
    except ImportError:
        print("skip: 未安装 opencv-python / numpy（pip install opencv-python numpy）")
        return 0

    detector = cv2.QRCodeDetector()
    failures = 0
    for text in CASES:
        try:
            qr = node_matrix(text)
        except Exception as e:  # noqa: BLE001 - 测试脚本，任何异常都算失败
            print(f"FAIL  {text[:24]!r}({len(text)}B): node 报错 {e}")
            failures += 1
            continue
        img = to_image(qr["modules"])
        got, _, _ = detector.detectAndDecode(img)
        label = f"{text[:24]!r}({len(text)}B) size{qr['size']}"
        if got == text:
            print(f"ok    {label} 解码回原文")
        else:
            print(f"FAIL  {label} 解码得到 {got[:40]!r}")
            failures += 1

    print()
    print("qr_selftest:", "FAIL" if failures else "PASS", f"({failures} 个用例没扫出来)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())