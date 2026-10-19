"""Synthesises Butters' sound effects: soft, warm, short, quiet.

Run: python3 scripts/make_sounds.py   ->   src-tauri/sounds/*.wav

Each sound is timed to its animation (see src/lib/motion.ts and Row.svelte):
  open    560 ms  panel pours in: a soft low "plop" with a gentle tail
  tick     80 ms  checkbox: a tiny muted tap, like a knife on butter
  melt   1000 ms  row melts: a quiet pan sizzle that swells, then a drip at ~800 ms
  freed  1300 ms  savings card: a warm two-note chime (major sixth), round and low
  close   220 ms  panel slumps away: a short, soft downward "fwip"
All are mixed far below full scale (peaks around -18 dBFS) so they never startle.
No samples, no licences: everything is generated here.
"""
import math
import random
import struct
import wave
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "sounds"
SR = 44100
random.seed(7)


def env(n, attack, release, curve=2.0):
    a = max(1, int(attack * SR))
    r = max(1, int(release * SR))
    out = []
    for i in range(n):
        if i < a:
            g = (i / a) ** 0.5
        elif i > n - r:
            g = ((n - i) / r) ** curve
        else:
            g = 1.0
        out.append(g)
    return out


def lowpass(x, cutoff):
    rc = 1 / (2 * math.pi * cutoff)
    a = (1 / SR) / (rc + 1 / SR)
    y, prev = [], 0.0
    for v in x:
        prev += a * (v - prev)
        y.append(prev)
    return y


def highpass(x, cutoff):
    rc = 1 / (2 * math.pi * cutoff)
    a = rc / (rc + 1 / SR)
    y, prev_y, prev_x = [], 0.0, 0.0
    for v in x:
        prev_y = a * (prev_y + v - prev_x)
        prev_x = v
        y.append(prev_y)
    return y


def sine_sweep(dur, f0, f1, shape=2.0):
    n = int(dur * SR)
    phase, out = 0.0, []
    for i in range(n):
        t = i / n
        f = f1 + (f0 - f1) * (1 - t) ** shape
        phase += 2 * math.pi * f / SR
        out.append(math.sin(phase))
    return out


def bell(dur, freq, decay):
    """Soft, round tone: fundamental plus a whisper of the octave."""
    n = int(dur * SR)
    out = []
    for i in range(n):
        t = i / SR
        g = math.exp(-t / decay)
        v = math.sin(2 * math.pi * freq * t) + 0.18 * math.sin(2 * math.pi * freq * 2 * t) * math.exp(-t / (decay * 0.4))
        out.append(v * g)
    return out


def mix(*parts, length=None):
    n = length or max(len(p) + off for p, off, _ in parts)
    out = [0.0] * n
    for p, off, gain in parts:
        for i, v in enumerate(p):
            j = i + off
            if j < n:
                out[j] += v * gain
    return out


def ms(x):
    return int(x / 1000 * SR)


def normalise(x, peak_db=-18.0):
    peak = max(1e-9, max(abs(v) for v in x))
    g = 10 ** (peak_db / 20) / peak
    # 4 ms fades so nothing clicks
    f = ms(4)
    out = [v * g for v in x]
    for i in range(min(f, len(out))):
        out[i] *= i / f
        out[-1 - i] *= i / f
    return out


def write(name, x, peak_db=-18.0):
    x = normalise(x, peak_db)
    OUT.mkdir(parents=True, exist_ok=True)
    with wave.open(str(OUT / f"{name}.wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, v)) * 32767)) for v in x))


def plop(dur=0.16, f0=520, f1=170):
    body = sine_sweep(dur, f0, f1, shape=3.0)
    e = env(len(body), 0.004, dur * 0.85, curve=2.5)
    return lowpass([b * g for b, g in zip(body, e)], 1800)


def sizzle(dur):
    n = int(dur * SR)
    noise = [random.uniform(-1, 1) for _ in range(n)]
    # crackle: sparse little pops riding on soft hiss
    for i in range(n):
        if random.random() < 0.0016:
            for k in range(60):
                if i + k < n:
                    noise[i + k] += math.sin(k / 3) * math.exp(-k / 14) * 2.2
    hiss = highpass(lowpass(noise, 5200), 1400)
    # swells as the butter softens, fades as it pools
    shape = [math.sin(math.pi * min(1, (i / n) * 1.05)) ** 1.6 for i in range(n)]
    return [h * s for h, s in zip(hiss, shape)]


# open: soft plop, landing with the panel at ~180 ms into its 560 ms pour
write("open", mix((plop(0.2, 440, 150), ms(30), 1.0)), peak_db=-20)

# tick: tiny muted tap
tap = sine_sweep(0.05, 900, 600)
write("tick", lowpass([v * g for v, g in zip(tap, env(len(tap), 0.002, 0.045, 3))], 2400), peak_db=-24)

# melt: sizzle across the 1000 ms melt, drip lands as the drips fall (~800 ms)
write(
    "melt",
    mix((sizzle(0.95), 0, 0.55), (plop(0.12, 700, 260), ms(800), 0.9), length=ms(1000)),
    peak_db=-22,
)

# freed: warm chime as the savings card plops in, second note as the number lands
c5, a5 = 523.25, 880.0
write("freed", mix((plop(0.14, 380, 160), 0, 0.6), (bell(1.1, c5, 0.35), ms(60), 0.8), (bell(1.1, a5, 0.4), ms(240), 0.55)), peak_db=-18)

# close: short soft downward fwip, matching the 220 ms slump
fw = sine_sweep(0.2, 360, 140, shape=1.4)
fwn = lowpass([random.uniform(-1, 1) for _ in fw], 900)
write("close", lowpass([(a * 0.8 + b * 0.5) * g for a, b, g in zip(fw, fwn, env(len(fw), 0.01, 0.17, 2))], 1500), peak_db=-24)

print("sounds written to", OUT)
