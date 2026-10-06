#!/bin/bash
# PureVox - AppImage packaging script (universal Linux)
# Output: dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.AppImage
# Bundles the embedded Python 3.12 (packages/python312) + app (pure Python engine).
# Requires: python3 (build), wget (appimagetool)
set -e
cd "$(dirname "$0")"

# Date stamp: single source is tools/automation/version.sh (same stamp as deb/rpm).
eval "$(bash tools/automation/version.sh)"
DATE="$STAMP"
APPIMG_FILE="PureVox-Linux-x64-${DATE}-release.AppImage"
DIST="dist"
STAGE="${TMPDIR:-/tmp}/purevox_appimage"
APPDIR="$STAGE/AppDir"
APP_NAME="purevox"

echo "==> ensure embedded Python 3.12 (packages/python312)"
if [ ! -x "packages/python312/bin/python3" ]; then
    ./bootstrap_python312.sh
fi

echo "==> prepare AppDir"
rm -rf "$STAGE"
mkdir -p "$APPDIR/usr/lib/purevox" "$APPDIR/usr/bin"

echo "==> stage payload (single source: tools/automation/stage_payload.sh, same set as deb/rpm)"
bash tools/automation/stage_payload.sh "$APPDIR/usr/lib/purevox" "$DATE"

# 捆绑系统 libopus（opuslib 经 ctypes find_library 加载；AppImage 自带一份，
# 保证宿主机没装 opus 时网络推流解码也可用）
for so in /usr/lib/x86_64-linux-gnu/libopus.so* /usr/lib64/libopus.so*; do
    [ -e "$so" ] && cp "$so" "$APPDIR/usr/lib/purevox/" || true
done

echo "==> bundle embedded Python 3.12"
cp -a packages/python312 "$APPDIR/usr/python312"

echo "==> desktop entry (AppImage needs it at AppDir root + usr/share/applications)"
mkdir -p "$APPDIR/usr/share/applications"
cat > "$APPDIR/usr/share/applications/$APP_NAME.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=PureVox
Exec=purevox
Icon=purevox
Terminal=false
Categories=AudioVideo;Audio;Utility;
EOF
cp "$APPDIR/usr/share/applications/$APP_NAME.desktop" "$APPDIR/$APP_NAME.desktop"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$APPDIR/usr/share/icons/hicolor/512x512/apps"
# 根目录 Icon 用 512 源 PNG（桌面文件 Icon=purevox 要求 AppDir 根有 png/svg/xpm）
if command -v magick >/dev/null; then
    magick assets/icons/audio_icon_base.png -resize 256x256 "$APPDIR/usr/share/icons/hicolor/256x256/apps/purevox.png"
    magick assets/icons/audio_icon_base.png -resize 512x512 "$APPDIR/usr/share/icons/hicolor/512x512/apps/purevox.png"
else
    python3 -c "
from PIL import Image
im = Image.open('assets/icons/audio_icon_base.png')
im.convert('RGBA').resize((256,256), Image.LANCZOS).save('$APPDIR/usr/share/icons/hicolor/256x256/apps/purevox.png')
im.convert('RGBA').resize((512,512), Image.LANCZOS).save('$APPDIR/usr/share/icons/hicolor/512x512/apps/purevox.png')
"
fi
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/purevox.png" "$APPDIR/purevox.png"

echo "==> AppRun launcher"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PYTHONHOME="$HERE/usr/python312"
export LD_LIBRARY_PATH="$HERE/usr/lib/purevox${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$HERE/usr/python312/bin:$PATH"
cd "$HERE/usr/lib/purevox" || exit 1
exec "$HERE/usr/python312/bin/python3" run_tk.py "$@"
EOF
chmod +x "$APPDIR/AppRun"

echo "==> appimagetool"
# 优先用缓存路径（CI 经 actions/cache 复用；本地默认 ~/.cache/purevox），
# 不存在才下载——避免每次打包都 wget
TOOL="${PUREVOX_APPIMAGETOOL:-$HOME/.cache/purevox/appimagetool}"
mkdir -p "$(dirname "$TOOL")"
[ -x "$TOOL" ] || bash tools/automation/retry.sh 3 wget -q -O "$TOOL" \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x "$TOOL"
mkdir -p "$DIST"
rm -f "$DIST/$APPIMG_FILE"
# CI 容器无 FUSE，AppImage 工具需用 --appimage-extract-and-run（本地有 FUSE 时参数无害）
"$TOOL" --appimage-extract-and-run "$APPDIR" "$DIST/$APPIMG_FILE" >/dev/null
echo "==> done: $DIST/$APPIMG_FILE"
