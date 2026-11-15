#!/usr/bin/env python3
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
"""Linux 界面 / 会话自动化驱动（AT-SPI + HTTP 调试接口）。

两件事：
  1. 按**可访问名**查找 / 点击 WebView 里的控件（不需要像素坐标）；
  2. 起应用 / 读 `/debug` / 断言 —— 无人值守的验证链（AGENTS.md「调试优先」：
     交互验证不许以「点不了」为由跳过；读数一律走调试接口，不靠截图猜）。

用法：
    # 纯 AT-SPI（需 python3-pyatspi，且 GTK 的 a11y 总开关要开：
    #   gsettings set org.gnome.desktop.interface toolkit-accessibility true）
    python3 ui.py list                     # 列出 PureVox 窗口里所有命名控件
    python3 ui.py click "启动"             # 点击第一个名为「启动」的控件
    python3 ui.py sel "EDIFIER R20, USB Audio"   # 在打开的下拉里选一项

    # 会话驱动（推荐：一条链做完整验证）
    python3 ui.py start                    # 起 /usr/bin/purevox，等 /debug 就绪
    python3 ui.py start --private-bus      # 另起一条没有 watcher 的私有总线（复现无 watcher 场景）
    python3 ui.py take '.update.phase'     # 读 /debug（给 jq 表达式；不给就整段打印）
    python3 ui.py verify-update deb        # 点「更新 → 检查更新」，断言选中的是 .deb
    python3 ui.py stop                     # 收掉我们起来的应用与 watcher 替身

    python3 ui.py watcher                  # 单独顶托盘 watcher 替身（前台跑，见下）

**为什么需要内部那个 watcher 替身**：托盘走 ksni，注册不上
`org.kde.StatusNotifierWatcher` 就会返回硬错误；GNOME 的这一角色由 appindicator 扩展提供，
扩展没启用（重装/最小化安装常见）时**会话总线上根本没有这个名字** —— 老版本 PureVox
（≤ 26.10.18）会因此在启动时 panic。替身只实现 ksni 真正会用的几件事
（`RegisterStatusNotifierItem` + `IsStatusNotifierHostRegistered`），让这类会话里的应用也能起来，
好把验证跑完。`start` 只在会话总线上确实没有 watcher 时才拉它；`--no-watcher` 可以关掉
（用来复现「没有 watcher 时应用能不能活」）。替身不是系统组件，`stop` 会一并收掉。

依赖：python3-pyatspi（AT-SPI 相关子命令）；watcher 替身另需 python3-gi；jq 可选。
"""

import glob
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

APP_HINT = "purevox"
DEBUG_URL = "http://127.0.0.1:47821/debug"
DEFAULT_BINARY = "/usr/bin/purevox"
WATCHER_NAME = "org.kde.StatusNotifierWatcher"
STATE_PATH = os.path.join(tempfile.gettempdir(), "purevox-ui-state.json")

# 安装方式 → 该认的产物后缀（与 src/update/mod.rs 的 ASSET_SUFFIXES 同一份约定；
# 对不上不会报任何错，只是自动更新静默失效，所以这里要断言）。
KIND_SUFFIX = {
    "deb": ".deb",
    "rpm": ".rpm",
    "appimage": ".AppImage",
    "nsis": "-setup.exe",
}

_pyatspi = None


def _atspi():
    """惰性导入：只有 AT-SPI 那几个子命令才需要它。"""
    global _pyatspi
    if _pyatspi is None:
        try:
            import pyatspi  # type: ignore
        except ImportError:
            sys.exit("缺少 python3-pyatspi（sudo apt install python3-pyatspi）")
        _pyatspi = pyatspi
    return _pyatspi


# ---------------------------------------------------------------- AT-SPI 部分


def _frames():
    pyatspi = _atspi()
    desk = pyatspi.Registry.getDesktop(0)
    for app in desk:
        if app and APP_HINT in (app.name or "").lower():
            for c in app:
                yield c


def _walk(n):
    yield n
    try:
        for c in n:
            yield from _walk(c)
    except Exception:
        return


def _name(n):
    try:
        return n.name or ""
    except Exception:
        return ""


