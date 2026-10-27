<!--
  PureVox — AI 麦克风降噪工具
  Copyright (C) 2024-2026 a2heng <752848283@qq.com>

  PureVox is licensed under the GNU General Public License v3.0 or
  later (GPL-3.0-or-later).  See LICENSE for details.

  The built-in AI models are NOT covered by the GPL; they are the
  property of a2heng and may only be used with PureVox under
  authorization.  See MODEL-LICENSE.md for details.

  SPDX-License-Identifier: GPL-3.0-or-later
-->

# PureVox Android 客户端

手机 ⇄ 电脑的第二端：**手机麦克风当电脑麦克风**、**手机扬声器当电脑音箱**、
**手机键盘当电脑全尺寸键盘**。三条能力共用电脑端的一个 WebSocket 连接。

界面是**卡片式**（连接 / 功能 / 远程输入 / 后台保活 / 更新），底部「诊断」读数与日志
**默认收起**、点标题行展开；客户端能**自动发现**电脑，也能**检查并安装自己的更新**。

包名 `com.purevox.mic`，应用名 PureVox。

## 协议（与 `src-tauri/src/net/proto.rs` 一一对应）

| 项 | 值 |
| --- | --- |
| 传输 | **TLS（`wss://`）**，默认端口 **59123**，路径 **`/ws`** |
| 证书 | 电脑**自签**（CA + 叶证书两级，存 `~/.purevox/ca/`）；手机系统信任库里没有 → 见「传输与证书」 |
| 发现 | 电脑端 mDNS 广播 **`_purevox._tcp.local.`**（随服务启停），客户端用系统 `NsdManager` 找 |
| 自检 | `GET https://<host>:59123/health`（不需要连上 ws） |
| 握手 | 客户端 → `{"t":"hello","proto":3}`；电脑回 `{"t":"ready",…}`（含 `header_bytes`/`target_ms`） |
| 音频 | **二进制帧 = 一个 Opus 包（无头）**，48 kHz 单声道，帧长固定 **10 ms（480 样本）** |
| 控制 | 文本帧 = JSON，判别字段 `t` |

端口 `59123` 是**电脑端的默认值**（`proto::PORT`），可以在电脑端界面改并持久化到
`~/.purevox/settings.json`；本客户端按默认端口连。

客户端 → 电脑：`hello` / `sub` / `text` / `key` / `ping` / `rtt`。
电脑 → 客户端：`ready` / `pong` / `err`（`err` 原文一律显示给用户，不静默）。

**收到 `ready` 之前不推流、不播放。** 远程输入在电脑端默认关闭，未开启时发
`key` / `text` 会收到 `err`，本客户端把它显示在状态区的红字行。

## 代码结构（每个文件的职责在文件头注释里）

```
app/src/main/java/com/purevox/mic/
├── Proto.kt                     协议常量与 JSON 拼装 / 解析（proto.rs 的镜像）
├── MainActivity.kt              唯一界面：卡片编排、连接与三条能力的状态机、通知按钮广播、更新流程
├── PvEditText.kt                输入框（包一层 InputConnectionWrapper）：空框里按删除键转发给电脑
├── net/
│   ├── WsClient.kt              WebSocket 连接、hello/ping/rtt、二进制包收发、RTT 计算
│   ├── HealthProbe.kt           GET /health 自检（不依赖 ws 已连上）
│   ├── Discovery.kt             局域网自动发现（NsdManager 找 `_purevox._tcp`）
│   └── LocalTls.kt              自签证书的放行范围（只对私有网段放宽，见下）
├── audio/
│   ├── OpusNative.kt            内置 libopus 1.5.2 的 JNI 绑定（加载 `libpvopus.so`）
│   ├── OpusSupport.kt           协议侧的固定东西：帧长、PCM 格式名、状态行文字
│   ├── OpusEncoder.kt           上行编码（帧长自己定：固定 10 ms = 480 样本一包）
│   ├── OpusDecoder.kt           下行解码（输出帧长按实际样本数，不假设）
│   ├── MicUplink.kt             AudioRecord 采集 + 编码线程
│   └── SpeakerDownlink.kt       收包即解码 → 播放环 → AudioTrack 阻塞写（唯一主时钟）
│   └── Rt.kt                    播放环（固定 20 ms，与 src-tauri/src/net/rt.rs 逐行镜像）
├── service/
│   └── CaptureService.kt        会话前台服务（常驻通知 + WakeLock；通知按钮发广播回界面）
└── update/
    ├── UpdateChecker.kt         查本仓库 GitHub Releases，比 versionCode 判新旧
    ├── UpdateDownloader.kt      下载 `.apk` 到应用私有外部目录的 `update/`
    └── UpdateInstaller.kt       经 FileProvider 把安装包交给系统安装器
```

