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

包名 `com.purevox.mic`，应用名 PureVox。

## 协议（与 `src-tauri/src/net/proto.rs` 一一对应）

| 项 | 值 |
| --- | --- |
| 传输 | 明文 `ws://`，端口 **59123**，路径 **`/ws`** |
| 自检 | `GET http://<host>:59123/health`（不需要连上 ws） |
| 握手 | 客户端 → `{"t":"hello","proto":1}`；电脑回 `{"t":"ready",…}` |
| 音频 | **二进制帧 = 一个 Opus 包（无头）**，48 kHz 单声道 |
| 控制 | 文本帧 = JSON，判别字段 `t` |

客户端 → 电脑：`hello` / `sub` / `text` / `key` / `ping` / `rtt`。
电脑 → 客户端：`ready` / `pong` / `err`（`err` 原文一律显示给用户，不静默）。

**收到 `ready` 之前不推流、不播放。** 远程输入在电脑端默认关闭，未开启时发
`key` / `text` 会收到 `err`，本客户端把它显示在状态区的红字行。

## 代码结构（每个文件的职责在文件头注释里）

```
app/src/main/java/com/purevox/mic/
├── Proto.kt                     协议常量与 JSON 拼装 / 解析（proto.rs 的镜像）
├── MainActivity.kt              单界面：连接、三条能力编排、实体键拦截、IME 文本同步、状态读数
├── net/
│   ├── WsClient.kt              WebSocket 连接、hello/ping/rtt、二进制包收发、RTT 计算
│   └── HealthProbe.kt           GET /health 自检（不依赖 ws 已连上）
├── audio/
│   ├── OpusSupport.kt           audio/opus 能力探测（编解码器名 + 输入 PCM 格式）
│   ├── OpusEncoder.kt           上行编码（输出驱动，帧长不假设 10 ms）
│   ├── OpusDecoder.kt           下行解码（输出帧长按实际样本数，不假设）
│   ├── MicUplink.kt             AudioRecord 采集 + 编码线程
│   └── SpeakerDownlink.kt       包队列 + 解码线程 + 样本环形缓冲 + AudioTrack 写入线程
└── service/
    └── CaptureService.kt        麦克风前台服务（通知 + WakeLock，Android 14+ microphone 类型）
```

## 没有的东西（刻意的）

* **没有 JNI / CMake / NDK / C 代码**：Opus 编解码全部走系统 `MediaCodec`（`audio/opus`）。
* **没有 TLS 与证书信任绕过**：新版协议就是明文 `ws://` + `usesCleartextTraffic="true"`。

## 构建

```bash
cd android
./gradlew :app:assembleDebug      # 产物 app/build/outputs/apk/debug/app-debug.apk
./gradlew :app:assembleRelease
```

需要 JDK 17 与 Android SDK（`compileSdk 34`）。`gradle/wrapper/gradle-wrapper.jar` 已入库，
不要替换；`gradle-wrapper.properties` 钉的是 Gradle 8.9（AGP 8.7.3 的最低要求）。

## 依赖 Android 版本的行为

| 行为 | 起始版本 |
| --- | --- |
| `AudioRecord` / `AudioTrack` 的 `ENCODING_PCM_FLOAT` | API 23（minSdk 24 覆盖） |
| `MediaFormat` 的 `opus-frame-duration-us` | API 29；不认这个 key 时自动降级 |
| `startForeground(…, FOREGROUND_SERVICE_TYPE_MICROPHONE)` | API 29；24~28 用旧的双参重载 |
| `setPerformanceMode(LOW_LATENCY)` | API 26；更低版本跳过 |
| `POST_NOTIFICATIONS` 运行时申请 | API 33 |