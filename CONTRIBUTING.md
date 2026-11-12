# CONTRIBUTING — 命名、文件与 i18n 规范

> 从 `AGENTS.md` §4 拆出。新增文件/字符串/平台实现前先读。

## 4. 命名与文件规范

- **品牌名统一 `PureVox`**（P、V 大写）：窗口标题、UI、日志、文档一律如此，禁止 `Purevox` / `PUREVOX` 等变体。
- 平台/协议强制小写的标识保持小写：用户数据目录 `~/.purevox/`、Tauri `identifier`
  `com.purevox.desktop`、Android 包名 `com.purevox.mic`、模型代号 `purevox9` 等。
- **代码命名**：Rust snake_case 函数/变量、PascalCase 类型；JS/TS camelCase。
- **平台特征代码按文件分（`<功能>_<平台>.rs`）**：同一功能有平台差异时，各平台实现放进
  `<功能>_windows.rs` / `<功能>_linux.rs` / `<功能>_other.rs`（其它平台 = 明确不可用的桩，
  不静默失败），功能文件本身只保留共享部分 + `#[cfg] #[path = "…_<平台>.rs"]` 的平台分派；
  **禁止**在一个文件里用 `#[cfg(windows)]` 包住大段平台实现体。现状（全部按此分）：
  `audio/loopback_{windows,linux,other}.rs`、`audio/virtual_mic*.rs`、`audio/rt_guard_*.rs`、
  `hotkey*.rs`、`winstate*.rs`、`cues*.rs`、`net/keys*.rs`、`net/opus_sys*.rs`、`devices*.rs`、
  `debug/{mem,gpu}_*.rs`、`openurl_*.rs`、`autostart*.rs`、`tray*.rs`、`sound_panel*.rs`、`vb_panel*.rs`。
  平台文件只放真实差异；跨平台逻辑留在功能文件里，不要为了“看起来整齐”复制三份。
- **但 Python 脚本反过来：尽可能一个 py 跨平台**（上面那条只管 `.rs` 与前端 JS）。理由：脚本
  的平台差异通常只有「产物格式 / 安装方式 / 启动前缀 / 收尾命令」各自三五行，拆成三个文件
  只会让「这件事到底做了哪几步」要跨文件拼起来才看得全；而它们本来就是**每个都要全跑一遍**
  的一次性工具，不进运行时。跨平台靠 `sys.platform` 分支，不靠文件。同理，**容器里跑的断言
  不要再嵌一段 bash** —— 让脚本自己兼任容器内入口（`test_packages.py --in-container`），
  这样宿主侧与容器侧的检查是同一份实现，不会各自漂移。
  **禁止**在 Python 里出现 `subprocess.run(["pkill", ...])` 这类「本机有、runner 没有」的调用：
  Windows 的 Git for Windows 不带 procps（`usr/bin` / `mingw64/bin` 都没有 `pkill`），
  会抛 `FileNotFoundError`；且这类调用常挂在 `finally` 上，**成功也会炸**。
  平台收尾用系统自带的东西：Windows `taskkill /F /T /PID`、Linux `os.killpg`。
- **许可证头**：每个源码文件（`.rs` / `.toml` / `.html` / `.js` / `.ts` / `.css` 等）顶部必须带 GPL-3.0 版权头 +
  模型声明 + `SPDX-License-Identifier: GPL-3.0-or-later`，照抄 `src-tauri/src/main.rs` 顶部并按注释风格替换。
  JSON 无注释语法，豁免。
- **UI 字符串（i18n，GPUI 版）**：中文字面量即 msgid，唯一字典
  `src-tauri/src/ui/i18n.rs`（zh 恒等返回、en 查表、缺键回退中文）。上屏入口只有一个：
  `i18n::tr("中文 msgid")`。新增用户可见中文串必须**同一次提交**补 en 条目；漏了 en
  就是漏译。不翻译的边界：注释、调试日志（`debug::events` / `eprintln!`）、Rust 诊断文本
  （probe 的 `reason` 体等内部原因串）。注册表节点/参数名、模型表、提示音预设同样在界面侧
  经 `i18n::tr` 翻译。调试面板（浏览器版）仍用 `src-tauri/ui/i18n.js`，只服务
  `/debug/ui` 自己的静态页，不要与 GPUI 界面混用。
