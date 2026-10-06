---
name: purevox-ci-release
description: Use when editing .github/workflows/ or cutting a PureVox release — release.yml / release-lite.yml / tests.yml / warm-cache.yml、tag 命名 v<yyyy.MM.dd.HHmm>、release notes 生成与 tag 家族前缀、产物级冒烟、缓存单写者、产物命名、CI 踩坑。这是 AGENTS.md 发版准则的按需展开。
---

# PureVox CI / 发版

> 本文件是 `AGENTS.md`「维护准则」第 6 条与 CI 事务的完整版（拆分出来以便按需加载）。
> 跨模块硬准则始终见 `AGENTS.md`。

### CI（`.github/workflows/release.yml`，精简为通用包 deb/rpm/appimage + 产物目录 + apk）

- **触发方式：push tag 触发 CI 构建 + 自动发 release 两件事**，分支 push 不触发构建（保持日常快速提交零成本）；需验证分支时可 `workflow_dispatch` 手动跑（仅触发三构建 job，不触发 release）。
- **测试工作流独立（`.github/workflows/tests.yml`）**：**每次分支 push / 手动
  触发**，零打包、分钟级——compileall + 引擎冒烟 + `tests/run_all.py` 全套
  （`test_session_plan.py` 会话计划纯函数 / `test_playback_sink.py` 播放
  正确性合成测试 / `test_transport.py` 传输层优雅降级 / `test_devices.py`
  设备面[枚举/虚拟麦克风/配置键]）。运行时 = 内嵌 python312（与发行产物
  同款）；cache **只读复用**共享桶（restore 的 key 与 release.yml 一致，**无
  save 步骤**——落盘仍由 release.yml 单写者负责，勿在此加缓存写入）。构建
  job 里保留同款轻量测试步（守打包环境），另有产物级冒烟（见下）。
- **产物级冒烟（打包流程自检，tag 时运行）**：deb/rpm 解包后用**包内嵌
  python312** 跑引擎冒烟 + `tests/`（依赖缺装/内嵌解释器问题在此暴露；
  fedora job 的 sysdeps 因此含 `cpio`）；AppImage `--appimage-extract`
  免 FUSE 后同样处理（随 AppImage best-effort）；Windows 断言
  `dist/PureVox` 布局关键项（exe / about 三页 / models / opus.dll / html）。
- **tag 命名规则**：`v<yyyy.MM.dd.HHmm>`（如 `v2026.08.10.1517`）。tag 名同时定义产物体内版本（`v` 去掉即 `yyyy.MM.dd.HHmm`），所有 job 的产物时间戳/版本都从 `${GITHUB_REF_NAME}` 推导，避免各 job 并发时刻漂移。回复发版即 `git tag v<yyyy.MM.dd.HHmm> && git push origin <tag>`。
- `linux` job：容器矩阵只留 3 项，产出通用安装包——
  - `ubuntu-22.04`：pip 装最新 numpy/onnxruntime/scipy 等 + 引擎冒烟（加载模型跑一帧）+ `pack_deb.sh` 出 deb + `pack_appimage.sh` 出 AppImage（best-effort，捆绑内嵌 python312）
  - `fedora`：冒烟 + `pack_rpm.sh` 出 rpm
  - `python3.12`：官方 `python:3.12-bullseye`，验证纯 Python 引擎在该基线可导入、可推理
