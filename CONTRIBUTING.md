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
  `audio/loopback_{windows,linux,other}.rs`、`audio/virtual_mic*.rs`、`hotkey*.rs`、
  `winstate*.rs`、`cues*.rs`、`net/keys*.rs`、`net/opus_sys*.rs`、`devices*.rs`、
  `debug/{mem,gpu}_*.rs`、`openurl_*.rs`。前端同理：`ui/drivers_linux.js` / `ui/drivers_windows.js`。
  开机自启**不在此列**：`tauri-plugin-autostart` 已是跨平台实现（见 §1.3 外围），不要再手写一份。
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
- **UI 字符串（i18n，沿袭旧实现 `i18n.py` / 旧 AGENTS 规则 13）**：中文字面量即 msgid，
  唯一字典 `src-tauri/ui/i18n.js`（zh 恒等返回、en 查表、缺键回退中文）。上屏只有三个入口：
  静态 `data-i18n="…"`、动态 `T('…')`（可带占位符 `T('列 {n}', { n: 1 })`，中英 `{name}`
  集合必须一致）、Rust 中文模板串 `__pvTpl(raw)`（TPL 表：zh = Rust `format!` 字面骨架、
  en = 同构英文模板、id 为稳定短名供业务逻辑判定，Rust 改拼串必须同步 TPL 骨架）。
  **Rust 提供的固定标签**（注册表节点/参数名、模型表、提示音预设）同样在界面侧经 `T()`
  翻译（中文即 msgid，条目必须在字典里）；**动态写入的文本**（状态串、按钮文字、下拉选项）
  必须同时可重放：记录最近状态 + 监听 `document` 的 `pv-langchange` 事件按新语言重绘。
  **其余中文一律不上屏**；不翻译的边界：注释、调试日志（`__pvDebug` / `ui_report` /
  `console`）、Rust 诊断文本（probe 的 reason 体，由变量拼接不经扫描）。**新增用户可见中文串
  必须同步加 en 条目**（漏了 en 就是漏译）。门禁 `tools/automation/i18n_lint.js`
  （挂 `check.py -Gate ui`）：E1 裸中文上屏、E2/E3 缺键、E4 占位符奇偶、E5 模板 id 重复、
  E6 Rust 标签缺条目为错误；W1 孤儿键、W2 TPL 骨架与 Rust 源脱节为告警。
- **README 双语**：中文 `README.md` + 英文 `README_EN.md`，结构变化时两处同步。
- **文档里的「跨平台」只写具体实现差异，不写抽象概念**（用户 2026-10-18 明确要求）：平台表里
  只留「哪个平台的机制真的不同」（ALSA 排他 / 回环用 `parec` 监听 monitor / WASAPI 共享可多开、
  回环是渲染端点上的独立采集流 / 独占模式不回退 / 其它平台 cpal 只有共享式）；「三平台同一套
  语义」「为什么不让 OS 混音」这类**理由**放到不变量或决定处写一次，不铺在平台表里。

---
