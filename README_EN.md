# PureVox

[中文](README.md)

A desktop app for real-time AI microphone noise suppression, target speaker extraction and echo cancellation.

## Status: framework migration in progress

PureVox is being migrated from Python + Tkinter to **Tauri 3**. The mainline is being rebuilt from scratch and
**there is currently no usable build from this branch**.

- For a working version, download `v2026.09.30.1944` or an earlier release from
  [Releases](https://github.com/a2heng/PureVox/releases).
- Previous implementation source: `legacy-v2026.09.30.1944/` (last version before the migration, read-only archive).
- New implementation: `src-tauri/`; build and toolchain notes are in [TAURI3.md](TAURI3.md) (Chinese).

## License

- Source code: GPL-3.0-or-later, see [LICENSE](LICENSE).
- The built-in AI models (`models/*.onnx`) are not covered by the GPL, see [MODEL-LICENSE.md](MODEL-LICENSE.md).
- The author also publishes MIT-licensed models free to use: `lightweight-denoise-48k` / `lightweight-aec-48k`.
