"""Sweeps one host parameter's display text over [0, 1] after loading each given
preset (to set hidden flags such as beat sync), printing where each label starts.

usage: display_sweep.py PARAM STEPS PRESET...
"""
import sys
from serum_render import load_plugin, SERUM, preset_state

def main():
    name, steps, presets = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
    serum = load_plugin(SERUM, plugin_name="Serum 2")
    base = serum.raw_state
    for path in presets:
        serum.raw_state = preset_state(base, open(path, "rb").read())
        p = serum.parameters[name]
        rows, last = [], None
        for i in range(steps + 1):
            p.raw_value = i / steps
            text = p.string_value.strip()
            if text != last:
                rows.append((round(i / steps, 5), text))
                last = text
        print(path, len(rows), rows, flush=True)

if __name__ == "__main__":
    main()
