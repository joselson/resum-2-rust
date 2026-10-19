"""Sweeps LFO 1's rate display with beat sync on, for each given preset (e.g. the
lfo-sync-dotted / -triplet / -x10 probes, which set those flags), and prints where
each division label starts. Plain rate = 100 * normalized^4 (parameter oracle).

usage: lfo_divisions.py PRESET...
"""
import json, sys
from serum_render import load_plugin, SERUM, preset_state

def main():
    serum = load_plugin(SERUM, plugin_name="Serum 2")
    base = serum.raw_state
    out = {}
    for path in sys.argv[1:]:
        serum.raw_state = preset_state(base, open(path, "rb").read())
        p = serum.parameters["lfo_1_rate"]
        rows, last = [], None
        for i in range(1001):
            p.raw_value = i / 1000
            text = p.string_value.strip()
            if text != last:
                rows.append([i / 1000, text])
                last = text
        out[path] = rows
        print(path, rows, flush=True)
    json.dump(out, open("lfo_divisions.json", "w"), indent=1)

if __name__ == "__main__":
    main()
