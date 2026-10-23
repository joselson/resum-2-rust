"""Folds, sine shaper and X-shaper laws (tools/make_probes.mjs `fxd4-` set, run with the
rare modes: linfold, sinfold, sineshaper, xshaper).

usage: fit_folds.py RUN_DIR

Prints the measurements behind the constants in tools/fit_distortion.py:
- the high-pass after the sine shaper and X-shaper: a one-pole fitted to the default
  X-shaper on the quietest range, where its curve is linear (y = HP(g·x));
- the folds: output zeros at input levels x_n with ln(1 + G·x_n) = n·ln(r), r fixed per
  mode, G per drive (and its ratio to G at drive 0);
- the sine shaper (high-pass undone): zeros evenly spaced in x, ω = π / spacing per
  drive (and its ratio to drive 0).
Levels are inside the rack (output ÷ master gain); the chain's latency is 6 samples.
"""
import json, sys
from pathlib import Path
import numpy as np
from scipy.signal import lfilter
from filter_response import read_wav
from fit_distortion import DRIVES, LATENCY, MASTER, one_pole_hp, segment


class Run:
    def __init__(self, path):
        self.path = path
        jobs = json.loads((path / "jobs.json").read_text())["jobs"]
        self.ids = {j["id"].split("_fxd4_", 1)[1]: j["id"] for j in jobs if "_fxd4_" in j["id"]}
        self.dry = [self.load(k) for k in ("off", "boost_1", "boost_2")]

    def full(self, key):
        y, rate = read_wav(self.path / "reference" / f"{self.ids[key]}.wav")
        return y[:, 0] / MASTER, rate

    def load(self, key, undo_hp=False):
        return segment(self.path / "reference" / f"{self.ids[key]}.wav", undo_hp)

    def zeros(self, key, undo_hp=False, floor=0.1):
        """Input levels x > floor where the output crosses zero, each from the quietest range."""
        out, reach = [], 0.0
        for b in range(3):
            x, y = self.dry[b][:-LATENCY], self.load(f"{key}_b{b}", undo_hp)[LATENCY:]
            s = np.nonzero((y[:-1] < 0) != (y[1:] < 0))[0]
            xz = x[s] + y[s] / (y[s] - y[s + 1]) * (x[s + 1] - x[s])
            a = np.abs(x).max()
            xz = np.sort(xz[(xz > max(reach * 0.98, floor)) & (xz < a * 0.97)])
            groups = np.split(xz, np.nonzero(np.diff(xz) > 0.004 * a)[0] + 1) if len(xz) else []
            out += [float(np.median(g)) for g in groups if len(g) >= 3]
            reach = a
        return np.array(sorted(out))


def fit_hp(run):
    x, rate = run.full("off")
    y, _ = run.full("xshaper_0_b0")
    x = np.concatenate([np.zeros(LATENCY), x[:-LATENCY]])
    seg = slice(int(0.05 * rate), int(0.55 * rate))
    best = None
    for fc in np.arange(8, 16, 0.01):
        h = lfilter(*one_pole_hp(fc, rate), x)[seg]
        g = (h @ y[seg]) / (h @ h)
        e = np.sqrt(np.mean((g * h - y[seg]) ** 2)) / np.abs(y[seg]).max()
        if best is None or e < best[0]:
            best = (e, fc, g)
    print(f"high-pass {best[1]:.2f} Hz, gain {best[2]:.4f}, residual {best[0]:.5f} of peak")


def fold_gain(z, lr):
    """G minimising the distance of ln(1 + G·z)/ln r from integers."""
    best = None
    for g in np.exp(np.linspace(np.log(0.003), np.log(300), 8000)):
        n = np.log1p(g * z) / lr
        e = np.mean((n - np.round(n)) ** 2)
        if np.round(n).min() >= 1 and (best is None or e < best[0]):
            best = (e, g)
    return best


def fit_fold(run, mode, r0):
    zs = {d: run.zeros(f"{mode}_{d}") for d in DRIVES}
    rows = []
    for r in np.arange(r0 - 0.002, r0 + 0.002001, 0.0002):
        fits = [fold_gain(zs[d], np.log(r)) for d in DRIVES]
        rows.append((np.mean([f[0] for f in fits]), r, [f[1] for f in fits]))
    err, r, gs = min(rows)
    print(f"{mode}: r {r:.4f} (misfit {err:.5f}); G by drive " + " ".join(f"{g:.4g}" for g in gs))
    print(f"  G / G(0) − 1 over (drive/100)³: " + " ".join(f"{(g / gs[0] - 1) / (d / 100) ** 3:.0f}" for d, g in zip(DRIVES[1:], gs[1:])))


def fit_sine(run):
    """ω per drive from antiperiodicity: y(x + π/ω) = −y(x) on the fine static curve."""
    grid = np.linspace(-10, 10, 20001)
    ws = []
    for d in DRIVES:
        curve, reach = np.full(len(grid), np.nan), 0.0
        for b in range(3):
            x, y = run.dry[b][:-LATENCY], run.load(f"sineshaper_{d}_b{b}", undo_hp=True)[LATENCY:]
            idx = np.clip(np.searchsorted(grid, x), 0, len(grid) - 1)
            cnt = np.bincount(idx, None, len(grid))
            val = np.bincount(idx, y, len(grid)) / np.maximum(cnt, 1)
            new = (cnt > 0) & np.isnan(curve) & (np.abs(grid) > reach * 0.98)
            curve[new] = val[new]
            reach = np.abs(x).max()
        ok = ~np.isnan(curve)
        curve = np.interp(grid, grid[ok], curve[ok])
        step = grid[1] - grid[0]
        best = None
        for w in np.linspace(0.6, 10, 9401):
            k = int(round(np.pi / w / step))
            e = np.mean((curve[k:] + curve[:-k]) ** 2)
            if best is None or e < best[0]:
                best = (e, np.pi / (k * step))
        ws.append(best[1])
    print("sine shaper ω by drive " + " ".join(f"{w:.4f}" for w in ws))
    print("  ω / ω(0) − 1 over (drive/100)²: " + " ".join(f"{(w / ws[0] - 1) / (d / 100) ** 2:.2f}" for d, w in zip(DRIVES[1:], ws[1:])))


def main():
    run = Run(Path(sys.argv[1]))
    fit_hp(run)
    fit_fold(run, "linfold", 1.081)
    fit_fold(run, "sinfold", 1.25)
    fit_sine(run)


if __name__ == "__main__":
    main()
