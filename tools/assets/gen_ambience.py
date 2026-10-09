"""Generate seamless ocean/wind ambience loops and a whoosh as 16-bit WAV (stdlib only).

Usage: python -I tools/assets/gen_ambience.py [out-dir]   (default: assets/sfx)
Writes ocean_loop.wav (stereo, 24 s), wind_loop.wav (stereo, 24 s), whoosh.wav (mono, 0.35 s).
Loops are seamless: modulators complete whole cycles per loop and the noise tail is crossfaded into the head.
"""
import math
import pathlib
import random
import sys
import wave
from array import array

SR = 22050
XFADE = SR  # 1 s crossfade


def cyclic(cycles, i, n):
    """Weighted sum of sinusoids with whole cycles per loop, scaled to 0..1."""
    t = 2 * math.pi * i / n
    return sum(w * (0.5 + 0.5 * math.sin(c * t + p)) for c, p, w in cycles) / sum(w for _, _, w in cycles)


def loop(buf, n):
    """Equal-power crossfade of buf[n:] into buf[:len(buf)-n] so buf[:n] repeats seamlessly."""
    x = len(buf) - n
    out = buf[:n]
    for i in range(x):
        w = i / x
        out[i] = buf[i] * math.sqrt(w) + buf[n + i] * math.sqrt(1 - w)
    return out


def ocean(n, seed, phase):
    rnd = random.Random(seed)
    swells = [(3, phase, 1.0), (5, 1.3 + phase, 0.6), (8, 2.1, 0.35)]
    brown = body = wide = 0.0
    buf = []
    for i in range(n + XFADE):
        s = cyclic(swells, i, n)
        w = rnd.uniform(-1, 1)
        brown = 0.997 * brown + 0.03 * w
        body += 0.08 * (w - body)
        wide += 0.5 * (w - wide)
        buf.append(brown * (0.3 + 0.5 * s) + body * (0.1 + 2.2 * s ** 3) + 0.35 * (w - wide) * s ** 5)
    return loop(buf, n)


def wind(n, seed, phase):
    rnd = random.Random(seed)
    gusts = [(2, phase, 1.0), (5, 2.0, 0.5), (11, 0.9 + phase, 0.25)]
    lo = hi = 0.0
    buf = []
    for i in range(n + XFADE):
        g = cyclic(gusts, i, n)
        w = rnd.uniform(-1, 1)
        a = 0.015 + 0.09 * g
        lo += a * (w - lo)
        hi += 0.2 * a * (w - hi)
        buf.append((lo - hi) * (0.2 + g * g) / a ** 0.5)
    return loop(buf, n)


def whoosh(seed=7):
    rnd = random.Random(seed)
    n = int(0.35 * SR)
    a1 = a2 = 0.0
    out = []
    for i in range(n):
        e = math.sin(math.pi * i / n)
        a = 0.02 + 0.35 * e
        a1 += a * (rnd.uniform(-1, 1) - a1)
        a2 += a * (a1 - a2)
        out.append(a2 * e * e / a ** 0.5)
    return out


def write(path, chans, peak=0.7):
    g = peak * 32767 / max(abs(v) for c in chans for v in c)
    data = array("h", (int(v * g) for frame in zip(*chans) for v in frame))
    if sys.byteorder == "big":
        data.byteswap()
    with wave.open(str(path), "wb") as f:
        f.setnchannels(len(chans))
        f.setsampwidth(2)
        f.setframerate(SR)
        f.writeframes(data.tobytes())


def check_seamless(chan):
    """The jump across the loop seam must look like any other sample step."""
    mean_step = sum(abs(chan[i + 1] - chan[i]) for i in range(len(chan) - 1)) / (len(chan) - 1)
    seam = abs(chan[0] - chan[-1])
    assert seam < 8 * mean_step, (seam, mean_step)


def _selftest():
    assert loop([1.0, 2.0, 3.0, 4.0, 5.0], 4) == [5.0, 2.0, 3.0, 4.0]
    assert abs(cyclic([(3, 0.4, 1.0)], 0, 100) - cyclic([(3, 0.4, 1.0)], 100, 100)) < 1e-9


def main(out_dir):
    out_dir.mkdir(parents=True, exist_ok=True)
    n = 24 * SR
    for name, gen in (("ocean_loop", ocean), ("wind_loop", wind)):
        chans = [gen(n, seed, phase) for seed, phase in ((1, 0.0), (2, 0.35))]
        for c in chans:
            check_seamless(c)
        write(out_dir / f"{name}.wav", chans)
    write(out_dir / "whoosh.wav", [whoosh()], peak=0.8)


if __name__ == "__main__":
    _selftest()
    main(pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2] / "assets" / "sfx")