Native 侧（**Gradle 不编译**，只是入库产物 + 备查源码）：

```
app/src/main/cpp/                重建 `libpvopus.so` 的源码（`pv_opus.cpp` 的 JNI 转发层）
app/src/main/jniLibs/arm64-v8a/libpvopus.so    libopus 1.5.2 + 转发层，预编译入库
```

## 音频链路（与电脑端同一套规则，NETWORK.md §4.1）

```text
电脑 → 手机：WS 收包（8 字节帧头）→ Opus 解码 → 播放环（固定 20 ms）→ AudioTrack 阻塞写 → 扬声器
手机 → 电脑：AudioRecord 480 样本阻塞读 → Opus 编码 → 加帧头（seq + 采集时刻）→ WS 发送
```

- **只有一处缓冲**：`audio/Rt.kt` 的播放环。深度是**常量 20 ms**，运行期只有两种动作：
  超了丢一格（10 ms）、空了补一格（Opus **PLC**，还空就淡出重复上一格）。**不做重采样、
  不做自适应、不按抖动调深度** —— 实测晶振差只有 ±25 ppm、局域网抖动 <1 ms。
- **写 AudioTrack 的线程就是主时钟**：阻塞写自然按设备时钟推进，没有第二个定时器。
- **时钟偏差是量出来的**：`SpeakerDownlink.clockPpm` 用 `getPlaybackHeadPosition()`
  （硬件实际播出的帧数）对 `System.nanoTime()` 差分，2 Hz 随 `st` 上报给电脑，
  `/debug` 的 `net.client.clock_ppm` 直接可读。
- **`TCP_NODELAY`**：`net/WsClient.kt` 用自定义 `SocketFactory` 交出已设 `tcpNoDelay` 的
  socket（10 ms 一包、几十字节，Nagle 会把它们攒成串）。

## 没有的东西（刻意的）

* **Gradle 构建不跑 native 编译**：Opus 用的是**预编译入库**的
  `jniLibs/arm64-v8a/libpvopus.so`（内置 libopus 1.5.2 + `cpp/pv_opus.cpp` 的 JNI 转发层），
  所以 `./gradlew` **不需要 NDK / CMake**，CI 也不用装。要重建那个 `.so` 才需要 NDK，
  走 `tools/automation/build_opus_android.py`（一次性工具，产物入库）。
* **只有 `arm64-v8a` 一个 ABI**：主流手机都是它。其它 ABI 上没有 `.so`，Opus 不可用，
  状态栏会写明原因（**不静默降级**）。
* **不用系统 `MediaCodec` 做 Opus**：Android 上**从来没有 `audio/opus` 这个 mime**，系统
  Opus codec 注册的是 `audio/raw` / `audio/ogg;codecs=opus` / `audio/webm;codecs=opus`，
  后两者只认容器封装，喂裸 Opus 包会**照单全收并输出垃圾 PCM（满屏刺啦且不报错）**。
  想吃裸包就得自己实现 Ogg / WebM 封装，那是拿「省一个依赖」换「自己维护一个协议」。
  （`audio/OpusSupport.kt` 里那句「探测」因此**只剩诊断**，不是第二条实现路径。）
* **不做「无条件信任所有证书」**：见下节 —— 只对私有网段放宽。

## 传输与证书（NETWORK.md §4.1.12）

电脑端**只监听 HTTPS**，证书是**每台机器自己签的**（CA + 叶证书两级，存 `~/.purevox/ca/`）。

