"""Enumerate every option of Serum 2's choice parameters in order (1/1024 sweep).
Writes serum_param_choices.json: field -> [[first_x, stored_name or null, display]]."""
import json, sys, time
sys.argv = sys.argv[:1]
import param_map as pm

def main():
    fields = {r["host"]: r["field"] for r in json.load(open("choice_fields.json"))}
    serum = pm.load_plugin(pm.SERUM, plugin_name="Serum 2")
    out, t = {}, time.time()
    for name, p in serum.parameters.items():
        field = fields.get(name)
        if not field:
            continue
        default = p.raw_value
        options, last = [], object()
        for i in range(1025):
            x = i / 1024
            p.raw_value = x
            stored = pm.snapshot(serum).get(field)
            display = p.string_value.strip()
            key = (stored, display)
            if key != last:
                options.append([x, stored, display])
                last = key
        p.raw_value = default
        pm.snapshot(serum)
        out[field] = {"default_raw": default, "options": options}
        print(f"{field}: {len(options)} options", flush=True)
    json.dump(out, open("serum_param_choices.json", "w"), indent=0)
    print(f"done in {time.time() - t:.0f}s")

if __name__ == "__main__":
    main()