- `windows` job：windows-latest + Python 3.12 + 引擎冒烟；`build_win.ps1`（PyInstaller one-folder）出 `dist/PureVox/`，CI 上传该目录（`actions/upload-artifact` 会自动压缩为 zip，命名 `PureVox-Windows-x64-<yyyy-MM-dd-HHmm>-release`）
- `android` job：ubuntu-latest 编 debug APK（JDK17 + SDK 34 + NDK r27）；下载 opus 源码到 `android/opus-src/`，产物改名 `PureVox-Android-arm64-<yyyy-MM-dd-HHmm>-debug.apk`
- `release` job：`needs` 三构建 job + `if: startsWith(github.ref,'refs/tags/')`，tag push 时下载全部产物，Windows 目录重打成 zip（`zip -9`），`gh release create` 把 deb / rpm / AppImage / Windows zip / APK 全部 attach
- **产物命名统一**：`PureVox-<平台>-<架构>-<yyyy-MM-dd-HHmm>-<release|debug>.<ext>`（Windows 上传目录由 CI 自动压缩 / Linux deb / rpm / AppImage 一律 release，Android 为 debug）。文件名时间戳 `yyyy-MM-dd-HHmm`；产物体内版本字段 = `yyyy.MM.dd.HHmm`（如 `2026.08.10.1517`，deb control / rpm / setup.py 一致，**由 tag 名 `v<yyyy.MM.dd.HHmm>` 推导**，避免并发 job 各自 `date` 导致产物版本不一）。
- **窗口标题版本戳 `_build_version.py` 同样由 tag 推导**：`uitk/main_window.py` 顶部 `try: from _build_version import BUILD_DATE`，缺失回退「开发版」。四个打包脚本（`pack_deb.sh` / `pack_rpm.sh` / `pack_appimage.sh` / `build_win.ps1`）都在打包时把 `BUILD_DATE = "yyyy-MM-dd-HHmm"` 写入产物内的 `_build_version.py`（tag 触发取 `GITHUB_REF_NAME`，本地回退当前时间），保证窗口标题与包版本/文件名同源；该文件已在 `.gitignore`，勿提交。新增打包脚本必须照此生成。
  - **Windows(PyInstaller) 的 `_build_version.py` 无需 `--add-data`（2026-08-12 实测，入口切 run_tk 后仍成立）**：PyInstaller
    静态分析入口顶层的 `from _build_version import BUILD_DATE`，
    会把仓库根的 `_build_version.py` 当作**模块编译进 PYZ**（`dist\PureVox\_internal\` 下看不到
    独立 `.py` 文件，属正常），运行时 import 正常、窗口标题带日期。勿再加 `--add-data="_build_version.py;."`
    ——PYZ 里没有它、且会被 PyInstaller 以模块方式收集，加 add-data 只会让文件重复打包。验证方法：
    启动 `dist\PureVox\PureVox.exe` 后抓窗口标题（Win32 `EnumWindows` + `GetWindowText` 按 PID 过滤）。
    注意：`about/` 目录与此**不同**——uitk 直接 import `about_content.py`（自动进
    PYZ），但三个 markdown 页按文件路径读取，PyInstaller 必须显式
    `--add-data="about;about"`，否则关于页手册/日志缺失（build_win.ps1 已带）。
- **onnxruntime 走 pip 最新版（2026-08-22 纯 py 迁移）**：不再捆绑预编译 C SDK；
  `requirements-win.txt` / `requirements-linux.txt` 不锁版本，CI/全新环境安装即最新。模型 opset ≤18，
  onnxruntime 长期向后兼容
- **外部下载全部预置化（2026-08-22）**：`server/opus.dll`（预编译 libopus，BSD）
  直接提交进仓库（`.gitignore` 对其白名单），CI 与本地开发均不再下载；
  Linux 内嵌 python312 编译产物与 appimagetool 二进制走 `actions/cache`
  （key 分别为固定 cpython 版本号与固定版本）；Android opus 源码 zip 同样缓存。
  本地开发 `pack_appimage.sh` 复用 `~/.cache/purevox/appimagetool`，也可用
  `PUREVOX_APPIMAGETOOL` 环境变量指定
- **CI 缓存单写者 + 作用域门控（2026-08-27）**：GitHub 缓存作用域 = 触发 ref，
  tag 触发的 save 只进 tag 作用域且未来 tag 永远读不到——纯垃圾副本。故全部
  缓存统一「显式 restore 共享 + save 仅在 main 分支（手动 dispatch）落盘」：
  Linux `~/.cache/purevox` 与 Windows pip 轮子桶（`purevox-windows-pip-v1-`
  前缀 + requirements hash 键）均如此；Lite 工作流纯 restore-only。依赖变更
  后需在 main 上手动 dispatch 一次 release.yml 暖桶，之后的 tag 全命中
- **Linux 的 opus**：opuslib 经 `ctypes.util.find_library('opus')` 加载**系统**
  libopus——deb Depends 带 `libopus0`、rpm Requires 带 `opus`；AppImage 从构建机
  拷贝 `libopus.so*` 进包并经 AppRun 注入 `LD_LIBRARY_PATH`。缺库时
  `opus_codec.OPUS_AVAILABLE=False` 优雅降级（仅网络推流解码不可用），主程序正常
- **Linux job 按发行版分开是刻意设计，勿合并成一个 job**（2026-08-10 决策）：deb 在
  Ubuntu、rpm 在 Fedora 产出，是因为 rpm 打包须依赖 `rpmbuild` 与真实 Fedora 包名解析，
  移到 Ubuntu 上构建可靠性下降；分开还有并行收益与故障隔离
  （一个发行版坏不掉其他产物）。AppImage 在 ubuntu job 内（best-effort，
  `continue-on-error: true`），捆绑内嵌 python312——该 job 需先装 `libssl-dev`
  （否则编译出的 CPython 无 ssl 模块，pip 无网络，`bootstrap_python312.sh` 失败）
  与 `file`（appimagetool 打包必需），并确保 `PyAudio` 不在 Linux 依赖里
  （`requirements-linux.txt` 不含 PyAudio，Windows 专用包只在 `requirements-win.txt`）。
- **CI 踩坑（实测细节补充，避免重踩）**：
  - 容器 job 在 checkout 前先装系统依赖（含 `git`）——REST API 下载不支持 submodules；
    cpython 子模块已移除（2026-08-23），bootstrap 按需下载 tarball
  - appimagetool 容器无 FUSE → 用 `--appimage-extract-and-run`；`.desktop` 要在
    AppDir 根目录放一份；图标用 `assets/icons/audio_icon_base.png`
    直接生成 256/512 png
  - `pack_deb.sh` 末尾 `| head` 会 SIGPIPE(141) 使 `sh -e` 退出 → 补 `|| true`
  - Ubuntu 容器 pip 装 pillow 遇到匹配版本时用 `--break-system-packages` 兜底
    （`||` 回退普通安装），不再 `pip install --upgrade`
  - Android JNI `CMakeLists.txt` 注释必须用 `#`（CMake 不认 `//`）；`gradlew`
    无执行位，构建前 `chmod +x`
  - Windows pwsh 无 `\` 行继续符
  - **`android-actions/setup-android` 的 `packages` 必须显式给**（2026-09-15）：
    默认包列表含 `tools`，Google 已从 SDK 仓库移除它，`sdkmanager tools` 直接
    exit 1（`Failed to find package 'tools'`），Android job 在建 APK 前就挂。
    现配置 `packages: platform-tools`，platform-34/build-tools/NDK 由
    `tools/automation/install_android_sdk.sh` 负责

### release notes 与 tag 家族（单一实现路径）

- release 描述由 `tools/automation/release_notes.sh` 生成（`release.yml` 与
  `release-lite.yml` **都调这一份**，勿再内联第二套 `git log` 逻辑）。
- 脚本用 `git describe --tags --abbrev=0 --match <前缀> "${TAG}^"` 取「上一个 tag」，
  前缀按家族推导：主线 tag（`v...`）→ `--match 'v*'`；Lite tag（`lite-v...`）→
  `--match 'lite-v*'`。**不带 `--match` 时，最近的任意家族 tag 都会赢**，会把一个
  release 的提交记录截断到只剩 1 条（2026-09-15 实测：`v2026.09.15.1605` 被同分支上的
  `lite-v2026.09.15.1601` 遮蔽）。
- **CI 失败、从未生成 release 的 tag 必须删除**（`git tag -d <tag> &&
  git push origin :refs/tags/<tag>`）：它仍留在 git 里，会成为同家族下一个 tag 的
  「上一个 tag」，同样截断提交记录（2026-09-15：失败的 `v2026.09.15.1532` 即此类）。
- 描述修正可回填：`gh release edit <tag> --notes-file <file>`。
