# RELEASE — 版本、CI、发版与更新

> 从 `AGENTS.md` §2.6/§2.7 与 `ARCHITECTURE.md` §4.2 拆出。改流水线/版本号/产物先读这里。

6. **发版 tag**：主线 `v<yyyy.MM.dd.HHmm>`。CI 失败、从未生成 release 的 tag 必须删除
   （`git tag -d <tag> && git push origin :refs/tags/<tag>`），否则会截断下一个 release 的提交记录。
   **应用/包版本号 = 年.月.日**：由 `tools/automation/version.py`（唯一实现，跨平台）从 tag
   （或本机当前 UTC）推导。`yy.m.d` **只有三段**：Tauri 要 semver 三段；历史上还要出 MSI，
   MSI 的 ProductVersion 限 major≤255 / minor≤255 / build≤65535（`2026.10.8` 直译会报
   `major number cannot be greater than 255`），故映射为 **major = 年-2000、minor = 月、
   build = 日**（上限 31，远低于 65535）；如 tag `v2026.10.08.1430` → 版本 `26.10.8`。
   **现在 Windows 只发 NSIS exe，但这个三段映射保持不变**（显示版本 / 产物名 / 更新说明
   都按它走）。
   **同日多次发布靠 tag 的时分区分**（tag 仍是完整时间戳 `v2026.10.08.1430`），
   更新检查比较的就是 tag（见 `src/update/`），因此显示版本重复不影响更新判定。
   版本写入 `src-tauri/.build-version.json` 并由 `cargo tauri build --config` 覆盖（不提交该文件）；
   窗口标题与调试面板「版本」走 `PUREVOX_BUILD_VERSION`，比较更新用的时间戳走
   `PUREVOX_BUILD_STAMP`（见 `src/version.rs`，开发态为空 → `is_dev()`）。
