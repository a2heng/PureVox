#!/bin/bash
# PureVox - Fedora/RHEL RPM packaging script
# Output: dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.rpm
# Layout (same single path as deb/AppImage): sources+models+html+bundled
# embedded Python 3.12 (packages/python312, all pip deps inside) under
# /opt/purevox; /usr/bin/purevox execs the bundled interpreter.
# Requires: rpm-build, wget/curl (bootstrap download), ImageMagick or PIL (icon)
set -e
cd "$(dirname "$0")"

# Version/date: single source is tools/automation/version.sh (tag runs take it from
# GITHUB_REF_NAME so deb/rpm/AppImage can never drift across concurrent jobs).
eval "$(bash tools/automation/version.sh)"
REV="1"
ARCH="x86_64"
PKG_NAME="purevox"
DATE="$STAMP"
PKG_FILE="PureVox-Linux-x64-${DATE}-release.rpm"
DIST="dist"
STAGE="${TMPDIR:-/tmp}/purevox_rpm_build"
SPEC="$STAGE/purevox.spec"
ROOT="$STAGE/root"

echo "==> prepare staging $STAGE"
rm -rf "$STAGE"
mkdir -p "$ROOT/opt/purevox" "$ROOT/usr/bin" \
         "$ROOT/usr/share/applications" \
         "$ROOT/usr/share/icons/hicolor/256x256/apps"

echo "==> stage payload (single source: tools/automation/stage_payload.sh, same set as deb/AppImage)"
# Bootstrap first: stage_payload.sh only copies sources, the embedded
# interpreter is bundled by each format script afterwards.
if [ ! -x "packages/python312/bin/python3" ]; then
    ./bootstrap_python312.sh
fi
bash tools/automation/stage_payload.sh "$ROOT/opt/purevox" "$DATE"

echo "==> bundle embedded Python 3.12 (packages/python312, all pip deps inside)"
cp -a packages/python312 "$ROOT/opt/purevox/python312"

echo "==> /usr/bin/purevox launcher"
cat > "$ROOT/usr/bin/purevox" <<'EOF'
#!/bin/sh
# PureVox — AI 麦克风降噪
# 使用捆绑的内嵌 Python 3.12（numpy/onnxruntime 等全部依赖已随包携带，
# GUI 为标准库 Tkinter），与系统 Python/发行版包名完全隔离。
export PYTHONHOME="/opt/purevox/python312"
export PATH="/opt/purevox/python312/bin:$PATH"
cd /opt/purevox || exit 1
exec /opt/purevox/python312/bin/python3.12 /opt/purevox/run_tk.py "$@"
EOF
chmod +x "$ROOT/usr/bin/purevox"

echo "==> desktop file"
cat > "$ROOT/usr/share/applications/purevox.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=PureVox
Name[zh_CN]=PureVox
GenericName=AI Mic Noise Reduction
GenericName[zh_CN]=AI 麦克风降噪
Comment=Real-time AI microphone noise reduction
Comment[zh_CN]=实时 AI 麦克风降噪
Exec=/usr/bin/purevox
Icon=purevox
Terminal=false
Categories=AudioVideo;Audio;Utility;
Keywords=mic;noise;denoise;audio;PureVox;
StartupNotify=false
EOF

echo "==> icon (512 基图 → png)"
magick assets/icons/audio_icon_base.png -resize 256x256 \
    "$ROOT/usr/share/icons/hicolor/256x256/apps/purevox.png" 2>/dev/null \
 || python3 -c "
from PIL import Image
im = Image.open('assets/icons/audio_icon_base.png')
im = im.convert('RGBA').resize((256, 256), Image.LANCZOS)
im.save('$ROOT/usr/share/icons/hicolor/256x256/apps/purevox.png')
" 2>/dev/null || true

echo "==> spec"
cat > "$SPEC" <<EOF
Name:           $PKG_NAME
Version:        $VERSION
Release:        $REV
Summary:        AI microphone noise reduction
License:        GPL-3.0-or-later
BuildArch:      $ARCH
Requires:       pipewire, pipewire-pulseaudio, pulseaudio-libs, opus

%description
Real-time AI microphone noise reduction, echo cancellation and target
speaker extraction. Installs under /opt/purevox with a bundled embedded
Python 3.12 runtime (all Python dependencies included); only PipeWire /
pipewire-pulse, the libpulse client library and libopus come from the
host system.

%install
cp -a $ROOT/. %{buildroot}/

%files
/opt/purevox
/usr/bin/purevox
/usr/share/applications/purevox.desktop
/usr/share/icons/hicolor/256x256/apps/purevox.png

%post
gtk-update-icon-cache /usr/share/icons/hicolor 2>/dev/null || true

%postun
gtk-update-icon-cache /usr/share/icons/hicolor 2>/dev/null || true
EOF

echo "==> rpmbuild"
# 载荷是自包含的预编译运行时（pbs 内嵌 Python，含 tcl/tk 等自带库），
# 必须关闭发行版默认的构建后处理（brp-strip/brp-compress/check-rpaths 等）：
# check-rpaths 会因 pbs 自带库中的死 RPATH（构建机路径）判违规终止，
# strip 也无谓地改动捆绑二进制——与 dpkg 打包 deb 时不做任何此类处理一致。
mkdir -p "$STAGE"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
rpmbuild --define "_topdir $STAGE" --define "buildroot $ROOT" \
    --define "__os_install_post %{nil}" \
    --define "source_date_epoch_from_changelog 0" \
    -bb "$SPEC" >/dev/null
RPMSRC="$STAGE/RPMS/$ARCH/${PKG_NAME}-${VERSION}-${REV}.$ARCH.rpm"
[ -f "$RPMSRC" ] || { echo "rpm not produced"; exit 1; }
mkdir -p "$DIST"
rm -f "$DIST/$PKG_FILE"
mv "$RPMSRC" "$DIST/$PKG_FILE"
echo "==> done: $DIST/$PKG_FILE"
