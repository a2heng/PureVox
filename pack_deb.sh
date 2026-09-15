#!/bin/bash
# PureVox — Linux deb 打包脚本
# 产出: dist/PureVox-Linux-x64-<yyyy-MM-dd-HHmm>-release.deb
#
# 布局（与既有版本一致）:
#   /opt/purevox/          全部源码+模型+html+uitk+pvplatform
#   /opt/purevox/python312  内嵌 Python 3.12（packages/python312 全量拷贝，
#                          zeroconf/aiohttp/cryptography/opuslib 等 pip 依赖
#                          全部装好，与系统 Python 完全隔离；GUI 为标准库 Tkinter）
#   /usr/bin/purevox       启动脚本 → 用内嵌 python312 跑 run_tk.py
#   /usr/share/applications/purevox.desktop
#   /usr/share/icons/hicolor/256x256/apps/purevox.png
#
# Depends: pipewire + pipewire-pulse（音频走 pipewire-pulse 兼容层，libpulse 经
#          ctypes 直调 _libpulse 客户端库）+ libpulse0 + libopus0。
#          numpy/onnxruntime/scipy 等 Python 依赖已捆绑进内嵌 python312。
# Python 依赖全部捆绑进 python312（与 AppImage 同一实现路径），不依赖系统
# python 及发行版 python 包名，故 Depends 不写任何 Python 依赖；
# 注意 `pipewire` 包本身既不依赖 `pipewire-pulse` 也不依赖 `libpulse0`，必须显式声明。
# 内嵌 python 为 python-build-standalone 预编译包（版本见 tools/automation/versions.env），
# 在新发行版（如 Debian 13）需补 libcrypt.so.2 软链指向系统 libcrypt.so.1
# （libxcrypt ABI 兼容）才能加载。

set -euo pipefail
cd "$(dirname "$0")"

# 版本号/文件名时间戳：单一实现见 tools/automation/version.sh —— tag 触发 CI 时取自
# GITHUB_REF_NAME（v2026.08.10.1517 → VERSION 2026.08.10.1517 / STAMP 2026-08-10-1517），
# 保证 deb/rpm/AppImage/窗口标题与 tag 名完全一致、不因各 job 并发时刻漂移；
# 本地/手动跑回退当前 UTC 时间。
eval "$(bash tools/automation/version.sh)"
REV="1"
DATE="$STAMP"
PKG_FILE="PureVox-Linux-x64-${DATE}-release.deb"
ARCH="amd64"
DIST="dist"
STAGE="${TMPDIR:-/tmp}/purevox_deb_build"
ROOT="$STAGE/root"
CONTROL="$ROOT/DEBIAN/control"

echo "==> 准备打包目录 $STAGE"
rm -rf "$STAGE"
mkdir -p "$ROOT/opt/purevox" \
         "$ROOT/usr/bin" \
         "$ROOT/usr/share/applications" \
         "$ROOT/usr/share/icons/hicolor/256x256/apps" \
         "$ROOT/DEBIAN"

echo "==> 拷贝 payload（sources/models/html/server/pvplatform/pvengine/uitk/about + 版本戳）"
# 文件集合的唯一来源：tools/automation/stage_payload.sh（deb/rpm/AppImage 同一套）。
if [ ! -x "packages/python312/bin/python3" ]; then
    ./bootstrap_python312.sh
fi
bash tools/automation/stage_payload.sh "$ROOT/opt/purevox" "$DATE"

echo "==> 捆绑内嵌 Python 3.12（packages/python312，含全部 pip 依赖；GUI 为标准库 Tkinter）"
cp -a packages/python312 "$ROOT/opt/purevox/python312"
# 内嵌 python 3.12.11 由 GCC 15 编译，链接 libcrypt.so.2；较新发行版
# （如 Debian 13）只有 libcrypt.so.1（libxcrypt，ABI 兼容），补软链使其可加载。
# 若构建机上本就存在 libcrypt.so.2 则跳过。
if [ ! -e "$ROOT/opt/purevox/python312/lib/libcrypt.so.2" ] && \
   [ -e /usr/lib/x86_64-linux-gnu/libcrypt.so.1.1.0 ]; then
    ln -s /usr/lib/x86_64-linux-gnu/libcrypt.so.1.1.0 \
        "$ROOT/opt/purevox/python312/lib/libcrypt.so.2"
fi

echo "==> /usr/bin/purevox 启动脚本"
cat > "$ROOT/usr/bin/purevox" <<'EOF'
#!/bin/sh
# PureVox — AI 麦克风降噪
# 使用捆绑的内嵌 Python 3.12（/opt/purevox/python312，numpy/onnxruntime 等
# 全部依赖已随包携带，GUI 为标准库 Tkinter），与系统 Python/发行版包名完全隔离。
export PYTHONHOME="/opt/purevox/python312"
export PATH="/opt/purevox/python312/bin:$PATH"
cd /opt/purevox || exit 1
exec /opt/purevox/python312/bin/python3.12 /opt/purevox/run_tk.py "$@"
EOF
chmod +x "$ROOT/usr/bin/purevox"

echo "==> desktop 文件"
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

echo "==> 图标 (512 基图 → png)"
magick assets/icons/audio_icon_base.png -resize 256x256 \
    "$ROOT/usr/share/icons/hicolor/256x256/apps/purevox.png" 2>/dev/null \
 || python3 -c "
from PIL import Image
im = Image.open('assets/icons/audio_icon_base.png')
im = im.convert('RGBA').resize((256, 256), Image.LANCZOS)
im.save('$ROOT/usr/share/icons/hicolor/256x256/apps/purevox.png')
"

echo "==> DEBIAN/control"
mkdir -p "$ROOT/DEBIAN"
cat > "$CONTROL" <<EOF
Package: purevox
Version: $VERSION-$REV
Section: sound
Priority: optional
Architecture: $ARCH
Maintainer: a2heng <752848283@qq.com>
Depends: pipewire, pipewire-pulse, libpulse0, libopus0
Description: PureVox — Real-time AI microphone noise reduction
 Real-time AI audio denoising / target speech extraction / echo cancellation
 for the local microphone, with remote network streaming support.
 PureVox 实时 AI 麦克风降噪/目标提取/回声消除。
 .
 Python 运行时与全部 Python 依赖（zeroconf / aiohttp /
 cryptography / opuslib）已捆绑于包内 /opt/purevox/python312，与系统 Python
 完全隔离，不依赖发行版 python 包名，跨发行版可安装即用；界面为标准库
 Tkinter，无 Qt。
 .
 Linux 音频基于 pipewire-pulse 兼容层（自研 ctypes 绑定直调系统 libpulse），
 格式协商 F32 单声道 48000Hz，重采样与声道转换由 PipeWire 负责。虚拟麦克风为
 单声道 null-sink purevox_out（并经 module-remap-source 提供真源 purevox_mic），
 其它应用可选 "PureVox 虚拟麦克风" 作为输入设备。
EOF

echo "==> 构建 $PKG_FILE"
mkdir -p "$DIST"
rm -f "$DIST/$PKG_FILE"
dpkg-deb --build --root-owner-group "$ROOT" "$DIST/$PKG_FILE" >/dev/null
echo "==> 完成: $DIST/$PKG_FILE"
dpkg-deb --info "$DIST/$PKG_FILE" | head -14 || true