* **为什么必须 TLS**：网页版客户端的麦克风靠 `getUserMedia`，它**只在安全上下文里可用** ——
  `http://192.168.x.x` 不是安全上下文，浏览器直接拒绝，**没有任何绕过办法**。
* **本客户端怎么做**（`net/LocalTls.kt`）：只对**私有网段 / 环回 / 链路本地**地址放宽校验
  （这些地址公网不可路由，威胁模型与「局域网无鉴权」一致）；**公网地址与域名一律走系统默认校验**。
* **想去掉证书警告**：在手机上打开 `https://<电脑IP>:59123/ca-cert` 下载 CA，再走系统
  「安装证书 → CA 证书」（Android 只接受 `basicConstraints: CA:TRUE` 的证书，所以电脑端签的是
  CA + 叶两级，不是一张自签）。不想装也行，网页版点一次「继续」即可。

## 自动发现（不用抄 IP）

电脑端服务一起来就把自己广播成 mDNS **`_purevox._tcp.local.`**（随服务启停，TXT 只带
`v=<显示版本>` 与 `proto=<协议号>`）。客户端用系统自带的 `NsdManager`（零依赖、零权限）找它，
找到就直接连 —— **没有「选哪台电脑」的下拉**：局域网里就一台电脑，用户想连的是**正在跑服务**
的那台。

* **服务没开就扫不到**，这是预期行为，不是二维码坏了。
* 「**重新发现**」与「**重新连接**」是**两件事、两个按钮**：换网络才需要重新发现；只是掉线、
  服务还在时，重新连接立刻就好。
* 组播被交换机挡了、开了 AP 隔离时，退回手工填电脑 IP（或直接粘电脑端给的网址）。

## 后台保活（`service/CaptureService.kt`）

**服务范围 = 整个会话**：只要「已连接」或「正在推流」就挂常驻通知（不是只有麦克风在用才挂）。
不做前台服务的话，Android 9+ 后台拿不到麦克风、Android 12+ 还会被系统直接掐掉。

* 前台服务类型：麦克风在用时 `microphone|dataSync`，只是保活连接时 `dataSync` ——
  **哪个在用就报哪个**（Android 14 起按实际用途校验，不能超报）。
* 通知带两个按钮「**停止推流**」「**断开连接**」，它们发**广播**回界面（`MainActivity` 的
  `notifReceiver`）—— 服务本身**不认识协议**，界面是唯一的编排者。
* 「后台保活」卡片里另有「忽略电池优化设置」入口
  （`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`），并常驻显示当前是否已忽略。

## 应用内更新（`update/`）

**手动**触发（不自动检查：一打开就弹更新对低频工具是打扰），数据源就是本仓库的
GitHub Releases，**与电脑端更新检查同一份数据**。

* **比较依据是 tag 的完整时间戳**映射出的 `yyMMddHHmm ÷ 10`
  （`v2026.10.17.2359` → `261017235`，10 分钟粒度）与安装包 `versionCode` 比整数 ——
  与 `tools/automation/version.py` 同一套算法。显示版本 `yy.M.d` 同日会重复，比它没用。
* **`÷ 10` 不能省**：`versionCode` 是 **Int32**（上限 2147483647），十位数的 `2610172359`
  装不进去。溢出的表现是**整条链静默失效** —— Gradle 侧 `toIntOrNull()` 返回 null 后
  回退成 `versionCode = 1`，应用侧 `stampToCode` 同样返回 null 报「没有可用的 APK 资产」。
  2026-10-17 的 CI 包就是这么来的（实测 `versionName=26.10.17` / `versionCode=1`）。
  现在两侧都按 ÷10 算，且 `build.gradle.kts` 里**传了 `-PpvVersionCode` 就必须解析成功**，
  否则构建当场失败（不再静默回退）。
* **开发构建（`versionCode = 1`）不下结论**，只报「开发构建」。注意 2026-10-17 及更早的 CI
  包也是 1（上面的溢出），那批包会一直报「开发构建」，只能手动装下一版。
* 产物按 **`.apk` 后缀**认领（不看版本号）；下载落应用私有外部目录的 `update/`，经
  `FileProvider`（`res/xml/file_paths.xml`）交系统安装器，需 `REQUEST_INSTALL_PACKAGES`。
