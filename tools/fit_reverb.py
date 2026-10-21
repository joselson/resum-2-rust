"""Reverb impulse-response metrics from a probe run (tools/make_probes.mjs `fxrv6-`).

usage: fit_reverb.py RUN_DIR [--lumen] [--match REGEX]
       fit_reverb.py --rust OUT.rs RUN_DIR...   (Serum renders only)

Each probe is a 2 ms click whose amplitude envelope then sustains at about −131 dB
(Serum's reverb stops producing output when its input falls silent before the tail
emerges), through one reverb setting at 100 % wet; `fxrv6-dry` is the click alone.
Prints per probe: level (wet energy over the dry click's, dB), onset (ms from the
dry click to −30 dB of the wet peak), T60 broadband and in octave bands around 125,
500, 2k and 8k Hz (Schroeder integral, −5 to −25 dB fit), the tail's L/R correlation
and its side/mid energy ratio (dB), and the gain of a copy of the click followed by
silence (Vintage near its default VintageScale; a start-up transient, which the other
metrics leave out). --lumen reads Lumen's renders.

--rust writes crates/engine/src/fx/reverb_table.rs: for every type, the metrics of the
default setting and, per control, piecewise-linear deltas at the swept values (one
control moved at a time) and, from the `fxrv8-` grids of two controls, what each pair
measured beyond the sum of its single deltas. Freq is fitted as a one-pole low cut
from the band levels, plus the level change that cut does not explain.
"""
import json, re, sys
from pathlib import Path
import numpy as np
from scipy.signal import butter, sosfilt
from filter_response import read_wav

BANDS = [125, 500, 2000, 8000]


def onset(x, rel_db):
    m = np.abs(x).max(1)
    return int(np.argmax(m > m.max() * 10 ** (rel_db / 20)))


def t60(e, sr):
    """T60 from the Schroeder curve of energy e: fit −5 to −25 dB, extrapolate."""
    edc = np.cumsum(e[::-1])[::-1]
    edc_db = 10 * np.log10(edc / edc[0] + 1e-30)
    i0, i1 = np.argmax(edc_db < -5), np.argmax(edc_db < -25)
    if i1 <= i0 + 10:
        return float("nan")
    t = np.arange(i0, i1) / sr
    slope = np.polyfit(t, edc_db[i0:i1], 1)[0]
    return -60 / slope if slope < 0 else float("nan")


def split_direct(x, dry, sr, d0):
    """A scaled copy of the dry click, up to 12 samples late, followed by silence (Vintage
    near its default VintageScale, gone with any predelay): its signed gain, and the
    response without it. A start-up transient: on a sustained saw (`fxrv11-`) nothing
    follows the first milliseconds, so Lumen does not emulate it and the other metrics
    leave it out. Only Vintage probes pass `dry`: Hall's similar head is a diffused
    burst that predelay moves with the tail, so it stays part of the tail."""
    if dry is None:
        return 0.0, x
    n = d0 + int(0.008 * sr)
    head, gap = x[:n], x[n:n + int(0.005 * sr)]
    e_head = (head ** 2).sum()
    fits = []
    for lag in range(13):
        ref = np.roll(dry[:n], lag, 0)
        g = (head * ref).sum() / (ref ** 2).sum()
        fits.append((((head - g * ref) ** 2).sum(), g))
    residual, g = min(fits)
    if abs(g) < 1e-2 or residual > 0.05 * e_head or (gap ** 2).mean() > 1e-6 * e_head / n:
        return 0.0, x
    x = x.copy()
    x[:n] = 0
    return float(g), x


