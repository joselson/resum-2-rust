"""Second pass for FX parameters whose default fx_defaults.py did not hit: dense float32-exact
candidates (2-significant-digit decimals, dyadic fractions, clamp bounds). Updates serum_fx_params.json."""
import json, sys, time
import numpy as np
sys.argv = sys.argv[:1]
import fx_defaults as fd
import param_map as pm

def candidates(lo, hi):
    vals = set()
    for e in range(-4, 5):
        for k in range(10, 100):
            for s in (1, -1):
                vals.add(s * k * 10.0 ** (e - 1))
    for den in (2, 4, 8, 16, 32, 64, 3, 6, 12, 24):
        for num in range(1, 4 * den + 1):
            vals.add(num / den)
    if lo is not None: vals.add(lo)
    if hi is not None: vals.add(hi)
    out = sorted(float(np.float32(v)) for v in vals if (lo is None or v >= lo - 1e-9) and (hi is None or v <= hi + 1e-9))
    return out

def main():
    res = json.load(open("serum_fx_params.json"))
    cand = json.load(open("fx-candidates.json"))
    todo = {n: {k: candidates(r["min"], r["max"]) for k, r in m["params"].items() if r["default"] is None and not k.startswith("kParamModuleCount") and not cand[n]["params"][k]["texts"]} for n, m in res.items()}
    todo = {n: v for n, v in todo.items() if v}
    print({n: {k: len(c) for k, c in v.items()} for n, v in todo.items()}, flush=True)
    serum = pm.load_plugin(pm.SERUM, plugin_name="Serum 2")
    base = serum.raw_state
    groups = [{"FXRack0": [n for n in todo if not n.startswith("FXSplit")]}, {"FXRack0": [n for n in todo if n.startswith("FXSplit")]}]
    t = time.time()
    for group in groups:
        names = group["FXRack0"]
        if not names:
            continue
        rounds = max(len(c) for n in names for c in todo[n].values())
        for i in range(rounds):
            mods, sent = [], {}
            for n in names:
                params = {k: c[i] for k, c in todo[n].items() if i < len(c) and res[n]["params"][k]["default"] is None}
                sent[n] = params
                mods.append((n, res[n]["type"], params))
            if not any(sent.values()):
                break
            serum.raw_state = fd.state_with(base, {"FXRack0": fd.rack(mods)})
            snap = pm.snapshot(serum)
            for j, n in enumerate(names):
                prefix = f"FXRack0.FX[{j}].{n}.plainParams."
                for k, v in sent[n].items():
                    if prefix + k not in snap:
                        res[n]["params"][k]["default"] = v
        print(f"group {names} done in {time.time() - t:.0f}s", flush=True)
    json.dump(res, open("serum_fx_params.json", "w"), indent=1)
    missing = [(n, k) for n, m in res.items() for k, r in m["params"].items() if r["default"] is None]
    print("still missing:", missing)

if __name__ == "__main__":
    main()
