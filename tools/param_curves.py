"""For each host parameter that writes one preset field: plain value at its default and across
the normalized range (every step for discrete parameters). Writes serum_param_curves.json."""
import json, sys, time
sys.argv = sys.argv[:1]
import param_map as pm  # reuses loader/snapshot; its own sweep runs only under __main__

def plain_at(serum, p, field, x):
    p.raw_value = x
    snap = pm.snapshot(serum)
    return snap.get(field), p.string_value

def main():
    mapping = {e["name"]: e["fields"] for e in json.load(open("serum_param_map.json")) if len(e["fields"]) >= 1}
    serum = pm.load_plugin(pm.SERUM, plugin_name="Serum 2")
    out, t = [], time.time()
    for name, p in serum.parameters.items():
        fields = mapping.get(name)
        if not fields:
            continue
        field = fields[0]
        default = p.raw_value
        steps = p.num_steps if hasattr(p, "num_steps") else None
        approx = getattr(p, "approximate_step_size", None)
        if approx and approx >= 1 / 256:
            xs = [min(1.0, i * approx) for i in range(int(round(1 / approx)) + 1)]
        else:
            xs = [i / 20 for i in range(21)]
        default_plain, default_text = plain_at(serum, p, field, default)
        samples = [[x, *plain_at(serum, p, field, x)] for x in xs]
        p.raw_value = default
        pm.snapshot(serum)
        out.append({"name": name, "field": field, "default_raw": default, "default_plain": default_plain,
                    "default_text": default_text, "discrete_step": approx if approx and approx >= 1 / 256 else None,
                    "units": getattr(p, "units", ""), "samples": samples})
    json.dump(out, open("serum_param_curves.json", "w"))
    print(f"{len(out)} params in {time.time() - t:.0f}s")

if __name__ == "__main__":
    main()
