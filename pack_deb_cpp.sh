#!/bin/bash
# PureVox — Qt C++ 重构版 Linux deb 打包脚本
# 产出: dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.deb
#
# 布局:
#   /opt/purevox/purevox         Qt C++ 可执行文件
#   /opt/purevox/*.onnx          三个 AI 模型
#   /opt/purevox/libonnxruntime.so*  捆绑的预编译 onnxruntime 1.11.1
#   /usr/bin/purevox             启动脚本（设 LD_LIBRARY_PATH）
#   /usr/share/applications/purevox.desktop
#   /usr/share/icons/hicolor/256x256/apps/purevox.png
#
# Depends: 系统 Qt6 + PipeWire + ALSA 运行库（onnxruntime 捆绑）

set -euo pipefail
cd "$(dirname "$0")"

if [ -n "${GITHUB_REF_NAME:-}" ] && [[ "$GITHUB_REF_NAME" == v* ]]; then
    VERSION="${GITHUB_REF_NAME#v}"
else
    VERSION="$(date +%Y.%m.%d.%H%M)"
fi
REV="1"
DATE="${VERSION//./-}"
PKG_FILE="PureVox-Linux-x64-${DATE}-release.deb"
ARCH="amd64"
DIST="dist"
STAGE="${TMPDIR:-/tmp}/purevox_deb_cpp_build"
ROOT="$STAGE/root"
CONTROL="$ROOT/DEBIAN/control"

echo "==> 构建 Qt C++ 版 (cmake)"
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build build -j4 >/dev/null
[ -x "build/purevox" ] || { echo "缺少 build/purevox"; exit 1; }

echo "==> 准备打包目录 $STAGE"
rm -rf "$STAGE"
mkdir -p "$ROOT/opt/purevox" \
         "$ROOT/usr/bin" \
         "$ROOT/usr/share/applications" \
         "$ROOT/usr/share/icons/hicolor/256x256/apps" \
         "$ROOT/DEBIAN"

echo "==> 拷贝可执行文件/模型/onnxruntime"
cp build/purevox "$ROOT/opt/purevox/"
for f in aec9_ep0544.onnx tse15_stream_ep_0673.onnx v9_fft2048_band256_epoch_261.onnx; do
    cp "$f" "$ROOT/opt/purevox/"
done
cp packages/onnxruntime-linux-x64-1.11.1/lib/libonnxruntime.so* "$ROOT/opt/purevox/"

echo "==> 生成图标 (256x256)"
python3 -c "
from PIL import Image
im = Image.open('audio_icon_base_on_1024.png').convert('RGBA')
im = im.resize((256, 256), Image.LANCZOS)
im.save('$ROOT/usr/share/icons/hicolor/256x256/apps/purevox.png')
" 2>/dev/null || cp audio_icon_base_on_1024.png "$ROOT/usr/share/icons/hicolor/256x256/apps/purevox.png"

echo "==> /usr/bin/purevox 启动脚本"
cat > "$ROOT/usr/bin/purevox" <<'EOF'
#!/bin/sh
# PureVox — AI 麦克风降噪（Qt C++ 重构版）
# 在 /opt/purevox 下运行；捆绑的 onnxruntime 提前注入 LD_LIBRARY_PATH。
export LD_LIBRARY_PATH="/opt/purevox${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cd /opt/purevox
exec /opt/purevox/purevox "$@"
EOF
chmod +x "$ROOT/usr/bin/purevox"

echo "==> control"
cat > "$CONTROL" <<EOF
Package: purevox
Version: $VERSION
Architecture: $ARCH
Maintainer: a2heng <752848283@qq.com>
Section: sound
Priority: optional
Description: PureVox — real-time AI audio denoise / target speaker extraction / echo cancellation (Qt C++ rewrite)
Depends: libqt6widgets6, libqt6gui6, libqt6core6, qt6-wayland, libpipewire-0.3-0t64, libasound2t64
EOF

echo "==> .desktop"
cat > "$ROOT/usr/share/applications/purevox.desktop" <<'EOF'
[Desktop Entry]
Name=PureVox
Comment=AI microphone denoise
Exec=/usr/bin/purevox
Icon=purevox
Terminal=false
Type=Application
Categories=AudioVideo;Audio;
EOF

echo "==> dpkg-deb 打包"
mkdir -p "$DIST"
dpkg-deb --build --root-owner-group "$ROOT" "$DIST/$PKG_FILE"
echo "==> 完成: $DIST/$PKG_FILE"
