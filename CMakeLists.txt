# PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# GPL-3.0-or-later.  See LICENSE for details.
# The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

cmake_minimum_required(VERSION 3.16)
project(PureVox VERSION 0.9.0 LANGUAGES C CXX)

set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

set(PV_SRC_DIR ${CMAKE_CURRENT_SOURCE_DIR})
set(PV_PKG_DIR ${PV_SRC_DIR}/packages)

# ── 内嵌 ONNX Runtime（捆绑 1.11.1，纯 CPU）──
set(PV_ORT_INC ${PV_PKG_DIR}/onnxruntime-linux-x64-1.11.1/include)
set(PV_ORT_LIB ${PV_PKG_DIR}/onnxruntime-linux-x64-1.11.1/lib)

# ── pffft + libsamplerate（aimic 依赖，源码随仓库）──
set(PV_PFFFT_SRC ${PV_PKG_DIR}/pffft)
set(PV_SRC_DIR_LIBSR ${PV_PKG_DIR}/libsamplerate)
file(GLOB PV_SR_SRC ${PV_SRC_DIR_LIBSR}/*.c)

# ── 音频核心（原 libaimic.so，纯 C，保留参考不链接）──
add_library(pvcore STATIC
    ${PV_SRC_DIR}/aimic.c
    ${PV_PFFFT_SRC}/pffft.c
    ${PV_PFFFT_SRC}/pffft_common.c
)
target_include_directories(pvcore PUBLIC ${PV_SRC_DIR} ${PV_SRC_DIR_LIBSR} ${PV_ORT_INC} ${PV_PKG_DIR}/pffft)
target_compile_definitions(pvcore PRIVATE HAVE_CONFIG_H)
target_compile_options(pvcore PRIVATE -O2 -w)

# ── 模块化 C++ 音频处理库（新架构，替换 aimic.c 各阶段）──
set(PV_DSP_SRC
    src/dsp/onnxmodel.cpp
    src/dsp/vad.cpp
    src/dsp/agc.cpp
    src/dsp/compressor.cpp
    src/dsp/eq.cpp
    src/dsp/stft.cpp
    src/dsp/ringbuffer.cpp
    src/dsp/resampler.cpp
    src/dsp/denoise.cpp
    src/dsp/tse.cpp
    src/dsp/aec.cpp
    src/dsp/melspectrum.cpp
    src/dsp/processor.cpp
    ${PV_SR_SRC}
    ${PV_PFFFT_SRC}/pffft.c
    ${PV_PFFFT_SRC}/pffft_common.c
)
add_library(pvdsp STATIC ${PV_DSP_SRC})
target_include_directories(pvdsp PUBLIC ${PV_SRC_DIR} ${PV_SRC_DIR_LIBSR} ${PV_ORT_INC} ${PV_PKG_DIR}/pffft)
target_compile_definitions(pvdsp PRIVATE HAVE_CONFIG_H)
target_compile_options(pvdsp PRIVATE -O2 -w)

# ── PipeWire / ALSA 桥（系统库）──
find_package(PkgConfig)
pkg_check_modules(PV_PIPEWIRE IMPORTED_TARGET libpipewire-0.3)
pkg_check_modules(PV_ALSA IMPORTED_TARGET alsa)

if(PV_PIPEWIRE_FOUND)
    add_library(pvpipe STATIC ${PV_SRC_DIR}/pipewire_client.c)
    target_include_directories(pvpipe PUBLIC ${PV_SRC_DIR})
    target_link_libraries(pvpipe PUBLIC PkgConfig::PV_PIPEWIRE)
endif()

if(PV_ALSA_FOUND)
    add_library(pvalsa STATIC ${PV_SRC_DIR}/alsa_client.c)
    target_include_directories(pvalsa PUBLIC ${PV_SRC_DIR})
    target_link_libraries(pvalsa PUBLIC PkgConfig::PV_ALSA)
endif()

# ── Qt6 GUI ──
find_package(Qt6 REQUIRED COMPONENTS Widgets)

qt_add_executable(purevox
    src/main.cpp
    src/mainwindow.cpp
    src/mainwindow.h
    src/audioengine.cpp
    src/audioengine.h
    src/audiothread.cpp
    src/audiothread.h
    src/audiobackend.h
    src/pwbackend.cpp
    src/pwbackend.h
    src/alsabackend.cpp
    src/alsabackend.h
    src/vubar.cpp
    src/vubar.h
    src/segmented.cpp
    src/segmented.h
    src/deviceenum.cpp
    src/deviceenum.h
    src/eqcurve.cpp
    src/eqcurve.h
    src/spectrum.cpp
    src/spectrum.h
    src/dialog_about.cpp
    src/dialog_about.h
    src/dialog_tse_reference.cpp
    src/dialog_tse_reference.h
    src/virtualmic.cpp
    src/virtualmic.h
)
target_include_directories(purevox PRIVATE
    ${PV_SRC_DIR}
    ${PV_ORT_INC}
)
target_link_directories(purevox PRIVATE ${PV_ORT_LIB})
target_link_libraries(purevox PRIVATE
    Qt6::Widgets
    pvdsp
    onnxruntime
)
if(PV_PIPEWIRE_FOUND)
    target_link_libraries(purevox PRIVATE pvpipe)
endif()
if(PV_ALSA_FOUND)
    target_link_libraries(purevox PRIVATE pvalsa)
endif()
