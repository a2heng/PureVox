# PureVox

[中文](README.md)

A desktop app for real-time AI microphone noise suppression, target speaker extraction and echo
cancellation, and a **multi-device mixer / router**: each input strip is source → fader → AI
processing chain → output buses. Buses are named by purpose (monitor / virtual mic / network), not
A1/A2. It is not a VoiceMeeter clone; AI processing and low latency come first.

## Features

- **AI denoise**: ONNX streaming model on a 10 ms grid (48 kHz / 480 samples), single-threaded,
  measured ~0.3 ms per hop (denoise small 0.28 / medium 0.30 / large 0.51).
- **Target speaker extraction (TSE)**: record a 10 s reference and keep only that voice.
- **Echo cancellation (AEC)**: remove speaker sound picked up by the microphone (calls, streaming,
  speakerphone), with delay calibration and level matching.
- **OBS audio filters**: noise gate, compressor, limiter, expander, upward compressor, 3-band EQ and
  polarity invert — ranges, defaults and steps match OBS.
- **31-band EQ / visualizer**: drag 31 bands (±24 dB, presets, low/high cut) and read VU + 64-band spectrum in the inspector.
- **Multi-device mixer (native GPUI node canvas)**: sources, processing nodes and outputs are nodes;
  drag a wire between ports to route. Nodes can fan out / fan in; buses are named by purpose.
- **Device pool**: one device used by several columns is opened once; multiple sources are summed by
  the software mixer, so nothing reports "device busy".
- **Source and sink types**: microphone, system-audio loopback, phone (network mic / speaker), test
  tone, media (soundpad / music player), any output device.
- **Virtual microphone**: VB-CABLE on Windows, self-created `purevox_mic` on Linux — pick it in OBS,
  a browser or a meeting app.
- **Phone as mic / speaker**: HTTPS + Opus over LAN (web client or Android APK). Audio only.
- **Settings page**: language (Chinese / English), start with Windows, global hotkey and cue-sound
  toggles, hotkey recording, and cue preset previews. Each soundpad pad can record its own hotkey.
- **System tray**: minimize (-) and close (×) both go to the tray; the menu shows/hides the window,
  toggles the engine, and quits.
- **Debug-first**: a separate debug window and the read-only HTTP endpoint
  `http://127.0.0.1:47821/debug` share one snapshot (system metrics, per-stream
  rate/buffer/underrun/drop counters, inference time, spectrum and waveform).

## Status

Audio engine complete: capture + resampling, playback with clock servoing, denoise / TSE / AEC,
OBS filters, device pool and mixing, networking (incl. the Android client), virtual drivers and
media rows. The UI is now a GPUI node canvas with per-node inspectors; AEC calibration, TSE
reference recording, media nodes, tray, hotkey recording and cue presets are wired. Window geometry memory and Linux tar.gz/deb packaging are wired; the Windows tray is complete, the Linux tray covers engine/quit only.

- Downloads: [Releases](https://github.com/a2heng/PureVox/releases) (version = year.month.day).
- The previous implementation (Python + Tkinter) is gone; use `git log` if you need it.
- Docs: [AGENTS.md](AGENTS.md) (Chinese), [SESSION-V2.md](SESSION-V2.md) (node graph design), [BUILD.md](BUILD.md).

## Quick start

```bash
# Development build + run
cargo build --manifest-path src-tauri/Cargo.toml
./src-tauri/target/debug/purevox.exe

# Tests / gate
cargo test  --manifest-path src-tauri/Cargo.toml --bin purevox
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Installer packaging is not restored yet (the Tauri bundler is gone); see `BUILD.md`.

## Dependencies

**Runtime**

| Platform | Needs | Notes |
| --- | --- | --- |
| Windows | GPU (DX11/12) | GPUI renders the UI on the GPU; integrated graphics is fine |
| Windows | VB-CABLE (optional) | Install it to expose PureVox as a virtual microphone |
| Windows | `opus.dll` | Shipped with the app; only network audio needs it |
| Linux | ALSA | Audio I/O (GPUI needs X11/Wayland + GPU) |
| Linux | PipeWire or PulseAudio | Device enumeration and capture (`pactl` / `parec` / `pw-dump`) |
| Linux | `libopus0` (optional) | Network audio; without it that feature reports why, everything else works |
| Linux | `pulseaudio-utils` (`paplay`) | Start/stop cue sounds; without it you only lose the sound |
| Linux | `nvidia-smi` or `amdgpu` sysfs (optional) | GPU readouts in the debug panel; absent → hidden, audio unaffected |
| Linux | `.desktop` entry (shipped in packages) | **Global hotkeys** need a desktop app id: start from the app grid, not a terminal (the UI says so) |
| Linux | `pkexec` (deb/rpm self-update) | Graphical prompt, then the package manager installs |
| Android | Android 8.0+ (API 26) | The web client needs no install — just open the desktop URL |

**Build time**

- Rust (`rust-version = "1.95"`); on Linux also `libasound2-dev pkg-config` and GPUI's X11/Wayland deps.
- Python 3 (gates / package / release scripts in `tools/automation/`); the UI is Rust, no Node/tsc.
- `onnxruntime` is downloaded by `ort` at build time; on Linux you can also link a system or local
  `libonnxruntime.so` (see the `ORT_*` variables in `BUILD.md`).

## Models and licensing

- Source: [GPL-3.0-or-later](LICENSE); per-dependency notices in
  [LICENSE-THIRD-PARTY.txt](LICENSE-THIRD-PARTY.txt).
- The built-in models (`models/*.onnx`) are **not** GPL-licensed and may only be used with PureVox,
  see [MODEL-LICENSE.md](MODEL-LICENSE.md).
- Want to train or swap in MIT-licensed models:
  [lightweight-denoise-48k](https://github.com/a2heng/lightweight-denoise-48k) (denoise) and
  [lightweight-aec-48k](https://github.com/a2heng/lightweight-aec-48k) (echo cancellation).
  ⚠ Those repos use the **old waveform-hop contract** (STFT inside the graph). Since 2026-10-29 the
  built-in models speak the **complex-spectrum contract** (`(1,2,481)` in/out, FFT outside the graph,
  see `models/README.md`), so they cannot be dropped into `models/` as-is — re-export them against
  the same contract first (loading one as-is fails loudly with "not a spectrum interface").

Copyright (C) 2024-2026 a2heng <752848283@qq.com>
