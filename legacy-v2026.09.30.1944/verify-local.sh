#!/bin/bash
# PureVox - 本地 CI 镜像（WSL / 原生 Linux 均可）
# 与 .github/workflows/ci.yml 的 linux job 同一套步骤，保证本地全流程可复现。
# 用法（在仓库根）:
#   ./verify-local.sh smoke            引擎冒烟（导入 + 加载模型跑一帧）
#   ./verify-local.sh deb              打 deb（自动引导内嵌 python312）
#   ./verify-local.sh appimage         打 AppImage
#   ./verify-local.sh rpm              打 rpm（仅 rpm 系发行版）
#   ./verify-local.sh all              依次执行本发行版支持的全部阶段
set -euo pipefail
cd "$(dirname "$0")"

stage="${1:-all}"

have_cmd() { command -v "$1" >/dev/null 2>&1; }

install_sysdeps() {
    # 与 CI 的 "Install system dependencies" 对应；已有则跳过，幂等
    if have_cmd apt-get; then
        local pkgs="build-essential make pkg-config libssl-dev libffi-dev libopus0 \
python3 python3-pip python3-setuptools zlib1g-dev file wget unzip zip imagemagick"
        local missing=""
        for p in $pkgs; do dpkg -s "$p" >/dev/null 2>&1 || missing="$missing $p"; done
        if [ -n "$missing" ]; then
            echo "==> apt 安装缺失系统依赖:$missing"
            sudo apt-get update -qq
            sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $missing
        fi
    elif have_cmd dnf; then
        echo "==> dnf 安装系统依赖"
        sudo dnf install -y --quiet make python3 python3-pip gcc rpm-build file wget unzip zip || true
    else
        echo "!! 未识别的包管理器，跳过系统依赖安装"
    fi
}

py() {
    if [ "${FORCE_PY:-}" != "" ]; then
        "$FORCE_PY" "$@"
    else
        python3 "$@"
    fi
}

# vboxsf 共享目录不支持 symlink（内嵌 python312 与打包拷贝都会坏），
# 检测到在 vboxsf 上运行时，自动把源码同步到原生文件系统构建，
# 完成后把产物拷回共享的 dist/。
relocate_if_vboxsf() {
    case "${1:-}" in
        deb|appimage|rpm|all) ;;
        *) return 0 ;;
    esac
    # 内层调用已设置此变量（pwd 已是原生目录），防止递归搬迁
    [ -n "${PUREVOX_RELOCATED:-}" ] && return 0
    # vboxsf(VirtualBox 共享) 与 9p(WSL /mnt/* drvfs) 都不支持 symlink，
    # 内嵌 python312 与打包拷贝会坏，均需切到原生文件系统构建
    case "$(findmnt -no FSTYPE -T "$(pwd)" 2>/dev/null || echo '')" in
        vboxsf|9p) ;;
        *) return 0 ;;
    esac
    BUILD_ROOT="${PUREVOX_VM_BUILD:-$HOME/purevox-build}"
    ORIG_DIR="$(pwd)"
    echo "==> 仓库在 vboxsf 上（不支持 symlink），切换到原生目录构建：$BUILD_ROOT"
    command -v rsync >/dev/null 2>&1 || { sudo apt-get install -y -qq rsync || true; }
    mkdir -p "$BUILD_ROOT"
    if command -v rsync >/dev/null 2>&1; then
        rsync -a --delete \
            --exclude '.git' --exclude 'legacy-v2026.*' --exclude 'dist' \
            --exclude 'packages' --exclude '.py312-src' \
            ./ "$BUILD_ROOT/"
    else
        find "$BUILD_ROOT" -mindepth 1 -maxdepth 1 ! -name dist -exec rm -rf {} +
        tar --exclude='./.git' --exclude='./legacy-v2026.*' --exclude='./dist' \
            --exclude='./packages' --exclude='./.py312-src' \
            -cf - . | tar -xf - -C "$BUILD_ROOT"
    fi
    cd "$BUILD_ROOT"
    # 显式 bash 调用：vboxsf 挂载带 noexec，直接执行共享目录里的脚本会被拒
    PUREVOX_RELOCATED=1 bash "$ORIG_DIR/verify-local.sh" "$1"
    rc=$?
    mkdir -p "$ORIG_DIR/dist"
    cp -f "$BUILD_ROOT"/dist/PureVox-* "$ORIG_DIR/dist/" 2>/dev/null || true
    echo "==> 产物已拷回共享目录 $ORIG_DIR/dist/"
    exit $rc
}

install_pydeps() {
    echo "==> 安装 Python 依赖（requirements-linux.txt，与 CI 同源 pin 版本）"
    py -m pip install -r requirements-linux.txt pillow 2>/dev/null \
        || py -m pip install --break-system-packages -r requirements-linux.txt pillow
}

smoke() {
    echo "==> 引擎冒烟（tools/automation/smoke.py）：导入 pvengine 并跑一帧降噪"
    py tools/automation/smoke.py
}

relocate_if_vboxsf "$stage"

case "$stage" in
    sysdeps)
        install_sysdeps
        ;;
    deps)
        install_pydeps
        ;;
    smoke)
        install_pydeps
        smoke
        ;;
    deb|appimage)
        install_sysdeps
        install_pydeps
        bash "pack_${stage}.sh"
        ls -lh dist/PureVox-Linux-x64-*
        ;;
    rpm)
        install_sysdeps
        install_pydeps
        bash pack_rpm.sh
        ls -lh dist/PureVox-Linux-x64-*.rpm
        ;;
    all)
        install_sysdeps
        install_pydeps
        smoke
        if have_cmd apt-get; then
            bash pack_deb.sh && ls -lh dist/*.deb
            bash pack_appimage.sh && ls -lh dist/*.AppImage
        elif have_cmd dnf; then
            bash pack_rpm.sh && ls -lh dist/*.rpm
        fi
        ;;
    *)
        echo "用法: $0 {sysdeps|deps|smoke|deb|rpm|appimage|all}"
        exit 2
        ;;
esac
echo "==> verify-local[$stage] 完成"
