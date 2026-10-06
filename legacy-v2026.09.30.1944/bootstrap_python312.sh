#!/bin/sh
# PureVox — 内嵌 Python 3.12 引导脚本（Linux）
# 下载 python-build-standalone 预编译 CPython（install_only 包，一次性，可缓存），
# 解压出自包含的 packages/python312/（独立于系统 Python）。幂等：已生成则跳过，
# 仅补齐依赖。不编译任何东西——预编译包自带 ssl/_ctypes/pip。
#
# 用法: ./bootstrap_python312.sh
# 环境变量:
#   PUREVOX_CPYTHON_TARBALL  预置的预编译包路径（离线/内网用）
set -e

APP_DIR="$(cd "$(dirname "$0")" && pwd)"
PREFIX="$APP_DIR/packages/python312"
PY_MAJOR=3.12
PY_BIN="$PREFIX/bin/python$PY_MAJOR"

# 版本唯一来源：tools/automation/versions.env（CI 缓存 key 由同一值推导）
# shellcheck disable=SC1091
. "$(dirname "$0")/tools/automation/versions.env"
PY_VER="$PBS_CPYTHON_VER"
TARBALL_NAME="cpython-${PY_VER}+${PBS_TAG}-x86_64-unknown-linux-gnu-install_only.tar.gz"
TARBALL_URL="https://github.com/astral-sh/python-build-standalone/releases/download/${PBS_TAG}/${TARBALL_NAME}"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/purevox"
TARBALL="${PUREVOX_CPYTHON_TARBALL:-$CACHE_DIR/$TARBALL_NAME}"

# 内嵌解释器自带 libpython3.12.so，需让子进程找到它（独立于系统环境）
export LD_LIBRARY_PATH="$PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

# 1. 获取预编译 CPython（一次性；已缓存则直接复用）
if [ ! -f "$TARBALL" ]; then
    echo "==> 下载 CPython ${PY_VER} 预编译包（python-build-standalone ${PBS_TAG}）..."
    mkdir -p "$(dirname "$TARBALL")"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --retry-delay 10 -o "$TARBALL.part" "$TARBALL_URL"
    else
        wget --tries=3 -O "$TARBALL.part" "$TARBALL_URL"
    fi
    mv "$TARBALL.part" "$TARBALL"
fi

# 2. 解压到 packages/python312（若未生成）。install_only 包根目录为 python/
if [ ! -x "$PY_BIN" ]; then
    echo "==> 解压 Python ${PY_VER} 到 $PREFIX ..."
    rm -rf "$PREFIX"
    mkdir -p "$PREFIX"
    tar -xzf "$TARBALL" -C "$PREFIX" --strip-components=1
    echo "==> 内嵌 Python 就绪"
fi

# 3. 安装依赖（缺什么装什么）
"$PY_BIN" -m pip install --upgrade pip setuptools wheel
"$PY_BIN" -m pip install -r "$APP_DIR/requirements-linux.txt"

cat <<EOF

PureVox 内嵌 Python 就绪（独立于系统环境，位于 packages/python312）。
用下面的启动器运行（自动带 LD_LIBRARY_PATH）：
  ./py312 run_tk.py
  ./py312 -m pip install <pkg>
EOF