def find(text, exact=True):
    out = []
    for f in _frames():
        for n in _walk(f):
            nm = _name(n)
            if (nm == text) if exact else (text in nm):
                out.append(n)
    return out


def do_click(n):
    try:
        a = n.queryAction()
    except Exception as e:
        return f"（该控件不支持动作：{e}）"
    for i in range(a.nActions):
        if a.getName(i) in ("click", "press", "activate", "jump"):
            a.doAction(i)
            return f"action={a.getName(i)}"
    a.doAction(0)
    return "action=默认"


def cmd_list():
    for f in _frames():
        print(f"窗口：{_name(f)}")
        for n in _walk(f):
            nm = _name(n)
            if nm.strip():
                try:
                    role = n.getRoleName()
                except Exception:
                    role = "?"
                print(f"  [{role}] {nm!r}")


def cmd_click(text):
    nodes = find(text)
    if not nodes:
        print(f"找不到控件：{text!r}")
        near = find(text, exact=False)
        if near:
            print("近似匹配：")
            for n in near[:20]:
                print(f"  [{n.getRoleName()}] {_name(n)!r}")
        return 1
    n = nodes[0]
    print(f"点击 [{n.getRoleName()}] {text!r} {do_click(n)}")
    return 0


# ------------------------------------------------------- 会话环境 / HTTP 读数


def session_env():
    """补齐桌面会话环境变量；调用方已经设了一律尊重（CI / 已在会话里跑时不必猜）。"""
    env = dict(os.environ)
    rt = env.get("XDG_RUNTIME_DIR")
    if not rt:
        rt = f"/run/user/{os.getuid()}"
        if os.path.isdir(rt):
            env["XDG_RUNTIME_DIR"] = rt
    if not env.get("DBUS_SESSION_BUS_ADDRESS"):
        bus = os.path.join(rt, "bus")
        if os.path.exists(bus):
            env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={bus}"
    if not env.get("WAYLAND_DISPLAY"):
        socks = [p for p in glob.glob(os.path.join(rt, "wayland-*")) if not p.endswith(".lock")]
        if socks:
            pref = os.path.join(rt, "wayland-0")
            env["WAYLAND_DISPLAY"] = "wayland-0" if pref in socks else os.path.basename(socks[0])
    if not env.get("DISPLAY"):
        for n in (0, 1, 2):
            if os.path.exists(f"/tmp/.X11-unix/X{n}"):
                env["DISPLAY"] = f":{n}"
                break
    return env


def http_json(url=DEBUG_URL, timeout=5):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def debug_alive():
    try:
        http_json(timeout=2)
        return True
    except Exception:
        return False


def wait_debug(timeout=30.0):
    end = time.time() + timeout
    while time.time() < end:
        if debug_alive():
            return True
        time.sleep(0.5)
    return False


def watcher_present(env):
    """会话总线上有没有 SNI watcher（有 dbus-send 就直接问总线）。"""
    if not shutil.which("dbus-send"):
        return False
    r = subprocess.run(
        [
            "dbus-send", "--session", "--print-reply",
            "--dest=org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus.NameHasOwner", f"string:{WATCHER_NAME}",
        ],
        env=env, capture_output=True, text=True, timeout=10,
    )
    return "boolean true" in (r.stdout or "")


# ------------------------------------------------------------- watcher 替身


