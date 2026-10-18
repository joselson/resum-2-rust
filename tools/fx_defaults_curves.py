"""Third pass: defaults that are curve values at round normalized positions
(linear, exponential, power k=2..5 at n = i/100 between the clamp bounds). Records the curve
that produced the default, which is also the parameter's normalized mapping."""
import json, sys, time
import numpy as np
sys.argv = sys.argv[:1]
import fx_defaults as fd
import param_map as pm

def curves(lo, hi):
    out = {}
    for i in range(101):
        n = i / 100
        forms = {"linear": lo + (hi - lo) * n}
        if lo > 0:
            forms["exp"] = lo * (hi / lo) ** n
        for k in (2, 3, 4, 5):
            forms[f"power{k}"] = lo + (hi - lo) * n ** k
        for name, v in forms.items():
            out.setdefault(float(np.float32(v)), (name, n))
    return out

def main():
    res = json.load(open("serum_fx_params.json"))
    todo = {}
    for n, m in res.items():
        for k, r in m["params"].items():
            if r["default"] is None and r["min"] is not None and r["max"] is not None and r["max"] > r["min"]:
                todo.setdefault(n, {})[k] = curves(r["min"], r["max"])
    serum = pm.load_plugin(pm.SERUM, plugin_name="Serum 2")
    base = serum.raw_state
    t = time.time()
    for group in ([n for n in todo if not n.startswith("FXSplit")], [n for n in todo if n.startswith("FXSplit")]):
        if not group:
            continue
        lists = {n: {k: list(c.items()) for k, c in todo[n].items()} for n in group}
        rounds = max(len(c) for n in group for c in lists[n].values())
        for i in range(rounds):
            mods, sent = [], {}
            for n in group:
                sent[n] = {k: c[i][0] for k, c in lists[n].items() if i < len(c) and res[n]["params"][k]["default"] is None}
                mods.append((n, res[n]["type"], sent[n]))
            if not any(sent.values()):
                break
            serum.raw_state = fd.state_with(base, {"FXRack0": fd.rack(mods)})
            snap = pm.snapshot(serum)
            for j, n in enumerate(group):
                prefix = f"FXRack0.FX[{j}].{n}.plainParams."
                for k, v in sent[n].items():
                    if prefix + k not in snap:
                        curve, pos = todo[n][k][v]
                        res[n]["params"][k].update(default=v, default_curve=curve, default_norm=pos)
        print(f"group done in {time.time() - t:.0f}s", flush=True)
    json.dump(res, open("serum_fx_params.json", "w"), indent=1)
    for n, m in res.items():
        for k, r in m["params"].items():
            if "default_curve" in r:
                print(f"  {n}.{k}: {r['default']} = {r['default_curve']} at n={r['default_norm']}")
    print("still missing:", [(n, k) for n, m in res.items() for k, r in m["params"].items() if r["default"] is None])

if __name__ == "__main__":
    main()
