"""Filter frequency responses from a probe run (tools/make_probes.mjs `filt-` set).

usage: filter_response.py RUN_DIR [--plot OUT.png NAME_REGEX] [--top N]

Each probe plays one low note (C1) through the filter; dividing its harmonic
magnitudes by the `filt-off` render gives |H(f)| at every harmonic, for Serum
(reference/) and Lumen (lumen/) separately, so oscillator differences cancel.
Writes RUN_DIR/filter-response.json and prints probes ranked by response error
(RMS dB over 40 Hz–16 kHz, harmonics within 60 dB of the passband peak).
"""
import json, re, sys
from pathlib import Path
import numpy as np


def read_wav(path):
    b = Path(path).read_bytes()
    pos, fmt, data = 12, None, None
    while pos + 8 <= len(b):
        cid, n = b[pos:pos + 4], int.from_bytes(b[pos + 4:pos + 8], "little")
        body = b[pos + 8:pos + 8 + n]
        if cid == b"fmt ":
            fmt = [int.from_bytes(body[a:c], "little") for a, c in ((0, 2), (2, 4), (4, 8), (14, 16))]
        elif cid == b"data":
            data = body
        pos += 8 + n + (n & 1)
    tag, ch, rate, bits = fmt
    if tag == 0xFFFE:
        tag = 3 if bits in (32, 64) else 1
    dtype = {(3, 32): np.float32, (3, 64): np.float64, (1, 16): np.int16, (1, 32): np.int32}[(tag, bits)]
    x = np.frombuffer(data, dtype=dtype).astype(np.float64)
    if tag == 1:
        x /= 2 ** (bits - 1)
    return x.reshape(-1, ch), rate


N = 32768
F0 = 440 * 2 ** ((24 - 69) / 12)


def harmonics(path):
    """Harmonic frequencies and per-channel magnitudes of the steady part of a render."""
    x, rate = read_wav(path)
    seg = x[int(0.45 * rate):int(0.45 * rate) + N]
    if len(seg) < N:
        return None
    spec = np.abs(np.fft.rfft(seg * np.hanning(N)[:, None], axis=0))
    bin_hz = rate / N
    ks = np.arange(1, int(min(16000, rate / 2 - 100) / F0))
    freqs, mags = [], []
    for k in ks:
        c = int(round(k * F0 / bin_hz))
        lo, hi = max(c - 3, 0), c + 4
        i = lo + int(np.argmax(spec[lo:hi].sum(axis=1)))
        freqs.append(i * bin_hz)
        mags.append(spec[i])
    return np.array(freqs), np.array(mags)


def response(run, side, pid, off):
    h = harmonics(run / side / f"{pid}.wav")
    if h is None or off.get(side) is None:
        return None
    f, m = h
    _, m0 = off[side]
    return f, 20 * np.log10((m + 1e-9) / (m0 + 1e-9))


def main():
    run = Path(sys.argv[1])
    args = sys.argv[2:]
    jobs = json.loads((run / "jobs.json").read_text())["jobs"]
    ids = [j["id"] for j in jobs]
    off_id = next(i for i in ids if i.endswith("filt_off"))
    off = {side: harmonics(run / side / f"{off_id}.wav") for side in ("reference", "lumen")}
    results = {}
    for pid in ids:
        if not re.search(r"_filt\d?_", pid) or pid == off_id:
            continue
        name = re.split(r"_filt\d?_", pid, maxsplit=1)[1]
        s, l = response(run, "reference", pid, off), response(run, "lumen", pid, off)
        if s is None or l is None:
            continue
        f, hs = s
        hl = l[1]
        # Mean over channels for ranking; keep both channels in the JSON.
        ms, ml = hs.mean(axis=1), hl.mean(axis=1)
        band = (f > 40) & (f < 16000) & (ms > ms.max() - 60)
        err = float(np.sqrt(np.mean((ms[band] - ml[band]) ** 2))) if band.any() else float("nan")
        results[name] = {"freqs": f.round(2).tolist(), "serum": hs.round(2).tolist(), "lumen": hl.round(2).tolist(), "rms_db": round(err, 2)}
    (run / "filter-response.json").write_text(json.dumps(results))

    if "--plot" in args:
        i = args.index("--plot")
        out, pattern = args[i + 1], re.compile(args[i + 2])
        plot(results, [n for n in results if pattern.search(n)], out)
        return
    top = int(args[args.index("--top") + 1]) if "--top" in args else len(results)
    ranked = sorted(results.items(), key=lambda kv: -np.nan_to_num(kv[1]["rms_db"], nan=99))
    print(f"{len(results)} probes, mean response error {np.nanmean([r['rms_db'] for r in results.values()]):.2f} dB")
    for name, r in ranked[:top]:
        print(f"{r['rms_db']:7.2f}  {name}")


def plot(results, names, out):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    cols = 4
    rows = max(1, (len(names) + cols - 1) // cols)
    fig, axes = plt.subplots(rows, cols, figsize=(cols * 4, rows * 2.6), squeeze=False)
    for ax, name in zip(axes.flat, names):
        r = results[name]
        f = np.array(r["freqs"])
        ax.semilogx(f, np.array(r["serum"]).mean(axis=1), "k", lw=1.2, label="Serum")
        ax.semilogx(f, np.array(r["lumen"]).mean(axis=1), "r", lw=0.9, label="Lumen")
        ax.set_title(f"{name}  ({r['rms_db']:.1f} dB)", fontsize=8)
        ax.set_ylim(-60, 25)
        ax.set_xlim(30, 16000)
        ax.grid(True, which="both", alpha=0.3)
        ax.tick_params(labelsize=6)
    for ax in axes.flat[len(names):]:
        ax.axis("off")
    axes.flat[0].legend(fontsize=6)
    fig.tight_layout()
    fig.savefig(out, dpi=80)


if __name__ == "__main__":
    main()
