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

"""
POSIX（Linux / macOS）系统服务实现。

    - 单实例：fcntl.flock 锁文件（~/.purevox/purevox_<name>.lock）
    - 自启动：XDG autostart (~/.config/autostart/purevox.desktop)
    - 提示音：pw-play / paplay / aplay 播放提示音文件
    - 声音面板：pavucontrol / systemsettings
    - 提权：pkexec（可选）
    - 虚拟麦克风：PipeWire null-sink purevox_out，其内置 monitor 即虚拟麦克风
"""

import os
import subprocess
import sys
import threading
import time
from typing import Optional

# Linux 虚拟麦克风（双出口，同一路降噪音频，单一生产者 purevox_out）：
#   VIRTUAL_MIC_SINK   = 单声道 null-sink（唯一写入口，PureVox 降噪音频的输出目标）
#   VIRTUAL_MIC_SOURCE = sink 的内置 monitor（purevox_out.monitor，宽口径源）
#   VIRTUAL_MIC_MIC    = 非 monitor 真源（purevox_mic），供 OBS 等"只列真源"软件选
#                        麦克风用；由 module-remap-source 把 monitor 重映射而来，
#                        自动取数，无第二生产者。
#   VIRTUAL_MIC_LABEL  = sink/monitor 显示名（out）
#   VIRTUAL_MIC_MIC_LABEL = 真源显示名（mic）
# 重要：建纯真源**不能**用 `pactl load-module module-null-sink
# media.class=Audio/Source/Virtual`——实测会弄坏 pipewire-pulse 协议状态（pactl 报
# "连接失败：协议错误"、plasma-pa context kaput、系统托盘清空，仅重启 pipewire-pulse
# 才恢复）。健康方案是 module-remap-source（见 _create_virtual_mic_source）。
VIRTUAL_MIC_SINK = "purevox_out"
VIRTUAL_MIC_SOURCE = "purevox_out.monitor"
VIRTUAL_MIC_MIC = "purevox_mic"
VIRTUAL_MIC_LABEL = "PureVox out"
VIRTUAL_MIC_MIC_LABEL = "PureVox mic"

AUTOSTART_PATH = os.path.join(
    os.path.expanduser("~"), ".config", "autostart", "purevox.desktop")


class _NullLogger:
    """logger 为空时的静默替代，避免调用方忘记传 logger。"""
    def sys(self, *a, **k): pass
    def msg(self, *a, **k): pass
    def warn(self, *a, **k): pass
    def err(self, *a, **k): pass


def _safe_logger(logger):
    return logger if logger is not None else _NullLogger()


