"""Envelope (10 ms RMS, dB) of reference vs Lumen for selected probes, every 50 ms."""
import sys, numpy as np
from harmonics import read
for name in sys.argv[2:]:
    r, sr = read(f"{sys.argv[1]}/reference/{name}.wav"); l, _ = read(f"{sys.argv[1]}/lumen/{name}.wav")
    def env(x):
        m = x.mean(axis=1); f = int(0.01 * sr)
        return np.array([10 * np.log10((m[i:i+f]**2).mean() + 1e-14) for i in range(0, len(m) - f, f)])
    er, el = env(r), env(l)
    el = el - (el[:150].max() - er[:150].max())  # level-match on the peak
    pts = range(0, min(len(er), len(el)), 5)
    print(name); print("  t(s) ", " ".join(f"{i*0.01:5.2f}" for i in pts if i < 200))
    print("  serum", " ".join(f"{er[i]:5.1f}" for i in pts if i < 200)); print("  lumen", " ".join(f"{el[i]:5.1f}" for i in pts if i < 200))
