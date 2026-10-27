// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// The built-in AI models are NOT covered by the GPL; they are the
// property of a2heng and may only be used with PureVox under
// authorization.  See MODEL-LICENSE.md for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

// :app —— 唯一的模块。**没有** externalNativeBuild：Opus 用的是预编译入库的
// `src/main/jniLibs/arm64-v8a/libpvopus.so`（libopus 1.5.2 + `cpp/pv_opus.cpp` 的 JNI
// 转发层），所以 Gradle 构建**不需要 NDK / CMake**，CI 也不用装。重建那个 .so 用
// `tools/automation/build_opus_android.py`。
//
// 为什么不用系统 MediaCodec：Android 上从来没有 `audio/opus` 这个 mime，系统 Opus codec
// 注册的是 `audio/raw` / `audio/ogg;codecs=opus` / `audio/webm;codecs=opus`，后两者只认
// 容器封装，喂裸 Opus 包会输出垃圾 PCM（听感满屏刺啦且不报错）。

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// ── release 签名 ──────────────────────────────────────────────────────────────
// **凭据一律走环境变量、keystore 一律放在仓库外面**，仓库里一个凭据文件都没有 ——
// 于是不存在「忘了写 .gitignore 而把私钥提交上去」这种坑。三个变量：
//
//   PV_KEYSTORE    keystore 文件的**绝对路径**
//   PV_KEY_ALIAS   密钥别名
//   PV_KEY_PASS    keystore 口令与私钥口令（本项目用同一个，keytool 默认如此）
//
// CI 从 `gh secret` 注入（base64 还原到 $RUNNER_TEMP，见 .github/workflows/release.yml）；
// 本机用 `~/.purevox/signing/` 里的那份，见 AGENTS.md §2.6。
//
// **签名缺失分两种处理**，不要混为一谈：
//   · 本机 `assembleRelease` 且没有凭据 → 出**未签名** APK，构建能过（方便本地看效果，
//     但装不上 —— Android 不允许装未签名包，这是平台规则不是我们拦的）。
//   · CI（`PV_REQUIRE_SIGNING=1`）且没有凭据 → **直接失败**，绝不产出未签名的 release
//     产物。发出去的包用户装不上、而且事后无法补救（重打要重发一版），宁可当场红。
fun pvSign(env: String): String? =
    System.getenv(env)?.trim()?.takeIf { it.isNotEmpty() }

val pvStoreFile = pvSign("PV_KEYSTORE")
val pvKeyAlias = pvSign("PV_KEY_ALIAS")
val pvKeyPass = pvSign("PV_KEY_PASS")
val pvHasSigning = listOf(pvStoreFile, pvKeyAlias, pvKeyPass).all { it != null }
val pvRequireSigning = pvSign("PV_REQUIRE_SIGNING") == "1"

if (pvRequireSigning && !pvHasSigning) {
    // 逐个报缺哪个，别只说「签名失败」—— 少了 secrets 与拼错变量名长得一样
    val miss = listOf(
        "PV_KEYSTORE" to pvStoreFile,
        "PV_KEY_ALIAS" to pvKeyAlias,
        "PV_KEY_PASS" to pvKeyPass,
    ).filter { it.second == null }.map { it.first }
    throw GradleException(
        "要求签名但缺这些环境变量：${miss.joinToString("、")}。" +
            "没有它们会产出**装不上的**未签名 release 包（Android 不允许装未签名包），" +
            "所以这里直接失败而不是打个警告继续。"
    )
}
if (!pvHasSigning) {
    logger.lifecycle(
        "[PureVox] 没有 PV_KEYSTORE/PV_KEY_ALIAS/PV_KEY_PASS → release 包未签名（仅供本地）"
    )
}

// ── versionCode ──────────────────────────────────────────────────────────────
// **传了参数就必须解析成功**。这里曾经是 `(findProperty("pvVersionCode") as String?)
// ?.toIntOrNull() ?: 1`，而 `ACODE` 当时是十位数的 `yyMMddHHmm`（2610172359 > Int32 上限
// 2147483647）—— 溢出成 null、悄悄回退成 1，CI 全绿，用户装上的 APK 却永远报「开发构建
// （没有版本时间戳），无法比较新旧」（2026-10-17 实测）。版本号错了不会自己浮出来，要等
// 用户点「检查更新」才发现，所以宁可构建当场失败。不传参数 = 本机构建，才退回 1。
val PV_VERSION_CODE_MAX = 2_100_000_000 // Play 的硬限值，也确保 Int32 装得下

val pvVersionCode: Int = (findProperty("pvVersionCode") as String?)?.let { raw ->
    val n = raw.trim().toIntOrNull()
    if (n == null || n !in 1..PV_VERSION_CODE_MAX) {
        throw GradleException(
            "pvVersionCode 必须是 1..$PV_VERSION_CODE_MAX 的整数，收到 '$raw'。" +
                "（CI 传的是 tools/automation/version.py 的 ACODE，见该文件注释）"
        )
    }
    n
} ?: 1

