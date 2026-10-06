# PureVox - engine smoke test (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Loads the denoise model and runs one 10ms hop through it.
# Usage: python tools/automation/smoke.py   (run from the repo root)
# Replaces the half-dozen inline heredocs across ci.yml / test.yml /
# verify-local.sh / verify-local.ps1, which used to drift apart.
"""Engine smoke: import pvengine, denoise one 480-sample frame."""

import os
import sys

# Ensure the repo root is on sys.path (run from any working directory).
_REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
if _REPO_ROOT not in sys.path:
    sys.path.insert(0, _REPO_ROOT)

import numpy as np

from pvengine import AudioProcessor


def main() -> None:
    ap = AudioProcessor(0.0)
    x = (np.sin(np.arange(480) * 0.05) * 0.3).astype("float32")
    out = ap.process(x.tolist())
    assert len(out) == 480, f"expected 480 samples, got {len(out)}"
    print("pvengine OK on Python", sys.version.split()[0])


if __name__ == "__main__":
    main()
