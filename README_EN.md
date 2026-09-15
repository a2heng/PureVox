# PureVox

Real-time AI audio processing for your microphone — denoising / target speech extraction /
echo cancellation, for both the local microphone and remote network streaming.

[中文](README.md) | English

## Docs

- 📖 The manual (Chinese) and the changelog are built into the app — menu "关于" (About: intro / Windows / Linux guides / changelog / license).

## Features

- 🎤 Real-time AI denoising (48 kHz, models loaded on demand)
- 🗣️ TSE target speech extraction (record a reference clip, then separate your voice from the background)
- 🔊 AEC echo cancellation
- 🎛️ EQ: 10-band (1-octave) / 31-band (1/3-octave) / 61-band (1/6-octave)
- 📊 AGC automatic gain control / VAD voice activity detection
- 📱 Remote microphone: phone browser / Android APK streams over LAN to the PC for processing
- 🖥️ Windows (WASAPI default / MME fallback) and Linux (pipewire-pulse compatibility layer + libpulse)

## Requirements

| Platform | Requirements |
|---|---|
| Windows | Windows 10/11, Python 3.12+ |
| Linux | Python 3.12+, PipeWire + pipewire-pulse + libpulse (libopus optional) |

> **⚠️ Windows 7 no longer supported**: Python 3.13 drops Win7; `v2026.08.14.1643` is the last Win7-compatible tag — download its [Windows asset](https://github.com/a2heng/PureVox/releases/tag/v2026.08.14.1643) and stay on that tag.

## Quick start

### Embedded Python 3.12 (recommended, independent of the system)

The project can bundle its own Python 3.12, fully isolated from the system Python.
The bootstrap script fetches official prebuilt CPython packages on demand (Windows uses
the NuGet full package; Linux downloads the python-build-standalone install_only tarball
and extracts it — no compilation). Everything lives under `packages/`.

```bash
# Linux:
./bootstrap_python312.sh                     # -> packages/python312 (self-contained) + deps
./py312 run_tk.py                            # run

# Windows (PowerShell, NuGet prebuilt download)
powershell -ExecutionPolicy Bypass -File bootstrap_python312.ps1   # -> packages\python312w
# build_win.ps1 then automatically uses packages\python312w\python.exe
```

Alternatively, use a system Python 3.12+:

```bash
# Windows system Python:
pip install -r requirements-win.txt

python run_tk.py
```

### Linux (AOSC / other distros)

```bash
# System deps (e.g. AOSC; Debian/Ubuntu: apt install pipewire pipewire-pulse libpulse0 libopus0)
sudo oma install -y python3 pipewire

# Recommended: embedded 3.12 (see above)
./bootstrap_python312.sh
./py312 run_tk.py

# Or run directly with system python3:
pip install --user -r requirements-linux.txt
python3 run_tk.py
```

Linux audio goes through the pipewire-pulse compatibility layer (in-house ctypes binding
to system libpulse); the format is negotiated as F32 mono 48000 Hz, with resampling and
channel conversion handled by PipeWire — no self-compiled binaries. The virtual microphone
is a mono null-sink `purevox_out` (single write point) with two outputs: the monitor source
**"PureVox 虚拟麦克风"** (PureVox Virtual Mic) and the remapped real source **"PureVox mic"**
(for OBS and other software that only lists real sources). AEC far-end capture (echo
reference) and loopback input use the same layer (listening to the speaker monitor source).

### Windows remote-mic add-ons

The Opus decoding library for the remote-microphone feature is shipped in the repo
(`server/opus.dll`, prebuilt libopus, BSD); only the **VB-CABLE** virtual sound card
needs a manual install:

- Download `VBCABLE_Setup_x64.exe` from [vb-audio.com/Cable](https://vb-audio.com/Cable/)
  (or the [official driver pack](https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip)
  directly), then double-click the installer and follow the prompts. When PureVox detects
  it is missing, an install guide is shown inline in the audio-output row
  (including the [install video tutorial](https://www.bilibili.com/video/BV1i2bazGEKe/)).

## Packaging

### Windows (bundle directory, CI zips it)

```powershell
powershell -ExecutionPolicy Bypass -File build_win.ps1   # produces dist/PureVox/ (PyInstaller one-folder bundle)
```

The script runs the full flow: PyInstaller one-folder bundling (automatically using
packages\python312w\python.exe) → tcl/tk + unused module cleanup → copy docs. Windows CI runs the same flow and uploads `dist/PureVox/`; the `actions/upload-artifact` step compresses it
to a zip automatically.

### Linux (deb / rpm / AppImage)

```bash
bash pack_deb.sh        # produces dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.deb (source + models + html)
bash pack_rpm.sh        # produces dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.rpm (Fedora/RHEL, bundles embedded Python 3.12 too)
bash pack_appimage.sh   # produces dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.AppImage (bundles embedded Python 3.12)
```

| Artifact | Name pattern |
|---|---|
| Windows (CI zips the upload) | `PureVox-Windows-x64-<date>-release` |
| Linux deb/rpm/AppImage | `PureVox-Linux-x64-<date>-release.<deb\|rpm\|AppImage>` |
| Android APK | `PureVox-Android-arm64-<date>-debug.apk` |

Timestamp `<date>` = `yyyy-MM-dd-HHmm` (filename); the in-package version field =
`yyyy.MM.dd.HHmm` (e.g. `2026.08.10.1517`), **derived from the tag name**
(`v2026.08.10.1517` → `2026.08.10.1517`), so deb/rpm/setup.py versions stay in sync and
never drift with the job's wall-clock time.

Release: pushing a tag `v<yyyy.MM.dd.HHmm>` (e.g. `v2026.08.10.1517`) triggers CI, and the
release job creates a GitHub Release with all artifacts (deb / rpm / AppImage / Windows zip / APK)
attached.

### Android APK

```bash
cd android
./gradlew assembleDebug    # output: android/app/build/outputs/apk/debug/app-debug.apk
```

Requires JDK 17, Android SDK platform 34, NDK r27. On first build put the Opus source at
`android/opus-src/` ([xiph/opus v1.5.2](https://github.com/xiph/opus/archive/refs/tags/v1.5.2.zip)).

## Remote microphone

Phone / browser → WSS(Opus) → PC server → AI processing chain → speaker / virtual mic

```
Phone → https://<PC_IP>:59123 (mDNS broadcast _purevox._tcp.local.) → denoise → output
```

- Browser: phone and PC on the same LAN, visit `https://<PC_IP>:59123`, trust the
  self-signed cert, tap the mic button to stream
- APK: opens and auto-discovers LAN servers, connects and streams automatically
- Client message: `{"type":"audio","data":"<base64 opus>","seq":N}`; server replies `{"type":"ack","seq":N}`
- Frame size 480 samples (10 ms @48 kHz), aligned with the Opus encoders and the engine hop

## Project layout

```
run_tk.py                 # entry point (single-instance lock + Tk main window)
uitk/                     # desktop UI (pure stdlib Tkinter): node panel, EQ editor, About
about_content.py          # About-page metadata/intro/license text (changelog & manuals live in about/*.md)
audio_processor.py        # core audio thread (capture/playback/network loops) + TSE reference recording utilities
pvengine/                 # pure-Python componentized audio engine (numpy + scipy + onnxruntime)
pvplatform/               # platform abstraction: audio/ (enum, SpeakerCapture), system/ (single-instance, virtual mic)
config_manager.py         # JSON config (strong config, per-API device keys)
model_config.py           # ONNX model filename constants
server/                   # remote-mic HTTPS/WSS server (aiohttp + Opus + mDNS + TLS)
html/                     # browser streaming front-end (AudioWorklet + Opus WASM)
android/                  # Android client (Kotlin + OkHttp + Opus JNI)
pack_deb.sh               # Linux deb packaging
pack_rpm.sh               # Linux rpm packaging (Fedora/RHEL)
pack_appimage.sh          # Linux AppImage packaging (bundles embedded Python 3.12)
build_win.ps1             # Windows packaging (PyInstaller bundle dir)
bootstrap_python312.sh / .ps1  # embedded Python 3.12 bootstrap (Linux downloads prebuilt tarball, Windows fetches NuGet)
```

## Tech stack

| Component | Technology |
|---|---|
| Desktop GUI | Python stdlib Tkinter (uitk, Stardew-pixel light theme) |
| Audio processing | Pure-Python engine pvengine (numpy + scipy + onnxruntime) |
| Linux audio | PipeWire (pipewire-pulse compatibility layer; in-house ctypes binding to system libpulse) |
| Windows audio | WASAPI (default) / MME fallback; separate input/output streams + per-output PlaybackSink |
| Server | Python aiohttp + zeroconf + cryptography |
| Audio codec | Opus (PC: opuslib, APK: NDK build, Web: WASM) |
| Android | Kotlin + OkHttp + NsdManager + AudioRecord |

## License

- **Source code**: [GPL-3.0](LICENSE) (GNU General Public License v3.0 or later)
- **Built-in AI models**: NOT covered by the GPL. They are the property of a2heng and may
  only be used with PureVox under authorization — see [MODEL-LICENSE.md](MODEL-LICENSE.md)

Author's MIT-licensed model repos (earlier versions, freely usable):

- <https://github.com/a2heng/lightweight-denoise-48k>
- <https://github.com/a2heng/lightweight-aec-48k>

Third-party components (ONNX Runtime, Opus, NumPy, etc.) are under their own
licenses; see [LICENSE-THIRD-PARTY.txt](LICENSE-THIRD-PARTY.txt).

## Contact

- GitHub: <https://a2heng.github.io/>
- Bilibili: <https://space.bilibili.com/10850943>
