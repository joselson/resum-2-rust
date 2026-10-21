"""Compressor static curves from a probe run (tools/make_probes.mjs `fxc-` and `fxmb-`
sets).

usage: fit_comp.py RUN_DIR [--lumen] [--from S --to S]

Each probe plays the sub (sine or square) through 1–2 FX Utility stages that set the
input level, then one compressor setting (`fxmb-`/`fxmbsq-`: multiband settings on a
sine/square, rendered at one note per run); `dry` is the same chain without the compressor. Prints, per
setting, the input peak in dB (inside the rack: the default master volume, applied
after the effects, is divided out) and the steady-state gain (wet rms over dry rms)
in dB for every input level. --lumen reads Lumen's renders.
"""
import json, re, sys
from collections import defaultdict
from pathlib import Path
import numpy as np
from filter_response import read_wav

MASTER = 0.25 * np.sqrt(2)


def window(path, t0, t1):
    x, r = read_wav(path)
    return x[int(t0 * r):int(t1 * r), 0] / MASTER


def db(v):
    return 20 * np.log10(max(v, 1e-9))


def main():
    args = sys.argv[1:]
    run = Path(args[0])
    sub = "lumen" if "--lumen" in args else "reference"
    t0 = float(args[args.index("--from") + 1]) if "--from" in args else 0.35
    t1 = float(args[args.index("--to") + 1]) if "--to" in args else 0.55
    probes = defaultdict(dict)  # (shape, level) -> setting -> job id
    for j in json.loads((run / "jobs.json").read_text())["jobs"]:
        m = re.search(r"_(fxc_sine|fxc_square|fxmb|fxmbsq)_(u\d+(?:_\d+)?(?:x2)?)_(.+)$", j["id"])
        if m:
            probes[(m[1], m[2])][m[3]] = j["id"]
    rows = defaultdict(list)  # (shape, setting) -> [(input dB, gain dB)]
    for (shape, level), settings in sorted(probes.items()):
        dry = window(run / sub / f"{settings['dry']}.wav", t0, t1)
        peak, rms = np.abs(dry).max(), np.sqrt(np.mean(dry ** 2))
        for setting, jid in settings.items():
            if setting == "dry":
                continue
            wet = window(run / sub / f"{jid}.wav", t0, t1)
            rows[(shape, setting)].append((db(peak), db(np.sqrt(np.mean(wet ** 2))) - db(rms)))
    for (shape, setting), pts in sorted(rows.items()):
        pts.sort()
        print(f"{shape:10} {setting:12}", " ".join(f"{i:6.1f}:{g:+6.2f}" for i, g in pts))


if __name__ == "__main__":
    main()
