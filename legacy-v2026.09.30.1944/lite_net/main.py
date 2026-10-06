# PureVox Lite Net Only — 入口（网络输入 → 降噪 → 本地输出）
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 零复用主线，仅复用自包含库 (onnxruntime, numpy, pyaudio, websockets, av,
# cryptography, zeroconf)。运行即启动 WSS 服务，浏览器/Android 客户端推流，
# 协议与主线一致（JSON + base64 opus + ack）

import sys
import ctypes

# 单实例：与完整版共用互斥名 PureVox，避免同时运行
def ensure_single_instance():
    if sys.platform.startswith("win"):
        try:
            kernel32 = ctypes.windll.kernel32
            mutex = kernel32.CreateMutexW(None, 0, "PureVox")
            err = kernel32.GetLastError()
            # ERROR_ALREADY_EXISTS = 183
            if err == 183:
                try:
                    import tkinter.messagebox as mb
                    import tkinter as tk
                    r = tk.Tk()
                    r.withdraw()
                    mb.showerror("PureVox Net Lite", "PureVox 已在运行（完整版或轻量版），不可同时启动。")
                    r.destroy()
                except Exception:
                    pass
                sys.exit(0)
            return mutex
        except Exception:
            return None
    else:
        import os
        import fcntl
        path = os.path.join(os.path.expanduser("~"), ".purevox", "purevox.lock")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fp = open(path, "w")
        try:
            fcntl.flock(fp, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except Exception:
            print("PureVox 已在运行")
            sys.exit(0)
        return fp

def set_autostart(enable):
    if sys.platform.startswith("win"):
        try:
            import os
            import winreg
            key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run", 0, winreg.KEY_SET_VALUE | winreg.KEY_QUERY_VALUE)
            name = "PureVox"
            if enable:
                exe = sys.executable
                script = os.path.join(os.path.dirname(__file__), "main.py")
                cmd = f'"{exe}" "{script}"'
                winreg.SetValueEx(key, name, 0, winreg.REG_SZ, cmd)
            else:
                try:
                    winreg.DeleteValue(key, name)
                except FileNotFoundError:
                    pass
            winreg.CloseKey(key)
        except Exception as e:
            print("autostart fail", e)

def _die(msgbox_title, msg):
    try:
        import tkinter.messagebox as mb
        import tkinter as tk
        r = tk.Tk()
        r.withdraw()
        mb.showerror(msgbox_title, msg)
        r.destroy()
    except Exception:
        print(msg)
    sys.exit(1)

def main():
    ensure_single_instance()
    import os
    import asyncio
    import threading
    sys.path.insert(0, os.path.dirname(__file__))
    # 仓库根：复用主线 server/（WSS/TLS/mDNS/Opus）与 pvengine 的环形缓冲
    _root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    if _root not in sys.path:
        sys.path.insert(0, _root)
    from config import load, save
    from model_config import DENOISE_MODEL
    import audio
    import engine
    import playback
    from pvplatform import netinfo
    from server.https_server import PureVoxServer
    from server.audio_bridge import RemoteAudioSource
    from pvengine.dsp.ring_buffer import RingBuffer as NetRing

    cfg = load()
    outs = audio.list_output_devices()

    # 默认输出回退（按主线同一名字模糊匹配恢复保存的设备）
    def _out_map(lst):
        return {item[0]: item[1] for item in lst}
    out_map = _out_map(outs)
    def resolve_out(name):
        matched = audio.best_name_match(name, list(out_map.keys()))
        if matched is not None:
            return out_map[matched]
        vals = list(out_map.values())
        return vals[0] if vals else -1

    # 模型常驻（仓库根 models/；冻结态在 _MEIPASS/models/）——文件名取自 model_config
    def _find_model():
        rel = DENOISE_MODEL
        meipass = getattr(sys, "_MEIPASS", None)
        cands = []
        if meipass:
            cands.append(os.path.join(meipass, rel))
        cands.append(os.path.join(
            os.path.dirname(os.path.dirname(os.path.abspath(__file__))), rel))
        cands.append(rel)
        for c in cands:
            if os.path.isfile(c):
                return c
        return cands[-1]

    try:
        eng = engine.LiteDenoiseEngine(_find_model())
    except Exception as e:
        _die("模型加载失败", str(e))

    # 跨时钟域播放缓冲 + 增益（闭包持有，网络写入/设备回调共享）
    ring = playback.PlaybackBuffer()
    gains = {"pre": audio.db_to_linear(cfg.get("pre_gain_db", 0.0)),
             "post": audio.db_to_linear(cfg.get("post_gain_db", 0.0))}

    def process_fn(chunk):
        # 网络帧与引擎 hop 对齐（480=10ms）：前增益 → 引擎（任意到达长度仍逐 hop 切）
        x = chunk * gains["pre"]
        return eng.process(x)

    stream = None
    def start_stream():
        nonlocal stream
        if stream:
            try:
                stream.stop()
            except Exception:
                pass
            stream = None
        out_idx = resolve_out(cfg.get("output_device", ""))
        if out_idx < 0 and not outs:
            _die("无可用输出设备", "未检测到 WASAPI 输出设备。")
            return
        stream = audio.LiteNetStream(out_idx, ring, post_db=cfg.get("post_gain_db", 0.0))
        try:
            stream.start()
        except Exception as e:
            emsg = str(e)
            try:
                import tkinter.messagebox as mb
                mb.showerror("音频启动失败", emsg)
            except Exception:
                print(emsg)
            stream = None

    # 启动即运行
    start_stream()

    # ── 网络：复用主线 PureVoxServer（WSS/HTTPS + Opus 解码 + TLS + mDNS）──
    from ui import LiteUI
    ui_holder = {}
    import time as _time

    # 网络输入源桥（主线实现）。注入 pvengine 线程安全环形缓冲，避免拉起
    # audio_processor（→ pyaudio/pvplatform/pvengine 全栈）。
    source = RemoteAudioSource(ring_cls=NetRing)
    reader_stop = threading.Event()

    def _network_reader():
        """读取网络 PCM → 逐 hop 降噪 → 播放缓冲（对齐主线 _network_reader）。

        flush / 突发硬顶 / 断流补零；速率差稳态由 PlaybackBuffer 伺服消化。
        """
        import numpy as _np
        MAX_ACC = audio.HOP * 8            # 硬上限 ~80ms
        TARGET_ACC = audio.HOP * 5         # 目标 ~50ms
        STALL_TIMEOUT = 0.15
        acc = []
        last = _time.time()
        while not reader_stop.is_set():
            if source.flush_event.is_set():
                source.flush_event.clear()
                acc.clear()
                ring.reset()
            n = source.available()
            if n > 0:
                chunk = source.read(n)
                if chunk:
                    acc.extend(chunk)
                    last = _time.time()
            if len(acc) > MAX_ACC:
                acc[:] = acc[-TARGET_ACC:]
            if _time.time() - last > STALL_TIMEOUT and 0 < len(acc) < audio.HOP:
                fade = min(64, len(acc))
                for i in range(fade):
                    acc[-fade + i] *= 1.0 - (i + 1) / (fade + 1)
                acc.extend([0.0] * (audio.HOP - len(acc)))
            if len(acc) < audio.HOP:
                _time.sleep(0.002)
                continue
            hop = _np.asarray(acc[:audio.HOP], dtype=_np.float32)
            del acc[:audio.HOP]
            try:
                out = process_fn(hop)
                _np.clip(out, -1.0, 1.0, out=out)
                ring.write(out)
            except Exception:
                pass

    # 默认广播网卡：配置保存值 > 自动选择（首个非 TUN 物理口）
    networks = netinfo.list_lan_ips()
    sel = cfg.get("net_ip")
    selected_ip = sel if sel in [i for i, _n in networks] else netinfo.best_lan_ip(networks)

    port = int(cfg.get("port", 8765))
    server = PureVoxServer(port=port, audio_source=source)
    server_loop = asyncio.new_event_loop()

    def _run_server_loop():
        asyncio.set_event_loop(server_loop)
        try:
            server_loop.run_until_complete(server.start())
            server.apply_network(selected_ip)   # 启动后按选中网卡重注册 mDNS
            server_loop.run_forever()
        except Exception as e:
            print("server loop error:", e)

    threading.Thread(target=_run_server_loop, daemon=True).start()
    threading.Thread(target=_network_reader, daemon=True).start()

    def on_net_state(clients, note):
        ui = ui_holder.get("ui")
        if ui:
            ui.set_server_state(clients, note)

    def _state_poll():
        while True:
            _time.sleep(1.0)
            on_net_state(source.active_clients, "")

    threading.Thread(target=_state_poll, daemon=True).start()

    # 防火墙零逻辑：WSS 开始监听即触发系统「安全中心警报」，点允许即放行；
    # 「重启」按钮重开监听会再次触发，无需任何主动检查/安装代码

    def on_gain(which, val):
        iv = int(val)
        if which == "pre":
            cfg["pre_gain_db"] = iv
            gains["pre"] = audio.db_to_linear(iv)
        else:
            cfg["post_gain_db"] = iv
            gains["post"] = audio.db_to_linear(iv)
            if stream:
                stream.set_post_gain(iv)
        save(cfg)
        ui = ui_holder.get("ui")
        try:
            if ui:
                if which == "pre":
                    ui.pre_var.set(str(iv))
                else:
                    ui.post_var.set(str(iv))
        except Exception:
            pass

    def on_output(out_name):
        cfg["output_device"] = out_name
        save(cfg)
        start_stream()

    def apply_network(ip):
        """切网统一路径（用户下拉切换与自动跟随共用）：
        保存选择 → 主线服务端按当前网卡重签证书并热加载 + mDNS 换接口重注册"""
        cfg["net_ip"] = ip
        save(cfg)
        server.apply_network(ip)

    def on_network(ip):
        # 用户手动切换网卡：mDNS/证书跟随即可
        apply_network(ip)

    def on_autostart(enable):
        cfg["autostart"] = bool(enable)
        save(cfg)
        set_autostart(enable)

    def _hide_window():
        ui = ui_holder.get("ui")
        try:
            ui.root.withdraw()
        except Exception:
            pass
    def _show_window():
        ui = ui_holder.get("ui")
        try:
            ui.root.deiconify()
            ui.root.lift()
            ui.root.focus_force()
        except Exception:
            pass
    def _do_close():
        _hide_window()

    ui = LiteUI(cfg, outs, on_gain, on_output, on_autostart, on_close=_do_close, on_minimize=_do_close,
                networks=networks, on_network=on_network)
    ui.set_server_state(source.active_clients, "")
    ui_holder["ui"] = ui

    # 网卡自动跟随：低频轮询本机 IPv4，网卡集合或选中 IP 变化时
    # 自动跟随（证书/mDNS/二维码统一走 apply_network）并刷新下拉列表
    def start_net_watch():
        state = {"prev": None}

        def _watch():
            while True:
                _time.sleep(5)
                try:
                    nets = netinfo.list_lan_ips()
                except Exception:
                    continue
                ips = [i for i, _n in nets]
                cur = frozenset(ips)
                if cur == state["prev"]:
                    continue
                state["prev"] = cur
                sel = cfg.get("net_ip")
                # 选中 IP 仍有效则不动（启动时 mDNS 已按其注册），失效才自动改选
                want = sel if sel in ips else netinfo.best_lan_ip(nets)
                if want and want != sel:
                    apply_network(want)
                u = ui_holder.get("ui")
                if u:
                    try:
                        u.root.after(0, lambda nn=nets: u.set_networks(nn))
                    except Exception:
                        pass
        threading.Thread(target=_watch, daemon=True).start()

    start_net_watch()

    def on_restart():
        # 手动重启：WSS 重开监听 + mDNS 重注册 + 下拉/状态刷新（异常恢复路径）
        def _run():
            err = ""
            try:
                server.restart()      # 主线服务端：停后原端口重启（线程安全）
            except Exception as e:
                err = str(e)
            u = ui_holder.get("ui")
            if u:
                try:
                    u.root.after(0, lambda: (u.set_networks(netinfo.list_lan_ips()),
                                             u.set_server_state(source.active_clients, err)))
                except Exception:
                    pass
        threading.Thread(target=_run, daemon=True).start()

    ui.on_restart = on_restart

    # 统一退出路径：托盘退出与无托盘关窗共用同一份清理逻辑
    def _shutdown():
        reader_stop.set()
        try:
            asyncio.run_coroutine_threadsafe(server.stop(), server_loop).result(3)
        except Exception:
            pass
        try:
            if stream:
                stream.stop()
        except Exception:
            pass
        try:
            ui.root.destroy()
        except Exception:
            pass
        os._exit(0)

    # 系统托盘：与主线同一原语（零依赖 ctypes Shell_NotifyIcon），创建即校验。
    # 三原则：NIM_ADD 结果在构造返回前同步可知；explorer 重启经 TaskbarCreated
    # 事件自动重加；关窗只在图标确实存活时隐藏，否则整体退出——僵尸进程不可能。
    # 图标 = 仓库资产 assets/icons/lite_tray.ico（开发/冻结/打包同源）。
    def _icon_ico_path():
        rel = os.path.join("assets", "icons", "lite_tray.ico")
        meipass = getattr(sys, "_MEIPASS", None)
        cands = []
        if meipass:
            cands.append(os.path.join(meipass, rel))
        cands.append(os.path.join(
            os.path.dirname(os.path.dirname(os.path.abspath(__file__))), rel))
        cands.append(rel)
        for c in cands:
            if os.path.isfile(c):
                return c
        return cands[-1]

    tray = None
    try:
        import tray as tray_mod

        # 挡位与 ui.RES_GEARS 输出对齐；「自动」按屏幕分辨率定挡。
        # 约束：托盘回调在独立线程，Tk 调用必须 after(0) 投递回主线程；
        # 勾选态改为弹出菜单时现算（与 lite_mic 同构）。
        def _tk(fn):
            def _run():
                try:
                    ui.root.after(0, fn)
                except Exception:
                    fn()
            return _run

        def menu_builder():
            auto = bool(cfg.get("auto_zoom", True))
            pct = int(getattr(ui, "_zoom", 100))
            gears = [{"label": f"{p}%",
                      "checked": (not auto and p == pct),
                      "cb": _tk(lambda p=p: ui.set_zoom(p))}
                     for p in (85, 95, 100, 110, 125, 145, 175)]
            return [
                {"label": "显示主界面", "default": True,
                 "cb": lambda: _show_window()},
                None,
                {"label": "缩放比例",
                 "sub": [{"label": "自动（按分辨率）",
                          "checked": auto,
                          "cb": _tk(ui.set_auto_zoom)}] + gears},
                None,
                {"label": "退出", "cb": _shutdown},
            ]

        tray = tray_mod.make_tray(
            _icon_ico_path(), "PureVoxLiteNetTray", "PureVox Net Lite — 运行中",
            lambda: _show_window(), _shutdown, menu_builder)
    except Exception as e:
        print("tray init fail:", e)
        tray = None

    # 关窗策略跟随真实托盘状态：有存活图标才隐藏到托盘，否则干净退出
    def on_close():
        if tray is not None and tray.alive:
            _hide_window()
        else:
            _shutdown()

    ui.root.protocol("WM_DELETE_WINDOW", on_close)
    ui.root.bind("<Unmap>", lambda e: on_close() if ui.root.state() == "iconic" else None)

    # 开机自启已配置：启动即隐藏到托盘，静默运行
    if cfg.get("autostart"):
        ui.root.after(100, _hide_window)

    ui.run()
    if tray:
        tray.stop()

if __name__ == "__main__":
    main()
