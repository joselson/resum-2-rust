"""Fit Serum 2 host-parameter samples (serum_param_curves.json) into parameter specs.

Each preset field gets: Serum host name, default plain value, and a mapping from
normalized [0, 1] to the stored plain value: toggle, choice list, linear,
power (min + (max-min) * n^k), exponential (min * (max/min)^n) or a lookup table.
Output: data/serum2-params.json (sorted by field) for the Rust table generator.
"""
import json, math, sys

def lstsq_line(pts):
    n = len(pts); sx = sum(x for x, _ in pts); sy = sum(y for _, y in pts)
    sxx = sum(x * x for x, _ in pts); sxy = sum(x * y for x, y in pts)
    den = n * sxx - sx * sx
    if abs(den) < 1e-12:
        return sy / n, 0.0
    b = (n * sxy - sx * sy) / den
    return (sy - b * sx) / n, b


def fit(samples):
    """Least-squares fit tolerant of missing points: Serum omits a field whenever it
    equals the default, so the sample at the default position is always absent."""
    ys = [y for _, y in samples]
    if any(isinstance(y, str) for y in ys):
        names = []
        for y in ys:
            if isinstance(y, str) and y not in names:
                names.append(y)
        return {"kind": "choice", "options": names}
    pts = [(x, float(y)) for x, y in samples if isinstance(y, (int, float)) and not isinstance(y, bool)]
    if len(pts) < 2:
        return {"kind": "unknown"}
    distinct = sorted(set(round(y, 9) for _, y in pts))
    if set(distinct) <= {0.0, 1.0}:
        return {"kind": "toggle"}
    cands = []
    a, b = lstsq_line(pts)
    cands.append(("linear", lambda x, a=a, b=b: a + b * x, {}))
    for k10 in range(11, 101):
        k = k10 / 10
        a2, b2 = lstsq_line([(x ** k, y) for x, y in pts])
        cands.append(("power", lambda x, a=a2, b=b2, k=k: a + b * x ** k, {"k": k}))
    if all(y > 0 for _, y in pts):
        c, d = lstsq_line([(x, math.log(y)) for x, y in pts])
        cands.append(("exp", lambda x, c=c, d=d: math.exp(c + d * x), {}))
    def err(f):
        lo, hi = f(0.0), f(1.0)
        span = max(abs(hi - lo), 1e-9)
        return max(abs(f(x) - y) for x, y in pts) / span
    e, name, f, extra = min(((err(f), name, f, extra) for name, f, extra in cands), key=lambda t: t[0])
    integer = all(abs(y - round(y)) < 1e-6 for _, y in pts) and len(distinct) < len(pts)
    if e < 0.003:
        return {"kind": name, "min": round(f(0.0), 9), "max": round(f(1.0), 9), **extra, **({"integer": True} if integer else {}), "_f": f}
    if integer:
        return {"kind": "steps", "points": pts, "min": min(distinct), "max": max(distinct)}
    return {"kind": "table", "points": pts, "fit_error": round(e, 4)}


def main(src, dst):
    rows = json.load(open(src))
    out = []
    for r in rows:
        if r["name"].endswith("_name") or "." not in r["field"]:
            continue
        spec = fit([(x, plain) for x, plain, _text in r["samples"]])
        f = spec.pop("_f", None)
        d = r["default_plain"]
        if d is None and f is not None:
            d = round(f(r["default_raw"]), 9)
        if d is None and spec["kind"] == "toggle":
            d = 1.0 if r["default_raw"] >= 0.5 else 0.0
        out.append({"field": r["field"], "host": r["name"], "default": d, "default_raw": r["default_raw"], "default_text": r["default_text"].strip(), "units": r["units"], **spec})
    out.sort(key=lambda e: e["field"])
    json.dump(out, open(dst, "w"), indent=1)
    kinds = {}
    for e in out:
        kinds[e["kind"]] = kinds.get(e["kind"], 0) + 1
    print(len(out), "fields:", kinds)

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