- **文案风格**：用户可见文案用「对象 + 动词」（`删除节点` / `开始校准` / `安装更新`）；
  错误要写「发生了什么 + 怎么恢复」，不写裸 `failed`。界面语言切换入口未落地前，不要在
  新页面里写只有中文的隐藏分支。
- **README 双语**：中文 `README.md` + 英文 `README_EN.md`，结构变化时两处同步。
- **文档里的「跨平台」只写具体实现差异，不写抽象概念**（用户 2026-10-18 明确要求）：平台表里
  只留「哪个平台的机制真的不同」（ALSA 排他 / 回环用 `parec` 监听 monitor / WASAPI 共享可多开、
  回环是渲染端点上的独立采集流 / 独占模式不回退 / 其它平台 cpal 只有共享式）；「三平台同一套
  语义」「为什么不让 OS 混音」这类**理由**放到不变量或决定处写一次，不铺在平台表里。

---

## 5. 软件工程基线

> 参考：Rust API Guidelines、GPUI Kit Coding Guides、Conventional Commits、SemVer。
> 本仓库的取舍优先于通用规范；有冲突时先改本文。

- **工具链**：Rust stable（`rust-version = 1.95`）、`rustfmt`（2 空格、100 列）、
  `clippy -D warnings`。本机与 CI 跑同一份门禁：
  `python tools/automation/check.py`（`-Gate` 可重复，逐项定位）。
- **分层边界**：`plan` 是图的数据模型；`engine` 只执行图；`audio` 管设备与实时线程；
  `ui` 只做展示与命令；`debug` 是可观测性（`/debug` 与日志同源）；`net` / `update` 是外围。
  低层不依赖 `ui`，业务状态不藏在元素树里。
- **UI 模块化**：`src-tauri/src/ui/` 按职责拆分，新增界面代码放进对应文件，共用原语走
  `ui/widgets.rs`；禁止继续把页面逻辑堆进单个 `mod.rs`。
  - `mod.rs`：外壳（顶栏 / 状态栏）、`Workspace` 状态、启动与轮询；
  - `canvas.rs`：节点卡片、端口、连线、拖拽/选中；
  - `graph.rs`：加删节点、连线、参数/通道、撤销重做、应用计划；
  - `inspector.rs`：当前选中节点的检查器；
  - `dock.rs`：右侧信息栏（节点页 / 全局设置、网络卡片、AEC 校准）；
  - `widgets.rs` / `theme.rs`：展示原语与设计 token；`i18n.rs` / `qr.rs` 是纯工具。
  右侧信息栏**一次只显示一页**（节点检查器或全局设置），不新增“常驻所有模块”的堆叠。
- **错误处理**：用户路径、I/O、设备、网络禁止 `unwrap()` / `expect()` 猜成功；返回
  `Result<_, String>` 或明确错误类型。失败必须同时进状态栏/界面提示与 `debug::events`，
  不允许“只写日志、界面无感”或“只弹窗、日志无因”。
- **unsafe / FFI**：只出现在平台文件（`*_windows.rs` / `*_linux.rs`）和明确的运行时候选，
  每处写清调用方保证；新增 unsafe 必须说明为什么安全、失败时如何回退。
- **测试**：纯逻辑用 `#[cfg(test)]` 单测；真实声卡测量放 `hw-tests` feature；
  新增交互状态（选择、空状态、冲突、禁用）优先补一个能失败的测试。测试不得写用户真实
  `~/.purevox/`（`config::config_dir` 已按测试线程隔离）。
- **提交**：Conventional Commits；scope 必须写 `desktop` / `android` / `core` / `docs` / `ci`
  之一（跨领域可组合）。一个提交只做一件事；界面行为改动同时改文档与 i18n。
- **依赖**：一个功能只有一条实现路径；新增 crate 先问“现有依赖能否扩展”、再问许可证与
  平台编译链。不引入需要 C 编译链的运行时依赖（Opus 等已预编译入库）。
- **死代码**：不保留“以后可能用得上”的实现；被替代的直接删。根上的
  `#![allow(dead_code)]` 只是迁移期临时门闩，新增未接线模块不得再靠它隐藏。
- **文档**：设计先改文档再改代码；`README.md` / `README_EN.md` 结构变化同步；
  `AGENTS.md` 的索引与不变量是入口，不要另起一套平行规范。

---
