"""Distortion transfer tables from a probe run (tools/make_probes.mjs `fxd4-` set).

usage: fit_distortion.py RUN_DIR... OUT.rs

Each probe plays the sub's sine at C1 through one distortion mode and drive, with
0, 1 or 2 FX Utility stages at +12 dB in front (inputs up to ±0.23, ±0.94, ±3.76).
Pairing every output sample with the dry input sample gives the static curve
y = F(x), the output taken LATENCY samples late: the chain's latency, which every
tabled mode's quietest probe shows by cross-correlation. It is fixed rather than
correlated per mode: densely folded or rectified output correlates poorly with its
input, and a few samples off open a loop that looks like level-dependent folding (tape
sat). The modes in the table are memoryless (a quieter
input traces the same curve within binning noise). Curves from the three ranges are
merged, each grid point taken from the quietest probe that reaches it, and sampled
on an asinh-spaced grid. The probes run at the default master volume, which is
applied after the effects, so levels are divided by its gain (0.25·√2) to give the
curve inside the rack. Writes a Rust table of i16 values (1/SCALE steps), plus the
laws of the modes tabled otherwise (NOT_TABLED) and the sine shaper's F(s) table.
Several runs may be given (probes-fxd4 for the common modes, probes-fxd5 for the rare
ones); each needs its own off/boost probes.
"""
import json, re, sys
from pathlib import Path
import numpy as np
from scipy.signal import lfilter
from filter_response import read_wav

MASTER = 0.25 * np.sqrt(2)
V0, VMAX, POINTS = 0.03, 3.7 / MASTER, 257
W = np.linspace(-np.arcsinh(VMAX / V0), np.arcsinh(VMAX / V0), POINTS)
GRID = V0 * np.sinh(W)
DRIVES = [0, 10, 25, 40, 50, 60, 75, 90, 100]
# Steps per unit: i16 then holds ±8 (drive 0 passes x/2 up to ±5.2).
SCALE = 4096
NAMES = ["kTube", "kSoftClip", "kHardClip", "kDiode1", "kDiode2", "kAsym", "kRectify", "kStompBox", "kTapeSat", "kOverdrive", "kSoftSat", "kZeroSquare"]
# Not tabled here: overdrive compresses its input by the signal level (Lumen models it
# from tools/fit_overdrive.py); the folds and the sine shaper are static but fold too
# densely for this grid, and the X-shaper (like the sine shaper) is followed by a
# high-pass. Their laws come from tools/fit_folds.py.
NOT_TABLED = {"overdrive", "linfold", "sinfold", "sineshaper", "xshaper", "xshaperasym"}
# Laws measured by tools/fit_folds.py. The one-pole high-pass after the sine shaper and
# X-shaper (Hz); the output peak of the folds, rectify and the X-shaper's clip; the
# folds' argument ln(1 + G·|x|)/ln(r) with G = G0·(1 + FOLD_DRIVE·(drive/100)³), as
# (G0, r); the sine shaper's ω = ω0·(1 + k·(drive/100)²), as (ω0, k).
HP_HZ = 11.95
PEAK = 0.83
FOLD_DRIVE = 2130
LIN_FOLD = (0.0235, 1.081)
SIN_FOLD = (0.06709, 1.25)
SINE_W = (0.738, 11.83)
# The sine shaper is F(sin(ω·x)), F odd: tabled on |sin| = 0…1.
S_POINTS = 65
LATENCY = 6


def one_pole_hp(fc, rate):
    """(b, a) of the one-pole high-pass Serum puts after the sine shaper and X-shaper."""
    p = np.exp(-2 * np.pi * fc / rate)
    return [(1 + p) / 2, -(1 + p) / 2], [1, -p]


def segment(path, undo_hp=False):
    x, r = read_wav(path)
    x = x[:, 0]
    if undo_hp:
        # Inverted from the silent start, so the integration constant is exact.
        b, a = one_pole_hp(HP_HZ, r)
        x = lfilter(a, b, x)
    return x[int(0.25 * r):int(0.55 * r)] / MASTER


MAXLAG = 200


def lag_of(dry, wet):
    corr = [abs(np.dot(dry[MAXLAG:-MAXLAG], wet[MAXLAG + L:len(wet) - MAXLAG + L])) for L in range(-MAXLAG, MAXLAG + 1)]
    return int(np.argmax(corr)) - MAXLAG


def pairs(dry, wet, lag=None):
    """Input and output samples, the output shifted by `lag` (default: its own best)."""
    lag = lag_of(dry, wet) if lag is None else lag
    return dry[MAXLAG:-MAXLAG], wet[MAXLAG + lag:len(wet) - MAXLAG + lag]


def curve(sources):
    """sources: [(dry, wet)] from quietest to loudest."""
    edges = np.concatenate([[-np.inf], (GRID[1:] + GRID[:-1]) / 2, [np.inf]])
    out = np.full(POINTS, np.nan)
    for dry, wet in sources:
        x, y = pairs(dry, wet, LATENCY)
        idx = np.searchsorted(edges, x) - 1
        cnt = np.bincount(idx, None, POINTS)
        val = np.where(cnt > 0, np.bincount(idx, y, POINTS) / np.maximum(cnt, 1), np.nan)
        reach = np.abs(GRID) <= np.abs(dry).max() * 0.985
        out = np.where(np.isnan(out) & reach, val, out)
    ok = ~np.isnan(out)
    return np.interp(W, W[ok], out[ok])