def metrics(x, sr, click, d0, dry):
    direct, x = split_direct(x, dry, sr, d0)
    e = (x ** 2).sum(1)
    level = 10 * np.log10(e.sum() / click + 1e-30)
    start = onset(x, -30)
    tail = x[start:]
    out = [level, start, t60((tail ** 2).sum(1), sr)]
    for f in BANDS:
        lo, hi = f / np.sqrt(2), min(f * np.sqrt(2), 0.45 * sr)
        sos = butter(4, [lo, hi], "band", fs=sr, output="sos")
        b = sosfilt(sos, tail, axis=0)
        out.append(t60((b ** 2).sum(1), sr))
    w = tail[int(0.05 * sr):int(0.05 * sr) + int(0.5 * sr)]
    mid, side = w[:, 0] + w[:, 1], w[:, 0] - w[:, 1]
    corr = np.corrcoef(w[:, 0], w[:, 1])[0, 1] if w.std() > 0 else float("nan")
    out += [corr, 10 * np.log10((side ** 2).sum() / max((mid ** 2).sum(), 1e-30) + 1e-30), direct]
    return out


def band_levels(x, sr):
    """Energy in each octave band (dB), summed over channels."""
    out = []
    for f in BANDS:
        sos = butter(4, [f / np.sqrt(2), min(f * np.sqrt(2), 0.45 * sr)], "band", fs=sr, output="sos")
        out.append(10 * np.log10((sosfilt(sos, x, axis=0) ** 2).sum() + 1e-30))
    return np.array(out)


# Control name in probe ids -> (preset key, default value).
CONTROLS = {"size": ("kParamSize", 35.0), "delay": ("kParamDelay", 30.625), "fb": ("kParamFeedback", 25.0),
            "freqb": ("kParamFreqB", 35.0), "freq": ("kParamFreq", 0.0), "width": ("kParamWidth", 20.0),
            "freqc": ("kParamFreqC", 70.0), "mode": ("kParamMode", 70.0), "vscale": ("kParamVintageScale", 70.0),
            "vscaleb": ("kParamVintageScaleB", 70.0), "predelay": ("kParamPreDelay", 0.0)}
TYPES = {"hall": "kHall", "vintage": "kVintage", "abyss": "kAbyss", "space": "kSpace", "untyped": "kUnnamed"}


def lowcut_hz(delta, sr=48000):
    """One-pole high-pass cutoff whose response best matches band-level changes."""
    best = (1e9, 5.0)
    for fc in np.geomspace(5, 5000, 400):
        resp = np.array([10 * np.log10(f * f / (f * f + fc * fc)) for f in BANDS])
        e = np.sum((resp - delta) ** 2)
        if e < best[0]:
            best = (e, fc)
    return best[1]


FIELDS = ["t60", "low", "upper", "high", "level", "onset", "corr"]
PREDELAY_SLOPE_MS = 919.0  # crates/engine/src/fx/reverb_model.rs


def delta(r, base):
    """A measurement as deltas from the type's default, in reverb_model::Point's terms."""
    ok = np.isfinite(r["t60"]) and r["level"] > -99
    d = {f: 0.0 for f in FIELDS}
    if ok:
        d["t60"] = np.log(r["t60"] / base["t60"])
        for f in ("low", "upper", "high"):
            d[f] = np.log(r[f] / base[f]) if np.isfinite(r[f]) else 0.0
        d["corr"] = r["corr"] - base["corr"] if np.isfinite(r["corr"]) else 0.0
    d["level"] = r["level"] - base["level"]
    d["onset"] = r["onset"] - base["onset"]
    return d


def interp1(points, ctl, x):
    """reverb_model::interpolate over [(at, deltas)], with the predelay extrapolation."""
    at = [v for v, _ in points]
    i = int(np.searchsorted(at, x, side="right"))
    if i == 0 or i == len(at):
        d = dict(points[max(i - 1, 0)][1])
        if ctl == "predelay" and i == len(at):
            d["onset"] += PREDELAY_SLOPE_MS * (x - at[-1])
        return d
    (a, da), (b, db) = points[i - 1], points[i]
    u = (x - a) / (b - a)
    return {f: da[f] + (db[f] - da[f]) * u for f in FIELDS}


def point(v, d, cut=0.0):
    return (f"Point {{ at: {v:.4f}, t60: {d['t60']:.4f}, low: {d['low']:.4f}, upper: {d['upper']:.4f}, high: {d['high']:.4f}, "
            f"level: {d['level']:.3f}, onset: {d['onset']:.2f}, corr: {d['corr']:.4f}, low_cut: {cut:.1f} }}")


