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

cmake_minimum_required(VERSION 3.16)
project(purevox_opus_jni C)

set(OPUS_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../../../opus-src")

set(OPUS_BUILD_SHARED_LIBRARY OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
set(OPUS_ENABLE_FLOAT_API ON CACHE BOOL "" FORCE)

add_subdirectory(${OPUS_SOURCE_DIR} opus_build)

add_library(opus_jni SHARED
    ${CMAKE_CURRENT_SOURCE_DIR}/opus_jni.c
)

target_include_directories(opus_jni PRIVATE
    ${OPUS_SOURCE_DIR}/include
)

target_link_libraries(opus_jni PRIVATE opus)