7. **CI 与门禁（多平台：Windows + Linux + Android；测试与打包解耦，无自动触发）**：
   - **触发纪律**：**没有任何分支推送 / PR 触发**。`tests.yml`（只测试、不打包）只
     `workflow_dispatch` 手动触发；`release.yml`（打包发版）由 tag `v*` 触发、也可手动。
     日常提交零成本，要验证就手动 dispatch 一次。
   - 门禁唯一实现 = `tools/automation/check.py`（**Python，跨平台唯一**：七道门禁 `fmt` /
     `clippy`（`-D warnings`）/ `build` / `test` / `ui`（`tsc` + `i18n_lint`）/ `version` /
     `bundle`）。选 Python 不选 PowerShell：门禁要跑 Windows / Linux / macOS 三边，pwsh 只有前两边
     装得像样（CI runner 自带、本机还得自己搞），Python 三边都自带，冷启动 ~20 ms vs pwsh ~330 ms，
     且只用标准库、不用 `pip install`。**CI 与本机跑同一份**（`-Gate` 拆 step 便于定位日志）。
     **判门禁只看退出码，而且要确认跑了几道**（启动先打印实际清单）：`-Gate` 可重复、不带 = 全部，
     少写一道不等于通过。`clippy` 那道会先碰 crate 根强制重编 —— cargo 命中缓存（Fresh）时
     **不再重打任何警告**，「绿」于是静默退化成「上次编译时是绿的」（本机缓存几乎必中、
     CI 干净树才暴露）。两条都是 2026-10-18 连挂两轮发版踩出来的。
   - 工作流（`.github/workflows/`）：
     - `tests.yml`：手动 → Rust 门禁矩阵 `windows-latest` + `ubuntu-24.04`
       （fmt/clippy/build/test）、`ui` 门禁（`ubuntu-24.04`：setup-node → `-Gate ui`）。**不打包**。
     - `release.yml`：tag `v*` 或手动 → 三平台并行：
       **Windows**（门禁 + `cargo tauri build` + `assert_bundle.py --smoke` → NSIS exe）；
       **Linux**（`ubuntu-24.04`：装 WebKitGTK/ALSA/rpm 依赖 + 门禁 + `cargo tauri build` +
       `assert_bundle.py --smoke` + `test_packages.py`（deb→ubuntu / rpm→fedora 容器安装验证）→
       deb/rpm/AppImage）；
       **Android**（JDK17 + `install_android_sdk.py` + 从 `gh secret` 还原 keystore →
       `./gradlew assembleRelease` → **已签名的 release APK**）。**凭据一律走环境变量、keystore 一律放
       仓库外面**（`PV_KEYSTORE` / `PV_KEY_ALIAS` / `PV_KEY_PASS`，见 `android/README.md`）——
       仓库里一个凭据文件都没有，所以不存在「忘了 gitignore 把私钥提交上去」。CI 另设
       `PV_REQUIRE_SIGNING=1`：缺 secret **当场失败**，绝不产出未签名的包（Android 不允许安装未签名
       包，发出去无法补救，重打要重发一版）。
       **为什么必须固定密钥**：Android 只允许同一把密钥的包互相覆盖升级。而在此之前
       `assembleDebug` 用的是 debug keystore，CI runner 上根本不存在 `~/.android/debug.keystore`
       —— AGP 每次都**当场生成一个一次性的**。实测：2026-09-30 的 release APK 与 2026-10-15 的
       CI APK **签名者公钥不同**（264 B，同为 RSA-2048，内容却不一样），即**以前每一版都得先卸载
       才能装**。换成固定的 release keystore 之后才第一次有稳定身份。不要退回 `assembleDebug`：
       那等于每次发版都换一次密钥，用户永远只能重装。
       **签名方案逐条写死在 `build.gradle.kts`（不靠 AGP 默认值，默认值会随版本变）**：
       v1 关（minSdk=24 用不上）、v2 开（必需）、v3 开（唯一支持密钥轮换的方案）、
       v4 不做（只多产一个 `.idsig`，零安全收益）。**开了 v3 也不要去轮换密钥** ——
       proof-of-rotation 只有 Android 13+ 认，Android 12 及更早仍用原密钥，而 minSdk=24，
       轮换会把那一大片用户挡在门外。CI 用 `apksigner verify` 断言 v3 真的在 +
       `zipalign -c` 断言已对齐（官方定义的「release-ready APK」是签过名**且**对齐过的）。
       tag 时再 `release_notes.py` 生成说明并 `gh release create` 附带全部产物；手动触发只打包、不建 release。
      **发版那一步的实现是 `tools/automation/release_stage.sh`（不内联在 workflow 的 `run:` 里）**：
      那段逻辑（按扩展名 stage → 统一改名 → 交给 gh）**从未成功执行过**，第一次真打 tag
      就暴露出「收集完文件名后 `cd ..` 回上一层 → 相对路径全部失效 → gh 报
      `no matches found for './PureVox-…apk'`」。**内联的脚本等于没被测过的脚本**。
      落到文件后，`tools/automation/run_release_stage.py` 用假产物树 + 假 `gh` 在本地跑通
      三个场景（齐全 / 缺产物 / 缺 notes），并挂在 `bundle` 门禁里 —— 测的就是发布的同一份文件。
      `update/mod.rs` 的两个单测也从 `release_stage.sh` 读产物名：一个查命名互不重叠，
      另一个查**每个产物名都落在运行期 `ASSET_SUFFIXES` 认的后缀里** —— 后缀对不上不会
      报任何错，只是自动更新静默失效，用户只看到「永远有新版本」。
       **没有 Lite 变体**（遗留资产已删除，见 §2.5）。
   - **缓存纪律（sccache 自动写，不需要手动预热）**：编译走 `mozilla-actions/sccache-action`
     + `SCCACHE_GHA_ENABLED=true`，按**源码/编译器的对象指纹**复用，跨 tag 自动命中、自动更新
     （不会因为换依赖就整桶失效）；`release.yml` 另外在构建后自动 `actions/cache/save`
     cargo registry 与 cargo-tauri（**同键已存在就跳过**，`continue-on-error`）。
     `tests.yml` 仍只 restore（不抢写）。
     - 键为 `purevox-<RUNNER_OS>-<发行版>-<桶>-<CACHE_GEN>-<Cargo.lock 哈希>`，发行版段来自
       `versions.env` 的 `LINUX_CACHE_TAG`（`runs-on:` 写死，静态已知）。**不写 `restore-keys`**：
       精确命中是唯一路径；兜底键会跨发行版回读旧 `target/`。
     - **`target/release` 不再做目录缓存**（太大）；release 的编译复用由 sccache 承担，
       所以「新依赖第一次冷、之后命中」不再依赖手动 `warm-cache.yml`。
     - 键里的 `<Cargo.lock 哈希>` 是 `hashFiles()`，不是文件 sha256 —— 本地算出来对不上很正常。
     - 缓存桶与版本（含 `JDK_VER` / `ANDROID_PLATFORM` / `ANDROID_BUILD_TOOLS`）的唯一来源 =
       `tools/automation/versions.env`（其余 workflow 不得硬编码版本）。
   - **Linux 系统依赖**（本机与 CI 同）：`libwebkit2gtk-4.1-dev libssl-dev libxdo-dev
     librsvg2-dev libgtk-3-dev libasound2-dev pkg-config file rpm`（wry→WebKitGTK、
     cpal→ALSA、rpmbuild→rpm）；运行期还需系统 `libopus0`（缺时网络音频降级，见 §1.2）。
   - `*.md` 改动不进 CI。
  - **Linux 基线 = ubuntu-24.04，打包与门禁都用它（唯一一档，不分双轨）**：
    glibc 的版本化符号**单方向兼容** —— 新 glibc 必定保留全部旧版本标签（实测 2.43 的
    libm 同时带 `log10f@GLIBC_2.2.5` 与 `log10f@@GLIBC_2.43`），所以「老基线编的包在新
    发行版上能跑」成立，**反过来不成立**：在 26.04 上编的包要求 `log10f@GLIBC_2.43`，
    装到 24.04 直接 `symbol lookup error` 起不来（已实测）。**把 runner 跟 ubuntu-latest
    对齐等于单方面砍掉老用户** —— CI 跟最新对齐只是卫生问题，两者的取舍完全相反。
    门禁跑在最新上游上的价值（验没落后）由 `test_packages.py` 的容器档替代：deb 在
    26.04 容器里实测能跑，正是「向前兼容成立」的直接证据（理论不值钱，容器里跑一遍才算）。
    **下限 24.04 不是挑的**：ort 预编译包要求 glibc ≥ 2.39。唯一来源 = `versions.env` 的
    `LINUX_RUNNER`；缓存键的发行版段是 `LINUX_CACHE_TAG=ubuntu24`（与 runner 分开是因为
    键不可变，将来换 runner 时旧桶要靠不同的键段自然失效）。
  - **依赖声明：deb 要手写，rpm 不要手写**（两种格式的机制不同，别当同一件事）：
    - **deb 必须显式列** `bundle.linux.deb.depends`：dpkg **没有**自动依赖机制，Tauri 直接把它
      写进 `Depends:`，**没有内置默认值**（`Option<Vec<String>>` → `unwrap_or_default()` → 空）。
      忘配的后果不是构建报错，而是**发出去一个「装得上、一启动就
      `error while loading shared libraries`」的包**，CI 全绿。门禁 `bundle` 挡「忘配」。
    - **rpm 必须留空**（`bundle.linux.rpm.depends` 不许写，`bundle` 门禁反过来断言它为空）：
      `rpmbuild` 从 ELF 的 `DT_NEEDED` **自动生成** `Requires`，生成的名字经 filelist 反查
      必然正确。手写反而是错的来源 —— 曾把 Fedora 的 `libsoup3` 写成 `soup3`，安装直接
      `nothing provides soup3`，**而且它在第一个错名就停下、后面的错一个都没暴露**。
      自动依赖看不见的只有 dlopen 的东西，opus 因此留在 `recommends`。
      `test_packages.py` 在 Fedora 容器里先把 `rpm -qp --requires` 的输出打进日志，
      免得「自动依赖到底工作没有」只能靠猜。
    - 清单**对不对**由 `test_packages.py` 在干净容器里用 `ld.so --list` 实测（缺库与缺符号
      版本都会报）。**别用 `ldd`**：它只看库文件在不在，不看符号版本够不够，在 glibc 偏老的
      系统上会给假绿（实测 `ldd` 输出干净而程序起不来）。
    - 容器内断言有两条硬约束：**装完整的 `python3`**（`python3-minimal` 的标准库是裁剪过的，
      本脚本要的 `shutil` 直接 `ModuleNotFoundError`），以及**模块级不许算仓库路径**
      （容器里脚本挂在 `/pv/test_packages.py`，`parents[2]` 直接 `IndexError`，三项断言一项
      都跑不到）。三档验证互相独立、一档挂了其余照跑，最后统一汇总。
  - **Linux 产物的可信度排序**：AppImage 最兼容（linuxdeploy 把非系统库打进去）> deb
    （限 Ubuntu 系，基线对齐后 24.04/26.04 都能跑，且容器里双向都验了）> rpm（在 Ubuntu
    上用 rpmbuild 打的，对 Fedora 只当「拦住明显装不上」，别当兼容性证明）。