def serve_watcher():
    """前台跑一个最小 SNI watcher（只够 ksni 注册用）。"""
    from gi.repository import Gio, GLib

    xml = """
    <node>
      <interface name="org.kde.StatusNotifierWatcher">
        <method name="RegisterStatusNotifierItem">
          <arg name="service" type="s" direction="in"/>
        </method>
        <method name="RegisterStatusNotifierHost">
          <arg name="service" type="s" direction="in"/>
        </method>
        <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
        <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
        <property name="ProtocolVersion" type="i" access="read"/>
        <signal name="StatusNotifierItemRegistered"><arg name="service" type="s"/></signal>
        <signal name="StatusNotifierItemUnregistered"><arg name="service" type="s"/></signal>
        <signal name="StatusNotifierHostRegistered"/>
        <signal name="StatusNotifierHostUnregistered"/>
      </interface>
    </node>
    """
    node = Gio.DBusNodeInfo.new_for_xml(xml)
    items = []

    def on_method(bus, sender, path, iface, method, params, invocation):
        args = params.unpack()
        if method == "RegisterStatusNotifierItem":
            items.append(str(args[0]))
            print(f"[watcher] 收到注册：{args[0]}", flush=True)
            invocation.return_value(None)
        elif method == "RegisterStatusNotifierHost":
            invocation.return_value(None)
        else:
            invocation.return_error_literal(
                Gio.io_error_quark(), Gio.IOErrorEnum.NOT_SUPPORTED, "not supported"
            )

    def on_get_property(bus, sender, path, iface, prop):
        if prop == "IsStatusNotifierHostRegistered":
            return GLib.Variant("b", True)
        if prop == "ProtocolVersion":
            return GLib.Variant("i", 0)
        if prop == "RegisteredStatusNotifierItems":
            return GLib.Variant("as", items)
        return None

    bus = Gio.bus_get_sync(Gio.BusType.SESSION)
    bus.register_object("/StatusNotifierWatcher", node.interfaces[0], on_method, on_get_property, None)
    Gio.bus_own_name_on_connection(bus, WATCHER_NAME, Gio.BusNameOwnerFlags.NONE, None, None)
    print(f"[watcher] 上线：{WATCHER_NAME}（替身，验证用）", flush=True)
    GLib.MainLoop().run()
    return 0


# ------------------------------------------------------------ start / stop


