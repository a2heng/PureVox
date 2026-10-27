# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
#
# SPDX-License-Identifier: GPL-3.0-or-later

# libpvopus.so —— JNI 转发层 + libopus 1.5.2（BSD-3-Clause）
#
# **产物直接入库**（`app/src/main/jniLibs/arm64-v8a/libpvopus.so`），与电脑端的
# `server/opus.dll` 同一思路：预编译二进制随包分发、运行时加载，好处是 Gradle 构建
# 完全不需要 NDK（CI 也就不用装 NDK、不用多一步编译）。重建用
# `tools/automation/build_opus_android.py`（唯一实现）。
#
# 只出 arm64-v8a：CI 产出的就是 `PureVox-Android-arm64-*.apk`，不打算发其它 ABI。
# 编译期就用 -Os + strip 去掉符号，静态库 3.3 MB 会瘦到 1 MB 上下。

cmake_minimum_required(VERSION 3.22)

# CXX 必须显式开：源文件是 .cpp（JNI 头在 C++ 下更顺），只 `project(... C)` 的话 CMake
# 认不出链接器语言，直接报 "can not determine linker language for target"。
project(pvopus C CXX)

set(CMAKE_CXX_STANDARD 11)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# libopus 静态库：由 tools/automation/build_opus_android.ps1 生成后放在这里（入库）。
set(OPUS_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/opus)
add_library(opus STATIC IMPORTED)
set_target_properties(opus PROPERTIES IMPORTED_LOCATION ${OPUS_DIR}/arm64-v8a/libopus.a)

add_library(pvopus SHARED pv_opus.cpp)

target_include_directories(pvopus PRIVATE
    ${OPUS_DIR}/include
    ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(pvopus opus log m)

# 符号可见性：只导出 JNI 入口，其余（libopus 内部）全部隐藏。
# 不加这个，libopus 的 C 符号会污染 .so 的导出表，既增大体积又可能与 app 里的同名符号撞车。
target_compile_options(pvopus PRIVATE -Os -fvisibility=hidden -ffunction-sections -fdata-sections)
# **16 KB 页对齐**（Android 15+ / Google Play 的硬要求）：不打这个标记，LINK 出来的
# PT_LOAD 是 4 KB 对齐（实测 `p_align 0x1000`），在 16 KB 页的设备上装不上 / 起不来。
# NDK r28 起默认就是 16 KB，但本机/CI 可能用 r27，所以显式写死，别靠默认值。
target_link_options(pvopus PRIVATE
    -Wl,-z,max-page-size=16384
    -Wl,-z,common-page-size=16384
    -Wl,--gc-sections -Wl,--exclude-libs,ALL -Wl,-s)