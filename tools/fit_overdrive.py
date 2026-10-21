"""Overdrive and tape sat against input level (tools/make_probes.mjs `fxod-` set).

usage: fit_overdrive.py RUN_DIR [--lumen]

Each probe plays the sub's sine at C1 through 0–3 FX Utility stages, which set one of
12 input levels, then one distortion mode and drive; `dry` is the same chain without
it. Levels are inside the rack (the master volume, applied after the effects, is
divided out). Prints for overdrive, per drive and level, the input peak (dB), the
gain of its linear part (dB; the output is inverted) and the output peak; for tape
sat, per drive and level, how far its curve y = F(x) lies from the quieter levels'
where both reach (rms, relative to the output peak): a static curve stays near 0.
"""
import json, re, sys
from collections import defaultdict
from pathlib import Path
import numpy as np
from fit_distortion import GRID, POINTS, pairs, segment


def db(v):
    return 20 * np.log10(max(v, 1e-9))


def binned(x, y):
    edges = np.concatenate([[-np.inf], (GRID[1:] + GRID[:-1]) / 2, [np.inf]])
    idx = np.searchsorted(edges, x) - 1
    cnt = np.bincount(idx, None, POINTS)
    val = np.where(cnt > 0, np.bincount(idx, y, POINTS) / np.maximum(cnt, 1), np.nan)
    return np.where(np.abs(GRID) <= np.abs(x).max() * 0.985, val, np.nan)


def main():
    args = sys.argv[1:]
    run = Path(args[0])
    sub = "lumen" if "--lumen" in args else "reference"
    probes = defaultdict(dict)  # level -> setting -> job id
    for j in json.loads((run / "jobs.json").read_text())["jobs"]:
        m = re.search(r"_fxod_(u[\d_x]+)_(.+)$", j["id"])
        if m:
            probes[m[1]][m[2]] = j["id"]
    wav = lambda jid: segment(run / sub / f"{jid}.wav")
    dry = {lv: wav(s["dry"]) for lv, s in probes.items()}
    levels = sorted(probes, key=lambda lv: np.abs(dry[lv]).max())
    for mode in ["overdrive", "tapesat"]:
        drives = sorted({int(k.split("_")[1]) for k in probes[levels[0]] if k.startswith(mode)})
        for d in drives:
            merged = np.full(POINTS, np.nan)
            for lv in levels:
                x, y = pairs(dry[lv], wav(probes[lv][f"{mode}_{d}"]))
                a, peak = np.abs(x).max(), np.abs(y).max()
                if mode == "overdrive":
                    lin = np.abs(y) < 0.7 * peak
                    g = -(x[lin] * y[lin]).sum() / max((x[lin] ** 2).sum(), 1e-12)
                    print(f"overdrive d{d:3} in {db(a):6.1f} dB  gain {db(g):6.1f} dB  out peak {peak:.3f}")
                else:
                    c = binned(x, y)
                    both = ~np.isnan(c) & ~np.isnan(merged)
                    dev = np.sqrt(np.mean((c[both] - merged[both]) ** 2)) / peak if both.any() else float("nan")
                    print(f"tapesat   d{d:3} in {db(a):6.1f} dB  out peak {peak:.3f}  off the quieter levels {dev:.3f}")
                    merged = np.where(np.isnan(merged), c, merged)


if __name__ == "__main__":
    main()