android {
    namespace = "com.purevox.mic"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.purevox.mic"
        minSdk = 24
        targetSdk = 34
        // versionCode 必须**单调递增**才能让应用内更新检查判出新旧，故取 `yyMMddHHmm ÷ 10`
        // （如 261008143，10 分钟粒度、Int32 内）。由 CI 传入（`-PpvVersionCode=`，见
        // tools/automation/version.py 的 ACODE）；本机不带参数 → 1。解析在 android 块上方。
        versionCode = pvVersionCode
        // 版本名 = 年.月.日，由 CI 传入（`-PpvVersion=`，见 tools/automation/version.py）；本机默认 1.0。
        versionName = (findProperty("pvVersion") as String?) ?: "1.0"
    }

    signingConfigs {
        if (pvHasSigning) {
            create("release") {
                // 用 rootProject 而不是 file()：这里最近的 receiver 是 ApkSigningConfig，
                // 靠脚本作用域的 Project 才解析得到 —— 写明确点，免得哪天作用域一变就找不到
                storeFile = rootProject.file(pvStoreFile!!)
                keyAlias = pvKeyAlias
                storePassword = pvKeyPass
                keyPassword = pvKeyPass

                // ── 签名方案：逐条写死，不靠 AGP 默认值 ──────────────────────────────
                // 理由：默认值会随 AGP 版本变，而「这个包到底能不能在用户手机上装上」
                // 完全取决于它。写死之后改动是显式的，CI 的 apksigner 断言能对上。
                //
                // v1（JAR 签名）**只在 minSdk < 24 时才需要**（Android 7.0 之前不认 v2+）。
                // 本项目 minSdk = 24，所以显式关掉 —— 开着只是白白多一份 META-INF 垃圾。
                enableV1Signing = false
                // v2 是必需项：Android 7.0+ 认它，整包哈希（比 v1 的逐条目哈希更难伪造）。
                enableV2Signing = true
                // v3 是**目前唯一带密钥轮换能力**的方案（OWASP 也推荐「至少 v2 或 v3，
                // 想要轮换就用 v3」）。开启后可以将来用 `apksigner --lineage` 换密钥而不必改包名。
                //
                // **但别真的去轮换**：proof-of-rotation 只有 Android 13+（API 33）认，
                // Android 12 及更早会忽略它继续用原密钥 —— 我们 minSdk = 24，那一大片用户
                // 会因为新密钥不认识而装不上。正确做法是开 v3 当保险，**把密钥守住**。
                enableV3Signing = true
                // v4 不做：只产出独立的 .idsig 文件（供 adb 增量安装用），自身不提供任何
                // 安全保护，还会给 release 里多挂一个附件。
            }
        }
    }

    buildTypes {
        release {
            // 反射/无：全是系统 API + OkHttp，混淆收益低，保持可读可审计
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            if (pvHasSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
        debug {
            isMinifyEnabled = false
            // ── 与 CI 的 release 包共存 ─────────────────────────────────────────
            // 两个包的**签名必然不同**：release 用 `PV_KEYSTORE` 里的固定密钥（§2.6），debug 用
            // AGP 自动生成的 debug keystore（`~/.android/debug.keystore`）。Android 只允许
            // **同签名的包**覆盖安装，包名相同而签名不同时系统直接拒绝（「应用未安装 / 签名不一致」）
            // —— 于是调试就得先把手机上 CI 装的 release 卸载掉，装回来又得重新下 20 MB。
            //
            // 加包名后缀让两者成为**两个独立应用**：可以同时装、同时跑，各自独立的偏好与数据
            // （调试包的设置不会污染正在用的 release）。副作用是包名变成
            // `com.purevox.mic.debug` —— 所有 `packageName` 都是运行期取的，FileProvider 的
            // authority 用的是 `${applicationId}.fileprovider`（manifest 里），会自动跟着变。
            //
            // 注意：调试包点「检查更新」仍会下载**同名 release 的** APK（更新源就是本仓库的
            // release 列表），装上去是另一只应用而不是覆盖自己 —— 想更新 release 就去装那次
            // 下载的结果，想在调试包里验升级流程就手动 adb install。这是刻意保留的真实链路。
            applicationIdSuffix = ".debug"
            // 界面「更新」卡片显示的是 versionName，带上 -debug 才不会把调试包当成线上版本。
            versionNameSuffix = "-debug"
        }
    }

    // ── APK 文件名 ──────────────────────────────────────────────────────────────
    // 默认名 `app-debug.apk` / `app-release.apk` 只差一个单词：手机上同时装着调试包与 CI 的
    // release 包时，从下载目录里挑哪一个装基本靠猜。统一成与 CI 产物相同的形状
    // （`PureVox-<版本>-android-arm64.apk`，见 tools/automation/release_stage.sh）。
    //
    // **debug 的 -debug 后缀由 `versionNameSuffix` 带进来**，不要在这里再拼一次
    // （否则 `PureVox-1.0-debug-debug-…`）：`versionName` 已经是 `1.0-debug`，于是调试包
    // 自然叫 `PureVox-1.0-debug-android-arm64.apk`，CI 的 `PureVox-<年.月.日>-android-arm64.apk`
    // 永远不会有这个后缀。
    //
    // 用 v1 变体 API 的 `outputFileName`：AGP 8.7 的 `androidComponents` 新 API 里 **没有**
    // 改 APK 文件名的入口（`VariantOutput` 只有 versionCode / versionName / enabled），
    // 所以这里只能走已弃用但仍在的 `applicationVariants`。AGP 9 删掉它时同步迁移。
    applicationVariants.all {
        val pvApkName = "PureVox-${versionName}-android-arm64.apk"
        outputs.forEach {
            (it as com.android.build.gradle.internal.api.BaseVariantOutputImpl)
                .outputFileName = pvApkName
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        buildConfig = false
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")
    // 明文 ws:// —— 没有 TLS 依赖，也没有证书信任绕过（新版协议就是明文 + usesCleartextTraffic）
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}