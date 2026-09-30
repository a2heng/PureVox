# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

PureVox — real-time AI audio denoising (denoise / target-speaker extraction / AEC) for Windows & Linux desktop (Python 3.12 + Tkinter) plus an Android remote-mic client and browser (purevox-web) variants.

**Read these before making changes** (docs use progressive disclosure):
- `AGENTS.md` — hard cross-module rules and invariants (authoritative alongside DESIGN.md)
- `DESIGN.md` — layered architecture / node model / data-flow invariants. **When code conflicts with AGENTS.md or DESIGN.md, DESIGN.md wins.**
- `.opencode/skills/purevox-{build,architecture,ci-release,linux}/SKILL.md` — task-specific recipes, loaded on demand

## Commands

```bash
# Run desktop app
python run_tk.py                          # or bootstrap embedded Python first:
./bootstrap_python312.sh                  # Linux -> packages/python312
powershell -ExecutionPolicy Bypass -File bootstrap_python312.ps1   # Windows -> packages\python312w

# Tests (suite = plain scripts run via runpy; no pytest/unittest runner)
./py312 tests/run_all.py                  # full suite (CI parity)
python tests/test_playback_sink.py        # single test file (each is __main__-executable)

# Compile check (syntax)
bash tools/automation/compileall.sh ./py312
./py312 tools/automation/smoke.py         # engine smoke test

# Packaging
powershell -ExecutionPolicy Bypass -File build_win.ps1   # -> dist/PureVox/ (PyInstaller)
bash pack_deb.sh / pack_rpm.sh / pack_appimage.sh        # Linux

# Browser web build (purevox-web)
python purevox-web/build/build_web.py [--model a|b|c] [--base-url <cdn>]
python purevox-web/serve.py --open        # local HTTPS server (port 59124); mic needs secure context

# Android
cd android && ./gradlew assembleDebug     # needs opus-src/ populated, JDK 17, NDK 27
```

## Architecture (L0–L4, each layer imports only the layer below)

```
L4 UI     uitk/            Tkinter; renders node rows, collects intent only
L3 会话    session_plan.py  chain doc -> executable session plan (pure function, unit-tested)
L2 传输    audio_processor.AudioThread  read -> process -> sinks.write; backend plugins
                           PwBridge (Linux libpulse ctypes) / PaBridge (Windows WASAPI/MME)
L1 引擎    pvengine/        Stage pipeline, pure DSP (numpy/scipy/onnxruntime), no I/O
L0 平台    pvplatform/      device enumeration, system integration
```

- Everything user-visible is a **node** (`NodeSpec` registry in `pvengine/plugins.py`; discover only via `all_specs()` / `get_spec()`). Kinds: `input` (multi-instance, mix), `output` (multi-instance, fan-out), `fx` (serial, user-ordered), `viz` (read-only tap). New audio features = new Stage component, not pipeline changes.
- Config is the **chain document** (`plugin_chain` JSON: `type/enabled/params`); unknown types ignored, no migration (strong config).
- DSP imports (numpy/scipy/onnxruntime) are allowed **only** inside `pvengine/`. uitk and pvplatform just move frames.
- All playback timing correctness lives in one place: `pvengine/dsp/playback.py` `PlaybackSink` (PI-servo ASRC). Never write buffer strategies (zero-pad/drop/reuse/resample) inside callbacks.

## Hard invariants (violations = defects)

- **10ms hop everywhere**: `hop = SAMPLE_RATE // 100` (48kHz → 480 samples, NFFT = 2×hop = 960), derived from time, not fixed counts. No fixed blocks misaligned with the 10ms grid (1024/2048 etc.) anywhere, including viz, network Opus frames, browser capture. Buffer watermarks are hop multiples.
- **Internal format is always F32 mono 48kHz**; resampling/channel conversion happens only at L0 (platform).
- **Single implementation path per feature** (project's prime constraint): extend existing functions/classes before creating new ones; replaced implementations are deleted, never kept as alternatives. Exception: per-API-suffixed device config keys in `config_manager.py` are intentional shared placeholders — do not delete them.
- `legacy-v2026.08.20.1943/` is a **frozen read-only snapshot**: never modify it, never let it participate in builds/CI/tests/global refactors.

## Conventions

- License header: every source file starts with the GPL-3.0 header + model notice + `SPDX-License-Identifier: GPL-3.0-or-later` (copy from `audio_processor.py` top, adjust comment style).
- Brand is always `PureVox` in user-visible text; lowercase `purevox` only for platform/protocol-mandated identifiers (Android package `com.purevox.mic`, mDNS `_purevox._tcp.local.`, `~/.purevox/`, localStorage keys, model code `purevox9`).
- `.ps1` scripts must be **pure ASCII** (PowerShell 5.1 misreads BOM-less UTF-8); use wildcards for Chinese filenames.
- Changelog: user-perceivable changes go at the **top** of `about/changelog.md` (Chinese, no emoji). Dev-process-only changes (CI, packaging, refactors, formatting) must NOT go into the changelog or README.
- `README.md` (Chinese) and `README_EN.md` must be kept in sync for structure/packaging changes.
- Desktop dialogs live in `uitk/dialogs.py` (`open_*` / `show_*` entry points); do not create parallel `dialog_*.py` files.
- Device-list refresh has a single entry point (`MainWindowTk.refresh_devices()`, background thread); never enumerate devices on the UI thread or at any other trigger point.
- Errors: internal `try/except` + `_module_log()`, never bubble to UI thread; logging via `logger.py` (`dev`/`msg`/`warn`/`err`).
- Naming: Python snake_case; C++ PascalCase classes; Kotlin camelCase.
- Licenses: code GPL-3.0; bundled ONNX models are proprietary (see `MODEL-LICENSE.md`) — never extract them.