- **新功能的完成标准**包含调试输出：新增模块（流、设备、模型、效果）必须同时把自身状态
  接入 1.2 节对应端点，否则视为未完成；测量 / 诊断类结果另按 1.4 提供结构化 JSON
  （落盘 + 端点）与可复现命令。
- **验证方式**：智能体与脚本验证运行状态一律请求 HTTP 接口（`curl http://127.0.0.1:<端口>/debug`），
  不靠模拟键鼠点界面（Windows 会拦截后台抢焦点，按键会打进别的窗口）。界面交互验证用
  Windows UI Automation（`TAURI3.md` 3.6 节）；**Linux 用 `tools/automation/ui.py`（AT-SPI）**——
  `list` 列控件、`click <可访问名>` 点按钮/勾选框（要 `DISPLAY=:0 XDG_RUNTIME_DIR=/run/user/1000
  DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus`），不要求人点。同一条链还有会话子命令：
  `start`（起应用并等 `/debug`；会话总线真缺 SNI watcher 时自动顶替身）/ `take '<jq 表达式>'`（读快照）/
  `verify-update <deb|rpm|appimage|none>`（点「更新 → 检查更新」，断言安装方式与**产物后缀对得上**，
  失败返回非 0）/ `stop`（收掉自己起的进程）。前置与跨总线限制见 `TAURI3.md` 3.6 节。
  **网络上下行 / 网页版播放不要靠耳朵听**：`python tools/automation/net_loopback.py`（`--mode speaker`
  测测试音→网络扬声器，`--mode roundtrip` 测网络麦克风→网络扬声器，含上下行 hub 与 10 ms 节拍；
  `--json` 出结构化结果、`--out` 落可听的 WAV）与 `python tools/automation/net_web_probe.py`
  （无头 Chrome 真开网页版，读 AudioWorklet 的欠载/水位，并验关/开后订阅 1→0→1）。
  两者都**隔离启动**（临时 HOME + 自动空闲端口），不碰用户的 `~/.purevox`、不需要手点界面。
  **少截图**，优先用调试接口读数。
  **不许动系统的音量 / 静音**（只读不写；用户音量不是我们该改的状态）。