* 三步状态机 **检查 → 下载 → 安装**，界面按状态卡按钮，**不许跳步**。

## 输入：打字同步 / 长文本 / 退格（`PvEditText.kt`）

一个输入框，两种用法：

* **打字同步**（开关）：每敲一次就转发并**清空**本地框，观感像「直接打在电脑上」。
* **长文本**：内容留在本地，按「**发送**」把攒好的一整段一次性发出（同一个 `text` 消息，
  只是时机不同）；「**清空**」只清本地。
* **退格**：`PvEditText` 包了一层 `InputConnectionWrapper` —— **框里空着的时候按删除键**，
  把 Android `KEYCODE_DEL` 转发给电脑（映射成 PC Backspace），用来改电脑端已经打错的字；
  框里有字时就是普通的本地删除。
* 未连接、或电脑端的「远程输入 / 打字同步」没开时会明确提示，**不静默丢弃**。

## 图标

`res/mipmap-*` 与桌面版**同一套 P + 渐变**，由 `tools/automation/gen_icons.py` 生成
（产物入库、**不参与 CI**）：五档密度的传统 `ic_launcher{,_round}.png`（API < 26 用）、
`ic_launcher_foreground.png`（108dp 画布、P 落在 72dp 安全区）、`ic_launcher_monochrome.png`
（单色剪影），以及 `mipmap-anydpi-v26/{ic_launcher,ic_launcher_round}.xml`（背景 + 前景 +
`<monochrome>`，供 Android 13+ 主题图标着色）。**改图标一律改脚本再重跑**，别手改 PNG / XML。

## 构建

```bash
cd android
./gradlew :app:assembleDebug      # 产物 …/apk/debug/PureVox-1.0-debug-android-arm64.apk
./gradlew :app:assembleRelease    # 产物 …/apk/release/PureVox-<版本>-android-arm64.apk
```

产物名由 `app/build.gradle.kts` 的「APK 文件名」段统一成与 CI 相同的形状
（`release_stage.sh` 发出去的也是 `PureVox-<版本>-android-arm64.apk`）——**debug 的
`-debug` 段来自 `versionNameSuffix`**，不是文件名里另拼的。别把这条当成可有可无的整洁癖：
`app-debug.apk` / `app-release.apk` 只差一个词，手机上两只应用并存时基本靠猜。

需要 JDK 17 与 Android SDK（`compileSdk 34`）——**不需要 NDK / CMake**（Opus 是预编译入库的
`.so`，见上）。`gradle/wrapper/gradle-wrapper.jar` 已入库，不要替换；
`gradle-wrapper.properties` 钉的是 Gradle 8.9（AGP 8.7.3 的最低要求）。

### debug 包与 CI 的 release 包共存

调试包**不覆盖**线上包，两者可以同时装在手机上：

| | debug（本机 `assembleDebug`） | release（CI 发出去的） |
| --- | --- | --- |
| 包名 | `com.purevox.mic.debug` | `com.purevox.mic` |
| 签名 | AGP 自动生成的 debug keystore | `PV_KEYSTORE` 里的固定密钥 |
| 应用名 | **PureVox Debug** | PureVox |
| APK 文件名 | `PureVox-<版本>-debug-android-arm64.apk` | `PureVox-<版本>-android-arm64.apk` |
| 偏好 / 数据 | 独立（互不污染） | 独立 |

**为什么必须换包名**：Android 只允许**同签名**的包覆盖安装，com.purevox.mic 的 debug 包与
release 包签名必然不同 —— 包名相同时系统直接拒绝（「应用未安装 / 签名不一致」），于是每次
调试都得先卸载手机上正在用的 release，装回来又得重新下一次。加 `.debug` 后缀让它们成为
两个独立应用，各自安装/升级互不影响。

跟着变的东西都不用单独配置：`packageName` 全是运行期取的（设置页的 intent、
`FileProvider.getUriForFile(ctx, "${ctx.packageName}.fileprovider")` 都自动对），manifest 里
FileProvider 的 authority 写的是 `${applicationId}.fileprovider`。

