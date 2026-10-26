"""独立探针：电脑播放一条扫频、耳机麦克风同时录，**按频窗峰值**量延时。

不依赖 PureVox 进程（不用 /debug，也不用 exe 里的校准）：PyAudio 直接开
「Headphone 播放 + Headset Microphone 录音」，播的扫频我们自己有（far = 播放信号本身），
然后用**特定频率窗口的峰值时刻**对齐两条信号：

    扫频是单调的 → 频率 = 时间的标签。取一段窄频窗（默认 2.0–3.0 kHz），
    在 far 与 mic 的「窗内能量随时间的曲线」上各找峰值位置，
    两个位置的差就是这一段频率从耳机到麦克风的延时。

用法（默认就是你那对 Realtek 耳机/麦克风，设备名可从 --list 里挑）：

    python tools/automation/aec_probe.py --list
    python tools/automation/aec_probe.py --play 17 --rec 19 --out %TEMP%/probe.png

退出码：0 = 量到（相关窗内 SNR 够），2 = 麦克风没听到探针（图上能看出来）。
"""

from __future__ import annotations

import argparse
import math
import os
import sys

import numpy as np

SR = 48000
F0, F1 = 1500.0, 2500.0      # 窄带扫频（与 src-tauri/src/engine/calib.rs 的 PROBE_F0/F1 一致）
DUR_MS = 120                 # 扫频时长
LEAD_MS = 400                # 播放前的静音（给录音一点底噪样本）
TAIL_MS = 900                # 播放后的余量（要能容下几十~几百 ms 的延时）
AMP = 0.6


def make_probe() -> np.ndarray:
    """1500→2500 Hz 对数扫频，120 ms，Hann 包络（与引擎同一形状，方便对图）。"""
    n = int(SR * DUR_MS / 1000)
    t = np.arange(n) / SR
    k = math.log(F1 / F0) / (n / SR)
    phase = 2 * np.pi * np.cumsum(F0 * np.exp(k * t)) / SR
    env = 0.5 - 0.5 * np.cos(2 * np.pi * np.arange(n) / n)
    return (AMP * env * np.sin(phase)).astype(np.float32)


def record(play_idx: int, rec_idx: int) -> tuple[np.ndarray, np.ndarray]:
    """返回 (mic, far)：far 是「实际送去播放的样本」（放在同一时间轴上）。"""
    import pyaudio

    probe = make_probe()
    pa = pyaudio.PyAudio()
    block = 480
    out = pa.open(format=pyaudio.paFloat32, channels=1, rate=SR, output=True,
                  output_device_index=play_idx, frames_per_buffer=block)
    inp = pa.open(format=pyaudio.paFloat32, channels=1, rate=SR, input=True,
                  input_device_index=rec_idx, frames_per_buffer=block)

    lead = np.zeros(int(SR * LEAD_MS / 1000), dtype=np.float32)
    tail = np.zeros(int(SR * TAIL_MS / 1000), dtype=np.float32)

    # 先起录音（丢掉第一块，避免打开时的咔哒），再一边播一边录
    inp.read(block, exception_on_overflow=False)
    rec: list[np.ndarray] = []
    buf = np.concatenate([lead, probe, tail])
    for i in range(0, len(buf), block):
        chunk = buf[i : i + block]
        if len(chunk) < block:
            chunk = np.pad(chunk, (0, block - len(chunk)))
        out.write(chunk.tobytes())
        rec.append(np.frombuffer(inp.read(block, exception_on_overflow=False), dtype=np.float32))

    out.stop_stream()
    out.close()
    inp.stop_stream()
    inp.close()
    pa.terminate()

    mic = np.concatenate(rec)
    far = buf[: len(mic)]
    return mic, far



def smooth_db(f: np.ndarray, db: np.ndarray, octave: float = 1 / 6) -> np.ndarray:
    """1/6 倍频程平滑（对数频率上取窗，dB 域平均）。

    麦克风那一路的幅度谱是「探针 + 房间 + 底噪」混在一起，单看 FFT 是一条毛刺，
    峰值淹在锯齿里；按倍频程平滑之后就剩一条干净的包络，峰在哪一眼可见。
    """
    out = np.empty_like(db)
    ratio = 2 ** (octave / 2)
    for i, fc in enumerate(f):
        lo, hi = fc / ratio, fc * ratio
        m = (f >= lo) & (f <= hi)
        out[i] = db[m].mean() if m.any() else db[i]
    return out

