# DEV — 本机开发环境（实测结论）

> 从 `AGENTS.md` §2.8 拆出。全是踩出来的，别凭常识重推一遍。

### 2.8 本机开发环境（实测结论，别凭常识重推一遍）

这一节全是**踩出来的**，每条都有代价，别当成可选项绕过去。

- **智能体的 shell 是 Git Bash**（全局 `~/.config/opencode/opencode.jsonc` 的 `"shell"`）。
  管道输出是标准 UTF-8，中文正常（换 PowerShell 反而全变 `?????`，见下）。
  - 路径：`/d/PureVox`（D 盘）、`/c/Users/...`（C 盘）。**没有 `/mnt/c`**，那是 WSL 的。
  - **命令里一律用单引号**：harness 把命令交给 `bash -c` 时**双引号会被外层吃掉**，
    `python -c "a; b()"` 直接语法错误，`python -c 'a; b()'` 正常。带括号的 jq 表达式
    写进临时文件再 `--jq "$(cat f)"`。
  - **不要把 shell 指回 PowerShell**：opencode spawn PowerShell 系时一律加 `-NoProfile`，
    `[Console]::OutputEncoding` 退回系统 ANSI 代码页（本机 936/GBK），子进程往管道写的是
    **GBK 字节**而调用方按 UTF-8 解 → 中文全变 `?????`。实测
    `[Console]::OutputEncoding = gb2312`、输出字节 `d6d0cec4b2e2cad4…`。
    （曾试过装便携版 PowerShell 7 想解决，**没用** —— `-NoProfile` 同样跳过它。
    顺带证明「换个新 shell 就好了」是想当然，已卸载，别再重装。）
- **Windows 上只有系统自带的 PowerShell 5.1**（`powershell.exe`），仓库脚本一律别写 `.ps1`：
  - **无 BOM 的 UTF-8 `.ps1` 按 GBK 解码**，中文把字符串引号吃掉 → 报
    `表达式或语句中包含意外的标记`，脚本根本没跑起来。写带中文的 `.ps1` 必须存成
    **带 BOM 的 UTF-8**，或者干脆全用 ASCII 注释。
  - `>/dev/null` 在 PowerShell 里**不是重定向、是真路径**（会真的建出 `dev\null` 这类垃圾文件，
    git status 里能看到）。要丢输出用 `| Out-Null`。
  - 命令行里拼复杂逻辑（`python -c "..."`、`gh --jq '...\(...)...'`）必被嵌套引号炸掉，
    写成脚本文件再执行。
- **本机网络对 GitHub 不友好**，要下二进制前先看这张表：

  | 目标 | 状态 | 走哪条 |
  | --- | --- | --- |
  | `github.com` / `api.github.com` | ✅ 通 | `gh` / `gh api` 正常 |
  | `raw.githubusercontent.com` | ❌ DNS 解析成 `0.0.0.0` | jsDelivr `cdn.jsdelivr.net/gh/<repo>@<ref>/<path>` |
  | GitHub release 资产 | ❌ 四个 CDN IP（`185.199.108-111.133`）全部 `000` | `gh-proxy.com`（~490 KB/s，实测最快；`ghfast.top`/`ghproxy.net` 只有 50 KB/s 左右） |
  | `wsl --list --online` | ❌ 它要拉上面那个 raw | 直接下官方 `.wsl` 再 `wsl --install --from-file` |

  - 包管理镜像：apt `mirrors.aliyun.com`、pip 清华、npm `registry.npmmirror.com`、cargo `rsproxy.cn`。
  - **经过第三方代理下载的二进制必须校验 SHA256** —— `gh-proxy.com` 这条尤其，
    校验值要从官方源单独取（PowerShell 的 `hashes.sha256` 就是这么验的）。