def acquire_single_instance_posix(lock_name: str) -> bool:
    """flock 锁文件单实例。返回 True 表示获得锁；进程退出自动释放。"""
    import fcntl
    global _SINGLE_INSTANCE_FH
    lock_file = os.path.join(os.path.expanduser("~"), ".purevox",
                             f"purevox_{lock_name}.lock")
    os.makedirs(os.path.dirname(lock_file), exist_ok=True)
    fh = open(lock_file, "w")
    try:
        fcntl.flock(fh.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        fh.close()
        return False
    fh.write(str(os.getpid()))
    fh.flush()
    _SINGLE_INSTANCE_FH = fh
    return True


_SINGLE_INSTANCE_FH = None


def _app_executable() -> str:
    """返回当前应用的启动命令（打包态用可执行文件，开发态用 python + 启动脚本）。"""
    if getattr(sys, 'frozen', False):
        return sys.executable
    main = os.path.abspath(os.path.join(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
        "run_tk.py"))
    if not os.path.exists(main):
        main = sys.argv[0]
    return f"{sys.executable} {main}"


def is_autostart_posix() -> bool:
    return os.path.exists(AUTOSTART_PATH)


def enable_autostart_posix(logger) -> bool:
    logger = _safe_logger(logger)
    try:
        os.makedirs(os.path.dirname(AUTOSTART_PATH), exist_ok=True)
        desktop = (
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=PureVox\n"
            "Comment=AI 麦克风降噪\n"
            f"Exec={_app_executable()}\n"
            "Terminal=false\n"
            "X-GNOME-Autostart-enabled=true\n"
        )
        with open(AUTOSTART_PATH, "w", encoding="utf-8") as f:
            f.write(desktop)
        logger.sys(f"开机自启: 已创建 {AUTOSTART_PATH}")
        return True
    except Exception as e:
        logger.err(f"创建自启动项: {e}")
        return False


def disable_autostart_posix(logger) -> bool:
    logger = _safe_logger(logger)
    try:
        if os.path.exists(AUTOSTART_PATH):
            os.remove(AUTOSTART_PATH)
        logger.sys("开机自启: 已移除")
        return True
    except Exception as e:
        logger.err(f"移除自启动项: {e}")
        return False


def beep_posix(freq_hz: int, duration_ms: int):
    """终端 BEL 提示音（非阻塞）。Linux 无 kernel32.Beep 等价物。"""
    def _play():
        try:
            sys.stdout.write("\a")
            sys.stdout.flush()
        except Exception:
            pass
    threading.Thread(target=_play, daemon=True).start()


def open_sound_panel_posix(logger):
    """打开系统声音设置。优先 pavucontrol（Pulse/PipeWire），回退 KDE/GNOME 设置。"""
    logger = _safe_logger(logger)
    candidates = [
        ["pavucontrol"],
        ["systemsettings", "kcm_pulseaudio"],
        ["gnome-control-center", "sound"],
    ]
    for cmd in candidates:
        try:
            # 用 Popen 异步启动，不阻塞、不杀进程（GUI 面板会一直运行到用户关闭）
            subprocess.Popen(cmd,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            logger.msg("已打开声音控制面板")
            return
        except Exception:
            continue
    logger.warn("未找到声音控制面板（pavucontrol / systemsettings）")


def run_as_admin_posix(cmd: str, logger) -> bool:
    """以管理员权限执行命令。Linux 用 pkexec，无则失败提示。"""
    logger = _safe_logger(logger)
    try:
        r = subprocess.run(["pkexec", "sh", "-c", cmd],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return r.returncode == 0
    except Exception as e:
        logger.err(f"提权执行失败（需 pkexec）: {e}")
        return False


# ── 虚拟麦克风（Linux）───────────────────────────────────────────────
# 架构（原生 PipeWire，单一生产者、双出口，两者均健康）：
#   1. 单生产者：只有一个单声道 null-sink purevox_out（降噪音频唯一写入口，
#      node.description="PureVox 虚拟麦克风"，audio.position=[MONO]）。
#   2. 出口 1 · monitor：purevox_out 的内置监视器源 purevox_out.monitor——
#      系统录音列表的一个 PureVox 项，单声道 48kHz；Audacity / 浏览器 /
#      pavucontrol 等"列出全部源"的软件可选它当麦克风。
#   3. 出口 2 · 真源：module-remap-source 把 purevox_out.monitor 重映射成
#      非 monitor 源 purevox_mic（media.class=Audio/Source、无 monitor_of），
#      自动从 monitor 取数，无第二生产者；OBS 等"只列真源"的软件在麦克风
#      下拉能选到它。
#
# 为什么纯真源用 module-remap-source 而非 module-null-sink：
#   曾用 `pactl load-module module-null-sink media.class=Audio/Source/Virtual
#   sink_name=purevox_mic` 建非 monitor 源，实测该模块会把 pipewire-pulse 协议
#   状态搞坏（pactl 报"连接失败：协议错误"、plasma-pa context kaput、系统托盘
#   清空，仅重启 pipewire-pulse 能恢复）。module-remap-source 是标准重映射，
#   实测健康且自带取数，无需额外 pw-link。
#
# 注意：PortAudio 直接打开 null-sink 会触发 PipeWire ALSA 插件堆损坏
# （free(): corrupted unsorted chunks），PureVox 永不直接打开它（走原生
# PipeWire 流）。

def _pw_node_id(node_name: str) -> Optional[int]:
    """返回 PipeWire 中指定 node.name 的本地 object id；不存在返回 None。"""
    try:
        out = subprocess.run(["pw-cli", "ls", "Node"],
                             capture_output=True, text=True, timeout=5).stdout
        cur_id = None
        for line in out.splitlines():
            if line.strip().startswith("id ") and "Node" in line:
                cur_id = int(line.split()[1].rstrip(","))
            elif node_name in line:
                return cur_id
    except Exception:
        pass
    return None


def virtual_mic_ready() -> bool:
    """虚拟麦克风（purevox_out sink，其 monitor 即虚拟麦克风）是否已创建。"""
    return _pw_node_id(VIRTUAL_MIC_SINK) is not None


def _kill_stray_loopbacks() -> None:
    """杀掉旧架构残留的 pw-loopback 进程（防御性清理，当前架构不再使用）。"""
    try:
        subprocess.run(["pkill", "-9", "-x", "pw-loopback"],
                       capture_output=True, text=True, timeout=5)
    except Exception:
        pass


def _create_virtual_mic_source(logger) -> bool:
    """创建非 monitor 真源 purevox_mic（供 OBS 等"只列真源"的软件当麦克风）。

    用标准 pulse 模块 module-remap-source 把 purevox_out.monitor 重映射成一个
    新源（media.class=Audio/Source、无 monitor_of），自动从 monitor 取数，无需
    额外 pw-link。实测比 module-null-sink+media.class=Audio/Source/Virtual 健康
    ——后者会弄坏 pipewire-pulse 协议（pactl 报协议错误、托盘清空）。无
    pactl/pipewire-pulse 时返回 False（此时仍保有 monitor 出口）。
    """
    if _pw_node_id(VIRTUAL_MIC_MIC) is not None:
        return True
    try:
        r = subprocess.run(
            ["pactl", "load-module", "module-remap-source",
             "master=" + VIRTUAL_MIC_SOURCE,
             "source_name=" + VIRTUAL_MIC_MIC,
             "channel_map=mono",
             "source_properties=device.description=" + VIRTUAL_MIC_MIC_LABEL],
            capture_output=True, text=True, timeout=10)
        if r.returncode != 0:
            logger.warn(f"创建虚拟麦克风源失败（无 pactl/pipewire-pulse？): {r.stderr.strip()}")
            return False
    except Exception as e:
        logger.warn(f"创建虚拟麦克风源异常: {e}")
        return False
    time.sleep(0.6)
    if _pw_node_id(VIRTUAL_MIC_MIC) is None:
        logger.warn("虚拟麦克风源创建后未就绪")
        return False
    return True


def ensure_virtual_mic(logger) -> bool:
    """确保虚拟麦克风（purevox_out sink + 真源 purevox_mic）存在，返回是否可用。

    PureVox 降噪音频经原生 PipeWire 输出流送入 purevox_out；其 monitor
    （purevox_out.monitor）与重映射真源（purevox_mic）即两个麦克风出口。
    已存在时幂等（检测-重置：有则不重建）；同时清理旧架构残留 loopback。
    缺 pactl 时仅保留 monitor 出口。
    """
    logger = _safe_logger(logger)
    _kill_stray_loopbacks()

    if _pw_node_id(VIRTUAL_MIC_SINK) is None:
        r = subprocess.run(
            ["pw-cli", "create-node", "adapter",
             "{ factory.name=support.null-audio-sink "
             "node.name=" + VIRTUAL_MIC_SINK + " media.class=Audio/Sink "
             "object.linger=true audio.position=[MONO] monitor.mode=disabled "
             'node.description="' + VIRTUAL_MIC_LABEL + '" }'],
            capture_output=True, text=True, timeout=10)
        if r.returncode != 0:
            logger.err(f"创建虚拟 sink 失败: {r.stderr.strip()}")
            return False
        time.sleep(0.5)

    if not virtual_mic_ready():
        logger.err("虚拟 sink 创建后未就绪")
        return False

    if _create_virtual_mic_source(logger):
        logger.sys(f"虚拟麦克风已就绪 ({VIRTUAL_MIC_SOURCE} / {VIRTUAL_MIC_MIC})")
    else:
        logger.warn("虚拟麦克风真源创建失败，仅保留 monitor 出口")
    return True


def _unload_virtual_mic_source(logger) -> None:
    """卸载纯 non-monitor 真源（按 pactl module 列表定位，module-remap-source）。"""
    try:
        out = subprocess.run(["pactl", "list", "short", "modules"],
                             capture_output=True, text=True, timeout=5).stdout
        for line in out.splitlines():
            if VIRTUAL_MIC_MIC in line:
                mid = line.split()[0]
                subprocess.run(["pactl", "unload-module", mid],
                               capture_output=True, text=True, timeout=5)
                break
    except Exception:
        pass
    # 防御：模块卸载后仍未消失的节点直接 destroy
    node_id = _pw_node_id(VIRTUAL_MIC_MIC)
    if node_id is not None:
        try:
            subprocess.run(["pw-cli", "destroy", str(node_id)],
                           capture_output=True, text=True, timeout=5)
        except Exception:
            pass


def remove_virtual_mic(logger) -> None:
    """卸载虚拟麦克风（幂等；无则忽略）。"""
    logger = _safe_logger(logger)
    _kill_stray_loopbacks()
    _unload_virtual_mic_source(logger)
    node_id = _pw_node_id(VIRTUAL_MIC_SINK)
    if node_id is not None:
        try:
            subprocess.run(["pw-cli", "destroy", str(node_id)],
                           capture_output=True, text=True, timeout=5)
            logger.sys("虚拟麦克风已卸载")
        except Exception as e:
            logger.err(f"卸载虚拟麦克风异常: {e}")
