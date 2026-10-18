"""Dump Serum 2's host parameters: default and display text across the normalized range."""
import json, time
from pedalboard import load_plugin
SERUM = r"C:\Program Files\Common Files\VST3\Serum2.vst3\Contents\x86_64-win\Serum2.vst3"
serum = load_plugin(SERUM, plugin_name="Serum 2")
first = list(serum.parameters.items())[0][1]
print("parameter attributes:", [a for a in dir(first) if not a.startswith("_")])
steps = [i / 20 for i in range(21)]
out = []
t = time.time()
for name, p in serum.parameters.items():
    default = p.raw_value
    entry = {"name": name, "default_raw": default, "default_text": p.string_value, "label": getattr(p, "label", ""),
             "discrete": getattr(p, "is_discrete", None), "steps": getattr(p, "num_steps", None), "sweep": []}
    for s in steps:
        p.raw_value = s
        entry["sweep"].append([s, p.string_value])
    p.raw_value = default
    out.append(entry)
json.dump(out, open("serum_params.json", "w"), indent=0)
print(f"{len(out)} parameters in {time.time() - t:.0f}s")