def band_energy(x: np.ndarray, lo: float, hi: float, win: int = 1024, hop: int = 128):
    """逐帧算 [lo,hi] 频窗内的能量，返回 (时间轴, 能量)。"""
    n = (len(x) - win) // hop + 1
    w = np.hanning(win)
    f = np.fft.rfftfreq(win, 1 / SR)
    m = (f >= lo) & (f <= hi)
    out = np.empty(n)
    for i in range(n):
        seg = x[i * hop : i * hop + win] * w
        spec = np.abs(np.fft.rfft(seg)) ** 2
        out[i] = spec[m].sum()
    t = (np.arange(n) * hop + win / 2) / SR
    return t, out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", action="store_true", help="列出设备后退出")
    ap.add_argument("--play", type=int, default=17, help="播放设备序号（默认 Realtek Headphone）")
    ap.add_argument("--rec", type=int, default=19, help="录音设备序号（默认 Realtek Headset Microphone）")
    ap.add_argument("--band", default="1500,2500", help="用于峰值匹配的频窗 Hz，如 1500,2500")
    ap.add_argument("--out", default=os.path.join(os.environ.get("TEMP", "."), "probe.png"))
    args = ap.parse_args()

    if args.list:
        import pyaudio

        pa = pyaudio.PyAudio()
        for i in range(pa.get_device_count()):
            d = pa.get_device_info_by_index(i)
            if d["maxInputChannels"] or d["maxOutputChannels"]:
                kind = "out" if d["maxOutputChannels"] else "in "
                print(f"{i:3d} {kind} {d['name']}  ({int(d['defaultSampleRate'])} Hz)")
        pa.terminate()
        return 0

    lo, hi = (float(v) for v in args.band.split(","))

    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    plt.rcParams["font.family"] = ["Microsoft YaHei", "DejaVu Sans"]
    plt.rcParams["axes.unicode_minus"] = False

    mic, far = record(args.play, args.rec)
    np.save(os.path.join(os.path.dirname(args.out) or ".", "probe_mic.npy"), mic)
    np.save(os.path.join(os.path.dirname(args.out) or ".", "probe_far.npy"), far)
    db = lambda v: 20 * np.log10(max(float(v), 1e-12))

    tf, ef = band_energy(far, lo, hi)
    tm, em = band_energy(mic, lo, hi)
    pf = int(np.argmax(ef))
    pm = int(np.argmax(em))
    delay_ms = (tm[pm] - tf[pf]) * 1000.0

    # 窗内 SNR：拿 mic 窗内峰值跟窗外的中位数比（只看这一路，判断"有没有听到"）
    floor = np.median(em)
    snr = 10 * np.log10(em[pm] / max(floor, 1e-20))

    def spec(x):
        w = np.hanning(len(x))
        s = np.abs(np.fft.rfft(x * w)) / (len(x) / 4)
        f = np.fft.rfftfreq(len(x), 1 / SR)
        return f, 20 * np.log10(np.maximum(s, 1e-12))

    fm, sm = spec(mic)
    ff, sf = spec(far)

    fig, ax = plt.subplots(figsize=(12, 6), constrained_layout=True)
    ax.plot(ff, sf, lw=1.0, color="#e08a1e", alpha=0.55, label="far（播放的扫频）")
    ax.plot(fm, sm, lw=0.8, color="#2f7bd6", alpha=0.35, label="mic（原始）")
    ax.plot(fm, smooth_db(fm, sm), lw=2.0, color="#12457f", label="mic（1/6 倍频程平滑）")
    ax.axvspan(lo, hi, color="#999", alpha=0.12, label=f"峰值匹配频窗 {lo:.0f}–{hi:.0f} Hz")
    ax.set_xlim(100, 12000)
    ax.set_ylim(-140, 0)
    ax.set_xlabel("频率 (Hz)")
    ax.set_ylabel("dBFS")
    ax.set_title(f"频谱：mic vs far（{lo:.0f}–{hi:.0f} Hz 窗内峰值时刻相差 "
                 f"{delay_ms:+.1f} ms；mic 窗内比底噪高 {snr:.1f} dB）")
    ax.grid(alpha=0.3)
    ax.legend()

    fig.savefig(args.out, dpi=110)
    print(f"图 -> {args.out}")
    print(f"far 峰值 {tf[pf] * 1000:.1f} ms，mic 峰值 {tm[pm] * 1000:.1f} ms → 延时 {delay_ms:+.1f} ms")
    print(f"mic 窗内峰值比底噪高 {snr:.1f} dB")
    print(f"mic peak {db(np.abs(mic).max()):.1f} dBFS / rms {db(np.sqrt((mic ** 2).mean())):.1f} dBFS")
    return 0 if snr >= 6.0 else 2


if __name__ == "__main__":
    sys.exit(main())
