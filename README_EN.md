# PureVox

[中文](README.md)

A desktop app for real-time AI microphone noise suppression, target speaker extraction and echo
cancellation. Wire microphones, system audio, phones and any other source into editable columns,
process them, and send the result to speakers, a virtual cable or a phone — one chain instead of a
shelf of tools.

## Features

- **AI denoise**: ONNX streaming model on a 10 ms grid (48 kHz / 480 samples), single-threaded,
  measured ~1.8–6 ms per hop.
- **Target speaker extraction (TSE)**: record a 10 s reference and keep only that voice.
- **Echo cancellation (AEC)**: remove speaker sound picked up by the microphone (calls, streaming,
  speakerphone), with delay calibration and level matching.
- **OBS audio filters**: noise gate, compressor, limiter, expander, upward compressor, 3-band EQ and
  polarity invert — ranges, defaults and steps match OBS.
- **Multi-column pipelines**: each column is input → processing rows → output; add/remove columns,
  reorder rows, suspend any row or column independently.
- **Device pool**: one device used by several columns is opened once; multiple sources are summed by
  the software mixer, so nothing reports "device busy".
- **Source and sink types**: microphone, system-audio loopback, phone (network mic / speaker), test
  tone, media (soundpad / music player), any output device.
- **Virtual microphone**: VB-CABLE on Windows, self-created `purevox_mic` on Linux — pick it in OBS,
  a browser or a meeting app.
- **Phone as mic / speaker**: HTTPS + Opus over LAN (web client or Android APK), plus an optional
  remote-input feature (typing / keyboard, **off by default**).
- **Tray-first**: start/stop, show/hide, quit; minimize and close both go to the tray; global hotkey;
  Chinese and English UI.
- **Debug-first**: a separate debug window and the read-only HTTP endpoint
  `http://127.0.0.1:47821/debug` share one snapshot (system metrics, per-stream
  rate/buffer/underrun/drop counters, inference time, spectrum and waveform).

## Status

Feature complete: capture + resampling, playback with clock
servoing, denoise / TSE / AEC, OBS filters, device pool and mixing, networking (incl. the Android
client), virtual drivers, media rows, in-app updates and packaging are all in place.

- Downloads: [Releases](https://github.com/a2heng/PureVox/releases) (version = year.month.day).
- The previous implementation (Python + Tkinter) is gone; use `git log` if you need it.
- Docs: [DESIGN.md](DESIGN.md) (Chinese), [TAURI3.md](TAURI3.md) (Tauri 3 notes), [AGENTS.md](AGENTS.md).

## Quick start

```bash
# Run in development (Windows / Linux)
cd src-tauri && cargo tauri dev

# Build installers: Windows = NSIS exe; Linux = deb / rpm / AppImage
cd src-tauri && cargo tauri build
```

The first build needs network access: Rust crates, `onnxruntime` (via `ort`'s `download-binaries`)
and the Tauri bundling tools. Linux also needs the system packages listed below.

## Dependencies

**Runtime**

| Platform | Needs | Notes |
| --- | --- | --- |
| Windows | WebView2 runtime | Usually present on Win10/11; the system offers to install it |
| Windows | VB-CABLE (optional) | Install it to expose PureVox as a virtual microphone |
| Windows | `opus.dll` | Shipped with the app; only network audio needs it |
| Linux | WebKitGTK 4.1, GTK3, ALSA | UI and audio I/O |
| Linux | PipeWire or PulseAudio | Device enumeration and capture (`pactl` / `parec` / `pw-dump`) |
| Linux | `libopus0` (optional) | Network audio; without it that feature reports why, everything else works |
| Linux | `pkexec` (deb/rpm self-update) | Graphical prompt, then the package manager installs |
| Android | Android 8.0+ (API 26) | The web client needs no install — just open the desktop URL |

**Build time**

- Rust (`rust-version = "1.95"`) + `tauri-cli` 3.0 alpha (`cargo install tauri-cli --version 3.0.0-alpha.5` — the version pinned here); on Linux also
  `libwebkit2gtk-4.1-dev libssl-dev libxdo-dev librsvg2-dev libgtk-3-dev libasound2-dev pkg-config
  file rpm`.
- Node.js 24 (`tsc` + i18n gate) and Python 3 (build/release scripts in `tools/automation/`).
- `onnxruntime` is downloaded by `ort` at build time; on Linux you can also link a system or local
  `libonnxruntime.so` (see the `ORT_*` variables in `TAURI3.md`).

## Models and licensing

- Source: [GPL-3.0-or-later](LICENSE); per-dependency notices in
  [LICENSE-THIRD-PARTY.txt](LICENSE-THIRD-PARTY.txt).
- The built-in models (`models/*.onnx`) are **not** GPL-licensed and may only be used with PureVox,
  see [MODEL-LICENSE.md](MODEL-LICENSE.md).
- Want to train or swap in MIT-licensed models:
  [lightweight-denoise-48k](https://github.com/a2heng/lightweight-denoise-48k) (denoise) and
  [lightweight-aec-48k](https://github.com/a2heng/lightweight-aec-48k) (echo cancellation).
  They follow the same 10 ms / 48 kHz streaming contract, so they can replace the files under
  `models/` directly.

Copyright (C) 2024-2026 a2heng <752848283@qq.com>