def sine_table(dry, wet):
    """The sine shaper's F(s) per drive on s = 0…1. dry: the three ranges; wet(drive, b):
    that probe with the high-pass undone. Each bin from the quietest range reaching it
    (the loudest folds too densely and comes out smeared)."""
    table = []
    for d in DRIVES:
        w = SINE_W[0] * (1 + SINE_W[1] * (d / 100) ** 2)
        val = np.full(S_POINTS, np.nan)
        for b in range(3):
            x, y = pairs(dry[b], wet(d, b), LATENCY)
            s = np.sin(w * x)
            idx = np.rint(np.abs(s) * (S_POINTS - 1)).astype(int)
            cnt = np.bincount(idx, None, S_POINTS)
            mean = np.bincount(idx, y * np.sign(s), S_POINTS) / np.maximum(cnt, 1)
            val = np.where(np.isnan(val) & (cnt > 0), mean, val)
        val[0] = 0.0
        ok = ~np.isnan(val)
        table.append(np.interp(np.arange(S_POINTS), np.nonzero(ok)[0], val[ok]))
    return table


def canonical(probe_name):
    """Serum's mode name for a lower-cased probe name ("tapesat" → "kTapeSat")."""
    return next(n for n in NAMES if n[1:].lower() == probe_name)


def main():
    *runs, dst = map(Path, sys.argv[1:])
    tables, sine = {}, None
    for run in runs:
        ids = {j["id"].split("_fxd4_", 1)[1]: j["id"] for j in json.loads((run / "jobs.json").read_text())["jobs"] if "_fxd4_" in j["id"]}
        wav = lambda key: segment(run / "reference" / f"{ids[key]}.wav")
        dry = [wav("off"), wav("boost_1"), wav("boost_2")]
        for m in sorted({re.match(r"(.+)_\d+_b\d$", k).group(1) for k in ids if re.match(r".+_\d+_b\d$", k)} - NOT_TABLED):
            tables[m] = [curve([(dry[b], wav(f"{m}_{d}_b{b}")) for b in range(3)]) for d in DRIVES]
        if "sineshaper_0_b0" in ids:
            sine = sine_table(dry, lambda d, b: segment(run / "reference" / f"{ids[f'sineshaper_{d}_b{b}']}.wav", undo_hp=True))
    modes = sorted(tables)
    for m in modes:
        print(m, "peak", round(float(np.abs(np.array(tables[m])).max()), 3), file=sys.stderr)
    lines = [
        "//! Generated by tools/fit_distortion.py from Serum renders (fxd4 probes): do not edit.",
        "//! Static distortion curves y = F(mode, drive, x), sampled at x = V0·sinh(w) on an",
        "//! even w grid, as i16 in 1/SCALE steps, at levels inside the FX rack.",
        "",
        f"pub const V0: f32 = {V0};",
        f"pub const SCALE: f32 = {SCALE}.0;",
        f"pub const POINTS: usize = {POINTS};",
        f"pub const W_MAX: f32 = {float(W[-1]):.6f};",
        f"pub const DRIVES: [f32; {len(DRIVES)}] = {[float(d) for d in DRIVES]};",
        f"pub const MODES: [&str; {len(modes)}] = [{', '.join(chr(34) + canonical(m) + chr(34) for m in modes)}];",
        "",
        "// Laws from tools/fit_folds.py (see tools/fit_distortion.py).",
        f"pub const HP_HZ: f32 = {HP_HZ};",
        f"pub const PEAK: f32 = {PEAK};",
        f"pub const FOLD_DRIVE: f32 = {FOLD_DRIVE}.0;",
        f"pub const LIN_FOLD: (f32, f32) = {LIN_FOLD};",
        f"pub const SIN_FOLD: (f32, f32) = {SIN_FOLD};",
        f"pub const SINE_W: (f32, f32) = {SINE_W};",
        f"pub const S_POINTS: usize = {S_POINTS};",
        "/// The sine shaper's F(s) per drive on s = |sin(ω·x)| from 0 to 1 (F is odd).",
        f"pub static SINE_SHAPER: [[i16; S_POINTS]; {len(DRIVES)}] = [",
        *["    [" + ", ".join(map(str, np.round(np.asarray(t) * SCALE).astype(int))) + "]," for t in sine],
        "];",
        "",
        f"pub static CURVES: [[[i16; POINTS]; {len(DRIVES)}]; {len(modes)}] = [",
    ]
    for m in modes:
        lines.append(f"    // {m}")
        lines.append("    [")
        for c in tables[m]:
            q = np.clip(np.round(c * SCALE), -32768, 32767).astype(int)
            lines.append("        [" + ", ".join(map(str, q)) + "],")
        lines.append("    ],")
    lines.append("];")
    dst.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
