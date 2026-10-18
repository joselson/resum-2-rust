"""Loads the Lumen VST3 in pedalboard (a real VST3 host) and checks that a preset
renders, that host parameter changes reach the engine, and that state round-trips.

usage: host_check.py PATH\\TO\\Lumen.vst3\\Contents\\x86_64-win\\Lumen.vst3 PRESET
"""
import os, sys
import numpy as np
from pedalboard import load_plugin

dll, preset = sys.argv[1], sys.argv[2]
os.environ["LUMEN_PRESET"] = preset
rate = 48000
lumen = load_plugin(dll)
names = list(lumen.parameters.keys())
print(f"loaded {lumen.name}: {len(names)} parameters")


def render():
    notes = [(bytes([0x90, 48, 100]), 0.0), (bytes([0x80, 48, 64]), 1.0)]
    return lumen(notes, duration=1.5, sample_rate=rate, num_channels=2, reset=True)


def rms(x):
    return float(np.sqrt(np.mean(np.square(x[:, : rate]))))


base = render()
print(f"preset render: rms {rms(base):.4f}, finite {bool(np.isfinite(base).all())}")
vol = next(n for n in names if "oscillator_1" in n and n.endswith("volume"))
before = lumen.parameters[vol].raw_value
lumen.parameters[vol].raw_value = 0.0
quiet = render()
print(f"{vol}: {before:.4f} -> 0: rms {rms(quiet):.4f}")
lumen.parameters[vol].raw_value = before
restored = render()  # pedalboard hands parameter changes to the plugin while processing
print(f"{vol} back to {before:.4f}: rms {rms(restored):.4f}")
state = lumen.raw_state
lumen.parameters[vol].raw_value = 0.0
render()
lumen.raw_state = state
again = render()
print(f"state {len(state)} bytes restored over a silenced oscillator: rms {rms(again):.4f}")
