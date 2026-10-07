# PureVox

[English](README_EN.md)

实时 AI 麦克风降噪 / 目标说话人提取 / 回声消除桌面应用。

## 当前状态：框架迁移中

PureVox 正在从 Python + Tkinter 迁移到 **Tauri 3**，主线从零重建，**目前没有可用的功能版本**。

- 需要可用版本：请到 [Releases](https://github.com/a2heng/PureVox/releases) 下载
  `v2026.09.30.1944` 及更早的发行版。
- 旧实现源码：`legacy-v2026.09.30.1944/`（迁移前最后版本，只读归档）。
- 新实现：`src-tauri/`；编译与工具链说明见 [TAURI3.md](TAURI3.md)。

## 许可证

- 源码：GPL-3.0-or-later，见 [LICENSE](LICENSE)。
- 内置 AI 模型（`models/*.onnx`）不随 GPL 授权，见 [MODEL-LICENSE.md](MODEL-LICENSE.md)。
- 作者另有 MIT 授权的模型仓库可自由使用：`lightweight-denoise-48k` / `lightweight-aec-48k`。