- **Windows 上从 Python 调 `bash` 会命中 WSL 启动器**：`C:\Windows\System32\bash.exe`
  排在 PATH 前面，它起的是 **WSL 发行版**，把 Windows 的**整个环境块丢掉**（子进程里
  `HOME=/root`、`PATH=/usr/local/sbin:…`，传进去的 `TAG`/`VERSION` 一个都读不到，文件路径
  也换了一套视图）。症状极其隐蔽：脚本「跑通了」，但所有环境变量都是空，很容易误判成脚本
  自己的 bug。**要显式找 Git Bash**（`C:\Program Files\Git\bin\bash.exe`），别信 PATH ——
  `tools/automation/run_release_stage.py` 的 `find_bash()` 就是干这个的。
- **WSL 两个发行版**：门禁跑 **`Ubuntu-24.04`**（与 CI 基线一致，glibc 2.39；仓库挂
  `/mnt/d/PureVox`，`~/.cargo/config.toml` 用 rsproxy **sparse**，`ORT_*` 指向 `/opt/ort`
  的官方 release）；`Ubuntu-26.04`（glibc 2.43）留着只做**向前兼容抽查** —— 用来验证
  「24.04 编的包在更新的 glibc 上真能跑」，别用它跑门禁（`log10f@GLIBC_2.43` 这类符号
  会让 24.04 的产物根本起不来）。以下几条两个发行版通用：
  - 两个发行版默认用户都是 root，systemd 已开；仓库挂 `/mnt/d/PureVox`（drvfs）。
  - **`wsl.exe` 输出是 UTF-16**：调它之前先设 `$env:WSL_UTF8 = "1"`，否则中文全是乱码。
  - `appendWindowsPath=false` 已关 —— 否则 PATH 里混着 Windows 的 `cargo.exe` / `nodejs`，
    Linux 构建可能调到 Windows 的 cargo。
  - `/root/.cargo/config.toml` 已把 `build.target-dir` 指到 `/root/purevox-target`
    （drvfs 上 Rust 构建慢一个数量级；CI 也在 Linux 文件系统上构建，更贴近）。
  - **`ort-sys` 绝不能让它下载**：build.rs 默认从 pyke CDN 拉 `libonnxruntime`，
    本机连得上但**收不到数据**（实测 20 秒 0 字节；另测过 CDN 限速 10–14 KB/s，
    一个 10 MB 的包要十几分钟）。两个发行版都改成本地已有的动态库，经
    `/root/.cargo/config.toml` 的 `[env]` 注入 `ORT_LIB_LOCATION` / `ORT_SKIP_DOWNLOAD` /
    `ORT_PREFER_DYNAMIC_LINK`：**24.04 用 `/opt/ort` 下官方 release 的 `.so`**
    （走 gh-proxy.com 下载，已校验 SHA256；只含 `.so`、没有 `.a`，所以必须动态链），
    26.04 用 apt 的 `libonnxruntime1.23` + `libonnxruntime-dev`（**CPU-only**）。
    **`ort` 的 C API 必须与运行库匹配**：现为 `api-23`（本机 1.23.2 只支持到 API 23；
    之前写 `api-27` 会报 `The requested API version [27] is not available`，之后所有
    模型错误都变成 `Mutex poisoned`）。**这些都是本机配置，不进仓库**，Windows 与 CI
    仍走 `download-binaries`。
  - 跑门禁：`python3 tools/automation/check.py -Gate fmt|clippy|build|test|ui|version|bundle`，
    七道已实测全绿（92 个单测通过）。
- **构建产物位置**（找东西时别找错盘）：Windows 走 `src-tauri/target/`（D 盘，
  一次 `clippy --all-targets` 后 `debug/deps` 能到 15 GB、`debug/incremental` 11 GB，
  后者可随时删、零风险）；WSL 走 `/root/purevox-target/`。
  `cargo tauri build` 约 5 分钟编译 + 4 分钟打包，产物在 `target/release/bundle/nsis/`
  （Windows 只出 exe；Linux 仍在 `bundle/{deb,rpm,appimage}/`）。

---