def read_rows(runs):
    """(type, ((control, value), ...)) -> metrics; () is the default, one entry a sweep."""
    rows = {}
    for run in map(Path, runs):
        jobs = {re.sub(r"^\d+_\d+_", "", j["id"]): j["id"] for j in json.loads((run / "jobs.json").read_text())["jobs"]}
        dry, sr = read_wav(run / "reference" / f"{jobs['fxrv6_dry']}.wav")
        d0 = onset(dry, -30)
        click = (dry[:d0 + int(0.01 * sr)] ** 2).sum()
        dry_bands = band_levels(dry[:d0 + int(0.01 * sr)], sr)
        for name, jid in jobs.items():
            m = re.match(r"fxrv[68]_([a-z]+)_(.+)$", name)
            if not m or name == "fxrv6_dry":
                continue
            settings = () if m[2] == "default" else tuple((c, float(v.replace("_", "."))) for c, v in re.findall(r"([a-z]+)(\d+(?:_\d+)?)", m[2]))
            # A control set to its default is the same as omitted (fxrv8-vintage-vscale70 renders as the default).
            settings = tuple((c, v) for c, v in settings if v != CONTROLS[c][1])
            x, _ = read_wav(run / "reference" / f"{jid}.wav")
            v = metrics(x, sr, click, d0, dry if m[1] == "vintage" else None)
            mid = v[4]  # 500 Hz; low = 125 Hz, upper = 2 kHz, high = 8 kHz
            rows[(m[1], settings)] = {"level": max(v[0], -100.0), "onset": (v[1] - d0) / sr * 1000, "t60": mid, "low": v[3] / mid, "upper": v[5] / mid,
                                      "high": v[6] / mid, "corr": v[7], "direct": v[9], "bands": band_levels(split_direct(x, dry if m[1] == "vintage" else None, sr, d0)[1], sr) - dry_bands}
    return rows