**调试包点「检查更新」仍会下载 release 的 APK**（更新源就是本仓库的 Release 列表），装上去
是**加了第二只应用**而不是覆盖自己 —— 这是刻意保留的真实链路，用来在调试包里验下载/安装流程。

CI 打包时会传 `-PpvVersion=<年.月.日>` 与 `-PpvVersionCode=<ACODE>`（见
`tools/automation/version.py`）：**应用内更新靠 `versionCode` 判新旧**，本机不带参数就退回
`versionCode = 1` + `versionName = "1.0"`（于是应用内更新会报「开发构建」）。传了参数却解析
不出来（非整数、越界）**构建直接失败** —— 版本号错了要等用户点「检查更新」才暴露，宁可红。

### release 签名

**凭据一律走环境变量、keystore 放在仓库外面**，仓库里没有 `keystore.properties` 也没有
`*.jks` —— 于是不存在「忘了写 .gitignore 把私钥提交上去」这种坑。

| 变量 | 内容 |
| --- | --- |
| `PV_KEYSTORE` | keystore 文件的**绝对路径**（PKCS12） |
| `PV_KEY_ALIAS` | 密钥别名 |
| `PV_KEY_PASS` | keystore 口令与私钥口令（同一个） |
| `PV_REQUIRE_SIGNING` | `1` = 缺凭据**直接失败**；不设 = 产出未签名的包 |

```bash
export PV_KEYSTORE="$HOME/.purevox/signing/purevox-release.p12"
export PV_KEY_ALIAS=purevox
export PV_KEY_PASS='…'
./gradlew :app:assembleRelease
```

### 签名方案（逐条写死，不靠 AGP 默认值）

| 方案 | 取值 | 依据 |
| --- | --- | --- |
| v1（JAR） | **关** | 只对 `minSdk < 24` 有意义；本项目 minSdk=24，开着只是白搭一份 `META-INF` |
| v2 | **开** | 必需项。整包哈希，比 v1 的逐条目哈希更难伪造 |
| v3 | **开** | 目前唯一带密钥轮换能力的方案（OWASP：「至少 v2 或 v3，想要轮换就用 v3」） |
| v4 | 不做 | 只产出独立的 `.idsig`（供 adb 增量安装），自身零安全收益，还多一个附件 |

密钥本身：PKCS12（JKS 已过时）、RSA-4096（≥ 官方要求的 2048）、有效期 30 年
（官方建议 ≥ 25 年，Play 额外要求有效期晚于 2033-10-22）。

**开了 v3 ≠ 应该去轮换密钥。** proof-of-rotation 只有 **Android 13+（API 33）**认，
Android 12 及更早会忽略它继续用原密钥 —— 本项目 `minSdk = 24`，一旦轮换，
那一大片用户会因为新密钥不被认识而**装不上**。所以正确做法是：开 v3 当保险，**把密钥守住**。

**没设凭据时 `assembleRelease` 仍能出包，但那是未签名的 —— Android 不允许安装未签名包。**
本机调试无所谓，CI 一律设 `PV_REQUIRE_SIGNING=1`，缺 secret 就当场红，不产出装不上的版本
（发出去无法补救，重打要重发一版）。CI 的三个 secret 与生成方式见
`~/.purevox/signing/keystore-info.txt`。

## 依赖 Android 版本的行为

| 行为 | 起始版本 |
| --- | --- |
| `AudioRecord` / `AudioTrack` 的 `ENCODING_PCM_FLOAT` | API 23（minSdk 24 覆盖） |
| 自适应图标（`mipmap-anydpi-v26`） | API 26 |
| `REQUEST_INSTALL_PACKAGES`（应用内更新装包） | API 26 |
| `startForeground(…, FOREGROUND_SERVICE_TYPE_*)` | API 29；24~28 用旧的双参重载 |
| `POST_NOTIFICATIONS` 运行时申请 | API 33 |
| `<monochrome>` 主题图标 | API 33 |
| `FOREGROUND_SERVICE_MICROPHONE` / `_DATA_SYNC` 权限与类型校验 | API 34 |
| `ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` 需用户确认 | API 23 起（各版本弹窗措辞不同） |