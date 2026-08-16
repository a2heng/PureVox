#!/usr/bin/env bash
# PureVox — Windows 交叉编译打包（host Linux 用 mingw）
# 收集 purevox.exe + Qt DLL + 平台插件 + onnxruntime + mingw 运行时 + 模型 + html 到 dist/PureVox-Windows/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
QT_DIR="/home/a2heng/qt-mingw/6.5.3/mingw_64"
OUT="${ROOT}/dist/PureVox-Windows"
rm -rf "$OUT"
mkdir -p "$OUT/platforms"

# 1. exe
cp "$ROOT/build-win/purevox.exe" "$OUT/"

# 2. Qt6 DLL（只需 Widgets/WebSockets/Svg 依赖链）
for dll in Core Gui Network WebSockets Widgets Svg; do
    cp "$QT_DIR/bin/Qt6${dll}.dll" "$OUT/"
done
# 平台插件
cp "$QT_DIR/plugins/platforms/qwindows.dll" "$OUT/platforms/"
# SVG 支持：图片格式插件 + 图标引擎插件（QIcon 加载 svg 必需）
mkdir -p "$OUT/imageformats"
cp "$QT_DIR/plugins/imageformats/qsvg.dll" "$OUT/imageformats/"
mkdir -p "$OUT/iconengines"
cp "$QT_DIR/plugins/iconengines/qsvgicon.dll" "$OUT/iconengines/"

# 3. onnxruntime
cp "$ROOT/packages/onnxruntime-win-x64-1.11.1/lib/onnxruntime.dll" "$OUT/"
cp "$ROOT/packages/onnxruntime-win-x64-1.11.1/lib/onnxruntime_providers_shared.dll" "$OUT/"
cp "$ROOT/packages/onnxruntime-win-x64-1.11.1/lib/api-ms-win-core-libraryloader-l1-2-0.dll" "$OUT/"
cp "$ROOT/packages/onnxruntime-win-x64-1.11.1/lib/api-ms-win-core-processtopology-obsolete-l1-1-0.dll" "$OUT/"

# 4. mingw 运行时
cp /usr/lib/gcc/x86_64-w64-mingw32/13-win32/libgcc_s_seh-1.dll "$OUT/"
cp /usr/lib/gcc/x86_64-w64-mingw32/13-win32/libstdc++-6.dll "$OUT/"
cp /usr/x86_64-w64-mingw32/lib/libwinpthread-1.dll "$OUT/"

# 5. 模型
cp "$ROOT"/*.onnx "$OUT/"

# 6. html（网络推流）
cp -r "$ROOT/html" "$OUT/"

echo "=== 打包完成: $OUT ==="
du -sh "$OUT"
