"""Reverb impulse-response metrics from a probe run (tools/make_probes.mjs `fxrv6-`).

usage: fit_reverb.py RUN_DIR [--lumen] [--match REGEX]
       fit_reverb.py --rust OUT.rs RUN_DIR...   (Serum renders only)

Each probe is a 2 ms click whose amplitude envelope then sustains at about −131 dB
(Serum's reverb stops producing output when its input falls silent before the tail
emerges), through one reverb setting at 100 % wet; `fxrv6-dry` is the click alone.
Prints per probe: level (wet energy over the dry click's, dB), onset (ms from the
dry click to −30 dB of the wet peak), T60 broadband and in octave bands around 125,
500, 2k and 8k Hz (Schroeder integral, −5 to −25 dB fit), the tail's L/R correlation
and its side/mid energy ratio (dB). --lumen reads Lumen's renders.

--rust writes crates/engine/src/fx/reverb_table.rs: for every type, the metrics of the
default setting and, per control, piecewise-linear deltas at the swept values (one
control moved at a time; Lumen combines them as independent). Freq is fitted as a
one-pole low cut from the band levels, plus the level change that cut does not explain.
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


def metrics(x, sr, click):
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
    out += [corr, 10 * np.log10((side ** 2).sum() / max((mid ** 2).sum(), 1e-30) + 1e-30)]
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


def write_rust(out, runs):
    rows = {}  # (type, control, value) -> metrics
    for run in map(Path, runs):
        jobs = {re.sub(r"^\d+_\d+_", "", j["id"]): j["id"] for j in json.loads((run / "jobs.json").read_text())["jobs"]}
        dry, sr = read_wav(run / "reference" / f"{jobs['fxrv6_dry']}.wav")
        d0 = onset(dry, -30)
        click = (dry[:d0 + int(0.01 * sr)] ** 2).sum()
        dry_bands = band_levels(dry[:d0 + int(0.01 * sr)], sr)
        for name, jid in jobs.items():
            m = re.match(r"fxrv6_([a-z]+)_([a-z]+?)(\d+(?:_\d+)?)$", name) or re.match(r"fxrv6_([a-z]+)_(default)()$", name)
            if not m:
                continue
            x, _ = read_wav(run / "reference" / f"{jid}.wav")
            v = metrics(x, sr, click)
            mid = v[4]  # 500 Hz; low = 125 Hz, upper = 2 kHz, high = 8 kHz
            rec = {"level": max(v[0], -100.0), "onset": (v[1] - d0) / sr * 1000, "t60": mid, "low": v[3] / mid, "upper": v[5] / mid, "high": v[6] / mid,
                   "corr": v[7], "bands": band_levels(x, sr) - dry_bands}
            value = None if m[2] == "default" else float(m[3].replace("_", "."))
            rows[(m[1], m[2], value)] = rec
    lines = ["// @generated by tools/fit_reverb.py --rust from fxrv6-/fxrv7- probe renders. Do not edit by hand.",
             "//! Serum reverb metrics per type: the default setting and per-control deltas.", "",
             "use super::reverb_model::{Point, Table, TypeTable};", ""]
    for t, key in TYPES.items():
        base = rows[(t, "default", None)]
        tables = []
        for ctl, (pkey, dflt) in CONTROLS.items():
            pts = sorted((v, r) for (tt, c, v), r in rows.items() if tt == t and c == ctl)
            if not pts:
                continue
            pts = sorted(pts + [(dflt, base)], key=lambda p: p[0])
            body = []
            for v, r in pts:
                ok = np.isfinite(r["t60"]) and r["level"] > -99
                t60 = np.log(r["t60"] / base["t60"]) if ok else 0.0
                low = np.log(r["low"] / base["low"]) if ok and np.isfinite(r["low"]) else 0.0
                high = np.log(r["high"] / base["high"]) if ok and np.isfinite(r["high"]) else 0.0
                upper = np.log(r["upper"] / base["upper"]) if ok and np.isfinite(r["upper"]) else 0.0
                corr = (r["corr"] - base["corr"]) if ok and np.isfinite(r["corr"]) else 0.0
                level = r["level"] - base["level"]
                cut = 0.0
                if ctl == "freq" and v > 0 and ok:
                    # The one-pole cut gives the spectral shape; the level keeps what it misses.
                    cut = lowcut_hz(r["bands"] - base["bands"])
                    e = 10 ** (base["bands"] / 10)
                    level -= 10 * np.log10(sum(e[i] * f * f / (f * f + cut * cut) for i, f in enumerate(BANDS)) / e.sum())
                body.append(f"Point {{ at: {v:.4f}, t60: {t60:.4f}, low: {low:.4f}, upper: {upper:.4f}, high: {high:.4f}, level: {level:.3f}, onset: {r['onset'] - base['onset']:.2f}, corr: {corr:.4f}, low_cut: {cut:.1f} }}")
            tables.append(f'        Table {{ key: "{pkey}", points: &[\n            ' + ",\n            ".join(body) + ",\n        ] }")
        b = base
        lines.append(f"pub const {t.upper()}: TypeTable = TypeTable {{")
        lines.append(f'    kind: "{key}",')
        lines.append(f"    t60: {b['t60']:.4f}, low: {b['low']:.4f}, upper: {b['upper']:.4f}, high: {b['high']:.4f}, level: {b['level']:.3f}, onset: {b['onset']:.2f}, corr: {b['corr']:.4f},")
        lines.append("    controls: &[\n" + ",\n".join(tables) + ",\n    ],")
        lines.append("};")
        lines.append("")
    lines.append("pub const TYPES: [&TypeTable; 5] = [&HALL, &VINTAGE, &ABYSS, &SPACE, &UNTYPED];")
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
    print(f"{'probe':28} {'level':>6} {'onset':>6} {'T60':>6} " + " ".join(f"{f:>6}" for f in BANDS) + f" {'corr':>5} {'S/M':>6}")
    for name, jid in jobs.items():
        if not name.startswith(("fxrv6_", "fxrvf_")) or name == "fxrv6_dry" or (pat and not pat.search(name)):
            continue
        x, _ = read_wav(run / sub / f"{jid}.wav")
        m = metrics(x, sr, click)
        m[1] = (m[1] - d0) / sr * 1000
        print(f"{name[6:]:28} {m[0]:6.1f} {m[1]:6.1f} " + " ".join(f"{v:6.2f}" for v in m[2:7]) + f" {m[7]:5.2f} {m[8]:6.1f}")


if __name__ == "__main__":
    main()
