# PureVox Lite Denoise Only — 入口
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 零复用主线，仅复用自包含库 (onnxruntime, numpy, pyaudio)
# 运行即启动，无启停

import os
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
                    # 需先创建隐藏 root
                    import tkinter as tk
                    r = tk.Tk()
                    r.withdraw()
                    mb.showerror("PureVox Lite", "PureVox 已在运行（完整版或轻量版），不可同时启动。")
                    r.destroy()
                except Exception:
                    pass
                sys.exit(0)
            return mutex
        except Exception:
            return None
    else:
        # Linux: 文件锁
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
            import winreg
            key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run", 0, winreg.KEY_SET_VALUE | winreg.KEY_QUERY_VALUE)
            name = "PureVox"
            if enable:
                exe = sys.executable
                # lite 入口
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

def main():
    mutex = ensure_single_instance()
    # 独立配置
    import os as _os, sys as _sys
    _sys.path.insert(0, _os.path.dirname(__file__))
    from config import load, save
    import audio
    import engine

    cfg = load()
    ins, outs = audio.list_devices()

    # 默认设备回退
    if not cfg.get("input_device") and ins:
        cfg["input_device"] = ins[0][0]
    if not cfg.get("output_device") and outs:
        cfg["output_device"] = outs[0][0]

    # 模型常驻（仓库根 models/；冻结态在 _MEIPASS/models/）
    def _find_model():
        rel = os.path.join("models", "purevox_denoise_202609_ep0000.onnx")
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
        try:
            import tkinter.messagebox as mb
            import tkinter as tk
            r = tk.Tk(); r.withdraw()
            mb.showerror("模型加载失败", str(e))
            r.destroy()
        except Exception:
            print(e)
        sys.exit(1)

    # 解析设备索引：兼容旧配置（无 [API] 前缀）按后缀匹配，支持 (disp,idx,props) 或 (disp,idx)
    def _to_map(lst):
        mp = {}
        for item in lst:
            if len(item) == 3:
                n,i,_ = item
                mp[n]=i
            else:
                n,i = item
                mp[n]=i
        return mp
    def _idx_list(lst):
        out=[]
        for item in lst:
            if len(item)==3:
                _,i,_=item
                out.append(i)
            else:
                _,i=item
                out.append(i)
        return out
    in_map = _to_map(ins)
    out_map = _to_map(outs)
    def resolve(name, mp, lst):
        if not name:
            vals = list(mp.values())
            return vals[0] if vals else -1
        if name in mp:
            return mp[name]
        for k, v in mp.items():
            if k.endswith(name) or name.endswith(k):
                return v
        for k, v in mp.items():
            if name in k or k in name:
                return v
        vals = list(mp.values())
        return vals[0] if vals else -1
    in_idx = resolve(cfg.get("input_device", ""), in_map, ins)
    out_idx = resolve(cfg.get("output_device", ""), out_map, outs)
    valid_in = set(_idx_list(ins))
    valid_out = set(_idx_list(outs))
    if in_idx not in valid_in and ins:
        in_idx = _idx_list(ins)[0]
    if out_idx not in valid_out and outs:
        out_idx = _idx_list(outs)[0]

    stream = None
    def start_stream():
        nonlocal stream
        # 同 API 校验：WASAPI/MME 混用即非法，仅提示不自动改输出
        ok, msg = audio.check_api_match(in_idx, out_idx)
        if not ok:
            try:
                import tkinter.messagebox as mb
                mb.showerror("组合非法", msg + "\n请将输入与输出设为同一 API。")
            except Exception:
                print(msg)
            return
        if stream:
            try:
                stream.stop()
            except Exception:
                pass
        stream = audio.LiteAudioStream(in_idx, out_idx, eng, cfg.get("pre_gain_db", 0.0), cfg.get("post_gain_db", 0.0))
        try:
            stream.start()
        except Exception as e:
            # 组合非法细化提示
            emsg = str(e)
            if "Invalid" in emsg or "非法" in emsg or "Unanticipated" in emsg:
                emsg = emsg + "\n提示：WASAPI 与 MME 不能混用，请将输入/输出设为同一 API。"
            try:
                import tkinter.messagebox as mb
                mb.showerror("音频启动失败", emsg)
            except Exception:
                print(emsg)
            # 不退出，允许改设备

    ui = None
    # UI 回调（主窗口与托盘双向同步，比例档位）
    def on_gain(which, val):
        iv = int(val)
        if which == "pre":
            cfg["pre_gain_db"] = iv
        else:
            cfg["post_gain_db"] = iv
        save(cfg)
        if stream:
            stream.set_gains(cfg["pre_gain_db"], cfg["post_gain_db"])
        try:
            if ui:
                # 同步主窗口数字框
                if which == "pre":
                    ui.pre_var.set(str(iv))
                else:
                    ui.post_var.set(str(iv))
        except Exception:
            pass

    def on_device(in_name, out_name):
        nonlocal in_idx, out_idx
        cfg["input_device"] = in_name
        cfg["output_device"] = out_name
        save(cfg)
        # 精确或后缀匹配
        if in_name in in_map:
            in_idx = in_map[in_name]
        else:
            for k, v in in_map.items():
                if k.endswith(in_name) or in_name.endswith(k):
                    in_idx = v
                    break
        if out_name in out_map:
            out_idx = out_map[out_name]
        else:
            for k, v in out_map.items():
                if k.endswith(out_name) or out_name.endswith(k):
                    out_idx = v
                    break
        start_stream()

    def on_autostart(enable):
        cfg["autostart"] = bool(enable)
        save(cfg)
        set_autostart(enable)

    # 启动即运行
    start_stream()

    # Tk UI 黑底白字（无系统标题栏，自绘）
    from ui import LiteUI

    def _hide_window():
        try:
            ui.root.withdraw()
        except Exception:
            pass
    def _show_window():
        try:
            ui.root.deiconify()
            ui.root.lift()
            ui.root.focus_force()
        except Exception:
            pass
    def _do_close():
        _hide_window()
    def _do_minimize():
        _hide_window()

    ui = LiteUI(cfg, ins, outs, on_gain, on_device, on_autostart, on_close=_do_close, on_minimize=_do_minimize)

    # 统一退出路径：托盘退出与无托盘关窗共用同一份清理逻辑
    def _shutdown():
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
        _p = _icon_ico_path()
        # 挡位与 ui.RES_GEARS 输出对齐；「自动」按屏幕分辨率定挡。
        # 约束：托盘回调在独立线程，Tk 调用必须 after(0) 投递回主线程；
        # 勾选态在右键弹出菜单那一刻现算，动态跟随当前缩放配置。
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
            _p, "PureVoxLiteMicTray", "PureVox Lite — 运行中",
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
    # 仅当窗口被最小化（iconic）时才隐藏，避免 withdraw 触发误隐藏
    ui.root.bind("<Unmap>", lambda e: on_close() if ui.root.state() == "iconic" else None)

    # 开机自启已配置：启动即隐藏到托盘，静默运行
    if cfg.get("autostart"):
        ui.root.after(100, _hide_window)

    ui.run()
    if tray:
        tray.stop()

if __name__ == "__main__":
    main()