- **测试自收尾**：任何启动进程的 shell 测试必须自己结束（PowerShell 用
  `try { … } finally { Get-Process purevox -ErrorAction SilentlyContinue | Stop-Process }`），
  命令一律带超时，后台任务不得残留；**不允许把运行中的 app 留给用户手动关闭**。
- **前端调试**：`ui/jsdebug.js` 把脚本错误、未处理的 Promise 拒绝、`console.error/warn` 转发到
  `ui_report`（即快照的 `ui` 数组）；顶栏「开发者工具」打开 WebView2 devtools（Cargo 已开
  `devtools` 特性，release 保留）。JS 静态检查用项目 `opencode.json` 的 LSP + `ui/jsconfig.json`
  （`checkJs`，配合 `ui/globals.d.ts`）。
- **外围**：系统托盘（启动/停止、显示/隐藏、退出；图标随运行状态在 `icons/tray_running.png` /
  `tray_stopped.png` 间切换，菜单随语言与状态重建）。**托盘注册失败不致命**：GNOME 上
  `org.kde.StatusNotifierWatcher` 由 appindicator 扩展提供，扩展没启用时总线上没有这个名字、
  ksni 直接报错 —— 此时只把「托盘不可用 + 原因」写进调试面板并继续跑（引擎与窗口跟托盘无关）；
  原来在这里 `?` 传播 = 启动即 panic，用户连界面都看不到（2026-10-18 实测）；
  **窗口最小化/关闭都收到托盘**（不退出）；**窗口位置/大小记忆**（`winstate.rs`）：Windows 用
  `GetWindowPlacement`/`SetWindowPlacement`（`rcNormalPosition` = 还原外框、`showCmd` = 是否最大化，
  所以最大化时不会把整屏写成用户的还原尺寸），其它平台用 Tauri 的 `outer_position`/`inner_size`；
  存 `settings.json` 的 `window`（**物理像素**），启动时夹进当前显示器（副屏拔掉也不会丢到屏外），
  运行中每 1.5 s 轮询、变了才落盘。主窗口在 `tauri.conf.json` 里是 `visible:false`，
  `setup` 恢复几何之后再 `show()`（否则会先按默认尺寸闪一下再跳位置）；
  开机自启（**官方插件** `tauri-plugin-autostart` 3.0.0-alpha：Windows 上是 `auto-launch`，
  先试 HKLM、非管理员回退 HKCU，**并写 `Explorer\StartupApproved\Run` 的「已启用」标记** ——
  只写 Run 键时任务管理器的「启动」页会当成被禁用，这就是旧版「开关没功能」的真因，
  2026-10-26 实测；不弹 UAC；`set_autostart` 失败会停在设置面板里显示原因）；**启动/停止**（命令 `set_running`/`get_running`，
  界面顶栏按钮（启动绿/停止红）、托盘、全局热键共用；快照带 `running`）；**全局热键**（`hotkey.rs`：
  `RegisterHotKey` + 规范串「`Ctrl+Alt+Shift+Win` 顺序」，空串=不监听，注册结果进 `ui`）；
  **提示音**（`cues.rs` 自合成 6 套预设，start 上行 / stop 下行，`PlaySound` SND_MEMORY）；
  **设置**（`~/.purevox/settings.json`：语言 / 热键 / 提示音；顶栏「设置」面板，键位可录制）；
  中英文（`ui/i18n.js`，中文即 msgid，顶栏「EN/中」）。应用图标 `src-tauri/icons/` 与 Android 的
  `res/mipmap-*` 由 `tools/automation/gen_icons.py` 生成（**不参与 CI**，一次性生成、产物入库；
  跨平台同一条命令、产物逐字节相同；P 的矢量定义与改法见 TAURI3.md §3.5.1）。
  **Android 图标 = 规范自适应图标**（与桌面版同一套 P + 渐变，不另画）：脚本一并写出
  `mipmap-anydpi-v26/{ic_launcher,ic_launcher_round}.xml`（含 `<monochrome>`，供 Android 13+
  主题图标着色），并生成五档密度的 `ic_launcher_foreground.png`（108dp 画布、P 落在 72dp 安全区）、