def load_state():
    try:
        with open(STATE_PATH, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def save_state(st):
    with open(STATE_PATH, "w", encoding="utf-8") as f:
        json.dump(st, f, ensure_ascii=False)


def alive(pid):
    if not pid:
        return False
    try:
        os.kill(int(pid), 0)
        return True
    except Exception:
        return False


def terminate(pid, label):
    if not alive(pid):
        return
    pid = int(pid)
    # 起进程时用了 start_new_session，pid 就是进程组组长：整组一起收。
    # `--private-bus` 起的是 `dbus-run-session -- purevox`，只杀包装进程会把真应用留下。
    pgid = None
    try:
        pgid = os.getpgid(pid)
    except Exception:
        pass
    group = pgid == pid

    def bang(sig):
        try:
            if group:
                os.killpg(pgid, sig)
            else:
                os.kill(pid, sig)
        except Exception:
            pass

    bang(signal.SIGTERM)
    for _ in range(20):
        if not alive(pid):
            print(f"  已收：{label}（pid={pid}）")
            return
        time.sleep(0.25)
    bang(signal.SIGKILL)
    print(f"  强杀：{label}（pid={pid}{'，整组' if group else ''}，TERM 不理）")


def cmd_start(binary=DEFAULT_BINARY, use_watcher=True, timeout=30.0, private_bus=False):
    if debug_alive():
        print("已有实例在跑（/debug 通）——先 `ui.py stop` 或自己收掉，免得抢 47821 端口")
        return 2
    if not os.path.exists(binary):
        print(f"找不到可执行文件：{binary}")
        return 2
    env = session_env()
    log = os.path.join(tempfile.gettempdir(), "purevox-ui-app.log")
    st = {"binary": binary, "log": log}
    cmd = [binary]
    if private_bus:
        # 私有会话总线：里面**没有**任何 SNI watcher。用来复现「无 watcher」场景，也是
        # 「托盘注册失败不该让应用起不来」的对照面（老版本在这种总线里启动即 panic）。
        if not shutil.which("dbus-run-session"):
            print("--private-bus 需要 dbus-run-session（dbus-daemon 包）")
            return 2
        env.pop("DBUS_SESSION_BUS_ADDRESS", None)
        cmd = ["dbus-run-session", "--", binary]
        print("  --private-bus：另起一条私有会话总线（没有 watcher）")
    elif use_watcher and not watcher_present(env):
        wlog = os.path.join(tempfile.gettempdir(), "purevox-ui-watcher.log")
        w = subprocess.Popen(
            [sys.executable, os.path.abspath(__file__), "watcher"],
            env=env, stdout=open(wlog, "w"), stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL, start_new_session=True,
        )
        st["watcher_pid"] = w.pid
        print(f"  会话总线上没有 {WATCHER_NAME} → 顶了个替身（pid={w.pid}，日志 {wlog}）")
        time.sleep(1.5)
    elif not use_watcher:
        print("  --no-watcher：不顶替身（用来复现「没有 watcher」的场景）")
    p = subprocess.Popen(
        cmd, env=env, stdout=open(log, "w"), stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL, start_new_session=True,
    )
    st["app_pid"] = p.pid
    print(f"  启动 {binary}（pid={p.pid}，日志 {log}）")
    if not wait_debug(timeout):
        print(f"  等 /debug 超时（{timeout:.0f}s）——应用日志尾部：")
        for line in tail(log, 12):
            print(f"    {line}")
        terminate(p.pid, "应用")
        if st.get("watcher_pid"):
            terminate(st["watcher_pid"], "watcher 替身")
        return 1
    save_state(st)
    print(f"  /debug 就绪：{DEBUG_URL}")
    return 0


def cmd_stop():
    st = load_state()
    if not st:
        print("  没有记录（这台机器上最近不是本工具起的应用）；什么都没做")
        return 0
    terminate(st.get("app_pid"), f"应用（{st.get('binary')}）")
    terminate(st.get("watcher_pid"), "watcher 替身")
    try:
        os.remove(STATE_PATH)
    except OSError:
        pass
    print("  收尾完成")
    return 0


def tail(path, n=10):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read().splitlines()[-n:]
    except OSError:
        return []


# ------------------------------------------------------------------ take


def cmd_take(expr=None):
    try:
        data = http_json()
    except Exception as e:
        print(f"读不到 {DEBUG_URL}：{e}（应用没起来？）")
        return 2
    if not expr:
        print(json.dumps(data, ensure_ascii=False, indent=2))
        return 0
    if not shutil.which("jq"):
        print("给了 jq 表达式但没装 jq；直接不带参数打印整段再自己筛")
        return 2
    r = subprocess.run(["jq", expr], input=json.dumps(data), text=True)
    return r.returncode


# ---------------------------------------------------------- verify-update


def open_update_panel():
    """打开「更新」面板（已开就别再点一次顶栏，那会把面板关掉）。"""
    if find("检查更新"):
        return True
    if not find("更新"):
        print("  找不到顶栏「更新」按钮——AT-SPI 只看得见**同一条会话总线**上的实例："
              "应用若是 `--private-bus` 起的（或界面根本没起来），换成会话总线里的实例再试")
        return False
    cmd_click("更新")
    for _ in range(10):
        time.sleep(0.5)
        if find("检查更新"):
            return True
    print("  点了「更新」但没出现「检查更新」")
    return False


def cmd_verify_update(kind):
    kind = (kind or "").lower()
    if kind != "none" and kind not in KIND_SUFFIX:
        print(f"未知安装方式：{kind}（可用：{', '.join(KIND_SUFFIX)}、none=开发构建）")
        return 2
    if not debug_alive():
        print("没有应用在跑：先 `ui.py start`（或自己起）")
        return 2
    before = (http_json().get("update") or {})
    print(f"  当前：{before.get('current_version')}（{before.get('current_stamp')}）"
          f" install_kind={before.get('install_kind')!r}")
    if not open_update_panel():
        return 1
    rc = cmd_click("检查更新")
    if rc != 0:
        return rc
    settled = {"available", "up_to_date", "unsupported", "failed", "ready", "installed"}

    def landed(ph, up):
        # 开发构建在「没有 CI 时间戳」处就停下：phase 仍是 idle，但给出原因 —— 也算落定。
        # （不能靠「原因变了」判断：第二次点同一颗按钮，原因跟上次一模一样。）
        if ph in settled:
            return True
        return kind == "none" and ph == "idle" and bool(up.get("reason"))

    up, end = {}, time.time() + 45
    while time.time() < end:
        up = (http_json().get("update") or {})
        if landed(up.get("phase"), up):
            break
        time.sleep(0.5)
    phase = up.get("phase")
    print(f"  phase={phase} reason={up.get('reason')!r}")
    if not landed(phase, up):
        print("  等检查更新落定超时（是不是没网 / api.github.com 不通）")
        return 1
    ok = True
    if phase == "failed":
        print(f"  ✗ 检查失败：{up.get('reason')}")
        return 1
    if kind == "none":
        # 开发构建两条支路：① 没有 CI 时间戳 → 不判断（phase 仍 idle，只给原因）；
        # ② 有时间戳但没有 bundle 标记 → unsupported。
        if phase == "unsupported":
            pass
        elif phase == "idle":
            pass
        else:
            print(f"  ✗ 开发构建本应 idle（不判断）或 unsupported，实际 {phase}")
            ok = False
        if ok and not up.get("reason"):
            print("  ✗ 没给原因（AGENTS：不静默失败）")
            ok = False
        elif ok:
            print(f"  ✓ 开发构建按约定不判断/不给下载：{up.get('reason')}")
    else:
        if up.get("install_kind") != kind:
            print(f"  ✗ install_kind={up.get('install_kind')!r}，期望 {kind!r}")
            ok = False
        name = up.get("asset_name") or ""
        if not name.endswith(KIND_SUFFIX[kind]):
            print(f"  ✗ 选中的产物 {name!r} 不是 {KIND_SUFFIX[kind]}（跨格式兜底会装不上）")
            ok = False
        if phase == "available":
            if up.get("latest_tag") == up.get("current_stamp"):
                print(f"  ✗ 有更新但 latest_tag 与本机时间戳相同：{up.get('latest_tag')}")
                ok = False
            print(f"  ✓ 本机=deb 类安装：{up.get('current_version')} → "
                  f"{up.get('latest_version')}（{up.get('latest_tag')}）产物 {name} "
                  f"{up.get('asset_size')} 字节")
        elif phase == "up_to_date":
            print(f"  ✓ 已是最新（{up.get('current_version')}）；产物仍按 {kind} 匹配：{name}")
        elif phase == "unsupported":
            print(f"  ✗ {kind} 安装本应能更新，却报 unsupported：{up.get('reason')}")
            ok = False
    print("  结论：" + ("通过" if ok else "不通过"))
    return 0 if ok else 1


# -------------------------------------------------------------------- main

USAGE = """用法：
  ui.py list | click <控件名> | sel <下拉项>          # 纯 AT-SPI
  ui.py start [--binary <路径>] [--no-watcher] [--private-bus] [--timeout <秒>]
  ui.py take [jq 表达式]                              # 读 /debug
  ui.py verify-update <deb|rpm|appimage|none>         # 检查更新并断言产物
  ui.py stop                                          # 收掉本工具起的进程
  ui.py watcher                                       # 顶一个托盘 watcher 替身（前台）
"""


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    op, rest = argv[1], argv[2:]
    if op == "list":
        cmd_list()
        return 0
    if op in ("click", "sel"):
        if not rest:
            print("用法：ui.py click <控件名>")
            return 2
        # AT-SPI 的下拉选项在展开后才出现；点击前若目标正是某个 <option>，直接点它
        return cmd_click(rest[0])
    if op == "watcher":
        return serve_watcher()
    if op == "start":
        binary = DEFAULT_BINARY
        use_watcher = True
        private_bus = False
        timeout = 30.0
        i = 0
        while i < len(rest):
            if rest[i] == "--binary" and i + 1 < len(rest):
                binary = rest[i + 1]
                i += 2
            elif rest[i] == "--no-watcher":
                use_watcher = False
                i += 1
            elif rest[i] == "--private-bus":
                private_bus = True
                i += 1
            elif rest[i] == "--timeout" and i + 1 < len(rest):
                timeout = float(rest[i + 1])
                i += 2
            else:
                print(USAGE)
                return 2
        return cmd_start(binary, use_watcher, timeout, private_bus)
    if op == "stop":
        return cmd_stop()
    if op == "take":
        return cmd_take(rest[0] if rest else None)
    if op == "verify-update":
        if not rest:
            print("用法：ui.py verify-update <deb|rpm|appimage|none>")
            return 2
        return cmd_verify_update(rest[0])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