def write_rust(out, runs):
    rows = read_rows(runs)
    lines = ["// @generated by tools/fit_reverb.py --rust from fxrv6-/fxrv7-/fxrv8- probe renders. Do not edit by hand.",
             "//! Serum reverb metrics per type: the default setting, per-control deltas and pair interactions.", "",
             "use super::reverb_model::{Pair, Point, Table, TypeTable};", ""]
    for t, key in TYPES.items():
        base = rows[(t, ())]
        tables, sweeps = [], {}
        for ctl, (pkey, dflt) in CONTROLS.items():
            pts = sorted([(s[0][1], r) for (tt, s), r in rows.items() if tt == t and len(s) == 1 and s[0][0] == ctl] + [(dflt, base)], key=lambda p: p[0])
            if len(pts) == 1:
                continue
            body, sweep = [], []
            for v, r in pts:
                d, cut = delta(r, base), 0.0
                if ctl == "freq" and v > 0 and np.isfinite(r["t60"]) and r["level"] > -99:
                    # The one-pole cut gives the spectral shape; the level keeps what it misses.
                    cut = lowcut_hz(r["bands"] - base["bands"])
                    e = 10 ** (base["bands"] / 10)
                    d["level"] -= 10 * np.log10(sum(e[i] * f * f / (f * f + cut * cut) for i, f in enumerate(BANDS)) / e.sum())
                body.append(point(v, d, cut))
                sweep.append((v, d))
            sweeps[ctl] = sweep
            tables.append(f'        Table {{ key: "{pkey}", points: &[\n            ' + ",\n            ".join(body) + ",\n        ] }")
        pairs = []
        for (ca, cb) in dict.fromkeys(tuple(c for c, _ in s) for (tt, s), _ in rows.items() if tt == t and len(s) == 2):
            grid = {(s[0][1], s[1][1]): r for (tt, s), r in rows.items() if tt == t and len(s) == 2 and (s[0][0], s[1][0]) == (ca, cb)}
            # The default's row and column carry no interaction.
            a_at = sorted({a for a, _ in grid} | {CONTROLS[ca][1]})
            b_at = sorted({b for _, b in grid} | {CONTROLS[cb][1]})
            if len(grid) != (len(a_at) - 1) * (len(b_at) - 1):
                continue
            cells = []
            for a in a_at:
                for b in b_at:
                    if a == CONTROLS[ca][1] or b == CONTROLS[cb][1]:
                        cells.append(point(0.0, {f: 0.0 for f in FIELDS}))
                        continue
                    d, da, db = delta(grid[(a, b)], base), interp1(sweeps[ca], ca, a), interp1(sweeps[cb], cb, b)
                    cells.append(point(0.0, {f: d[f] - da[f] - db[f] for f in FIELDS}))
            pairs.append(f'        Pair {{ a: "{CONTROLS[ca][0]}", b: "{CONTROLS[cb][0]}", a_at: &[{", ".join(f"{v:.4f}" for v in a_at)}], b_at: &[{", ".join(f"{v:.4f}" for v in b_at)}], cells: &[\n            '
                         + ",\n            ".join(cells) + ",\n        ] }")
        b = base
        lines.append(f"pub const {t.upper()}: TypeTable = TypeTable {{")
        lines.append(f'    kind: "{key}",')
        lines.append(f"    t60: {b['t60']:.4f}, low: {b['low']:.4f}, upper: {b['upper']:.4f}, high: {b['high']:.4f}, level: {b['level']:.3f}, onset: {b['onset']:.2f}, corr: {b['corr']:.4f},")
        lines.append("    controls: &[\n" + ",\n".join(tables) + ",\n    ],")
        lines.append("    pairs: &[" + ("\n" + ",\n".join(pairs) + ",\n    " if pairs else "") + "],")
        lines.append("};")
        lines.append("")
    lines.append("pub const TYPES: [&TypeTable; 5] = [&HALL, &VINTAGE, &ABYSS, &SPACE, &UNTYPED];")
    # Summed deltas can leave what Serum ever measured; the model clamps to this.
    ratios = np.array([[r[f] for f in ("low", "upper", "high")] for r in rows.values() if r["level"] > -60 and np.isfinite(r["t60"])])
    ratios = ratios[np.isfinite(ratios).all(1)]
    lo, hi = np.percentile(ratios, 1, axis=0), np.percentile(ratios, 99, axis=0)
    lines.append("")
    lines.append("/// The 125 Hz, 2 kHz and 8 kHz decay ratios measured (1st to 99th percentile).")
    lines.append("pub const RATIO_RANGE: [(f32, f32); 3] = [" + ", ".join(f"({a:.3f}, {b:.3f})" for a, b in zip(lo, hi)) + "];")
    Path(out).write_text("\n".join(lines) + "\n")


def main():
    args = sys.argv[1:]
    if args[0] == "--rust":
        return write_rust(args[1], args[2:])
    run = Path(args[0])
    sub = "lumen" if "--lumen" in args else "reference"
    pat = re.compile(args[args.index("--match") + 1]) if "--match" in args else None
    jobs = {re.sub(r"^\d+_\d+_", "", j["id"]): j["id"] for j in json.loads((run / "jobs.json").read_text())["jobs"]}
    dry, sr = read_wav(run / sub / f"{jobs['fxrv6_dry']}.wav")
    d0 = onset(dry, -30)
    click = (dry[:d0 + int(0.01 * sr)] ** 2).sum()
    print(f"{'probe':28} {'level':>6} {'onset':>6} {'T60':>6} " + " ".join(f"{f:>6}" for f in BANDS) + f" {'corr':>5} {'S/M':>6} {'direct':>6}")
    for name, jid in jobs.items():
        if not name.startswith(("fxrv6_", "fxrv8_", "fxrvf_")) or name == "fxrv6_dry" or (pat and not pat.search(name)):
            continue
        x, _ = read_wav(run / sub / f"{jid}.wav")
        m = metrics(x, sr, click, d0, dry if "vintage" in name else None)
        m[1] = (m[1] - d0) / sr * 1000
        print(f"{name[6:]:28} {m[0]:6.1f} {m[1]:6.1f} " + " ".join(f"{v:6.2f}" for v in m[2:7]) + f" {m[7]:5.2f} {m[8]:6.1f} {m[9]:6.2f}")


if __name__ == "__main__":
    main()
