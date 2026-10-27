# PureVox — AI 麦克风降噪

实时 AI 音频降噪 / 目标说话人提取 / 回声消除的桌面应用。

**栈**：Tauri 3（alpha）+ Rust 后端 + WebView 前端（Windows WebView2，wry 运行时）
**工程**：`src-tauri/`（编译/运行/打包见 `TAURI3.md`）

> **本文件只保留总纲、索引和跨主题硬约束；细节在子文档里。**
> 开工前先读 `ARCHITECTURE.md`；**实现与文档冲突时以文档为准**，改设计先改文档。

## 文档索引

| 文档 | 内容 |
| --- | --- |
| `ARCHITECTURE.md` | 分层、数据不变量、列/行、Stage/Pipeline、SessionPlan、设备池、模型生命周期、内存、扩展、实施顺序 |
| `PLUGINS.md` | 节点清单（输入/处理/输出/可视化）、AEC/TSE、媒体行、OBS 滤镜、31 段 EQ、AGC |
| `NETWORK.md` | 手机 ⇄ 电脑链路、协议、缓冲、TLS/mDNS、Android/网页客户端、网络卡片 |
| `UI.md` | 视觉风格与布局（不写死高度、侧栏宽度、网络卡、EQ、调试面板） |
| `UX.md` | 交互行为（热参数、行/列开关、扫码/重连、预设、失败不静默） |
| `DEBUG.md` | 调试优先：面板、HTTP 接口、Probe、实现约束、AI 可复现 |
| `RELEASE.md` | 版本号、CI 门禁、打包/产物、更新检查 |
| `DEV.md` | 本机开发环境（Windows/Git Bash/WSL 的实测结论） |
| `CONTRIBUTING.md` | 命名、平台文件拆分、i18n、README 双语 |
| `TAURI3.md` | Tauri 3 现状、资料固化、版本锚点、编译步骤、踩坑 |
| `IDEAS.md` | 还没开工的设想（不是承诺） |

## 维护准则（简版）

1. **一个功能只有一条实现路径**。有多种做法时只保留一种并写进文档，被替代的实现直接删除，不留备选。
2. **先扩展，再新建**：先看已有函数/模块能否扩展，确认不能才新建。
3. **不再保留旧实现快照**：`legacy-v*/` 两个只读归档已按要求删除（2026-10-27）。
   由此产生的两条纪律：① 想查「旧行为」只能看 git 历史（`git show <old>:path`），
   工作树里**没有**第二份实现可以照抄；② 现役树里**不许**再堆积用不到的旧资产
   （见本章第 5 条）——删掉的东西就别以「以后可能用得上」为由留一份。
4. **当前不写用户更新日志**：旧的 `about/changelog.md` 已随旧实现归档；开发进度记在 `TAURI3.md`。
   新版具备用户可见功能后再恢复更新日志，届时在本节写明位置。
5. **遗留二进制：只剩仍在用的**（不再有「收进快照备查」这条路，不用就删）：
   - **现役引用**：`server/opus.dll`（Windows 网络 Opus，运行时加载，见 `DEBUG.md` §1.2）、
     `android/app/src/main/jniLibs/arm64-v8a/libpvopus.so`（Android 网络 Opus，内含
     libopus 1.5.2 静态库 + `android/app/src/main/cpp/pv_opus.cpp` 的 JNI 转发层，
     NDK 预编译入库，**重建用 `tools/automation/build_opus_android.py`**）、
     `android/app/src/main/cpp/third_party/opus/{include/*.h, arm64-v8a/libopus.a}`（同上，编译输入）、
     `android/gradle/wrapper/gradle-wrapper.jar`（Android 客户端构建，**勿替换**）、
     `assets/icons/audio_icon_base.png`（`cargo tauri icon` 源图）。
   - **已删除**（现役实现不引用，2026-10-27 随快照一起清掉）：
     `assets/icons/{audio_icon.ico, tray_running.ico, tray_stopped.ico, lite_tray.ico, lite_tray.png}`、
     `assets/fonts/ark-pixel-12px-monospaced-zh_cn.ttf`、`html/wasm/libopus-encoder.wasm.min.wasm`。
     现役图标是 `src-tauri/icons/`、界面用系统字体（不捆绑字体文件）。

## 共同不变量

- **音频格式**：引擎内部一律 F32 单声道 48 kHz；设备原生格式只在平台层转换（重采样、下混）。
- **10 ms hop 网格**：`hop = SAMPLE_RATE / 100`（48 kHz 下 480 样本，FFT/OLA 窗 2×hop = 960）。
  所有数据面按 10 ms 整数倍前进，不引入 1024/2048 等错位块。**可视化频谱**用 4800 点
  （100 ms）分析窗只为频率分辨率，不进信号路径（见 `UI.md`）。
- **模型**：`models/*.onnx` 为现役模型文件，不随 GPL 授权（`MODEL-LICENSE.md`），
  禁止提取用于其他项目。
- **一个功能只有一条实现路径**；被替代的实现直接删，不留备选。
- **调试优先**：看不见的状态等于不存在的状态；新增模块必须同时接入 `/debug`（`DEBUG.md`）。
- **不改系统音量 / 静音**：测试、校准、诊断都只读系统状态。

## 许可证

- 源码 **GPL-3.0**（`GPL-3.0-or-later`），见 `LICENSE`；第三方声明见 `LICENSE-THIRD-PARTY.txt`。
- 内置 AI 模型（`*.onnx`）归 a2heng 所有，仅随 PureVox 经授权使用，见 `MODEL-LICENSE.md`。
- 作者另有 MIT 模型仓库：`lightweight-denoise-48k` / `lightweight-aec-48k`。
