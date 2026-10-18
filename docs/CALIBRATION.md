# Calibration against Serum 2

Lumen is matched to Serum 2 by measurement, never by reading its code. Two
oracles run the user's installed plugin headlessly on Windows through
[pedalboard](https://github.com/spotify/pedalboard) (scripts in `tools/`, run
with the Windows Python venv in `C:\Users\<you>\lumen-ref\venv`):

1. **Parameter oracle**: sets every host parameter, lets the processor run a
   block, and reads the plugin state back to see which preset field changed and
   to what. This yields each field's host name, value curve, default and option
   list.
2. **Render oracle**: loads a preset into Serum by repackaging its processor
   sections as VST3 state, plays notes and writes WAVs.

## Parameter tables

```bash
# Windows side (from lumen-ref): map, sample curves, enumerate options
venv\Scripts\python.exe param_map.py        # serum_param_map.json
venv\Scripts\python.exe param_curves.py     # serum_param_curves.json
venv\Scripts\python.exe param_choices.py    # serum_param_choices.json
# Repo side
python3 tools/fit_params.py serum_param_curves.json data/serum2-params.json
node tools/preset_keys.mjs "<Serum 2 Presets>" > data/serum2-preset-keys.json
python3 tools/gen_params.py                 # crates/engine/src/params/serum2.rs
```

`data/serum2-param-order.json` holds each module's parameter enum order from the
binary's name strings, so table indices equal the `destModuleParamID` mod slots
store. Each parameter records its evidence (measured, presets, name table,
assumed).

## Render comparisons

```bash
# Probe presets: Serum's Init state with one change each
node tools/make_probes.mjs serum_default_state.bin.IComponent <lumen-ref>/probes
lumen-compare jobs --presets <lumen-ref>/probes --dir <run> --count 41 --notes 60
venv\Scripts\python.exe serum_render.py runs\<run>\jobs.json     # Windows
lumen-compare report --dir <run> --tables "<Serum 2 Presets>/Tables"
```

The report ranks presets by a score built from loudness, level-matched envelope,
third-octave spectral balance, spectral centroid, pitch and stereo width.
`harmonics.py`, `chan.py` and `envcmp.py` drill into single probes (harmonic
levels, per-channel levels, envelope over time).

## Findings applied so far

| Behaviour | Measured | Source |
|---|---|---|
| Envelope times | 32·n⁵ s; Env 1 attack 1 ms, others 0.5 ms | parameter oracle |
| Amp envelope | level applied squared (sustain 0.5 = −12 dB) | parameter display, decay probe |
| Filter cutoff | 8.1758 Hz·(22050/8.1758)ⁿ | parameter display |
| MG Low 6/12/18/24 | ladder tapped after 1–4 stages, no resonance gain compensation | filter probes |
| Oscillator level | plain amplitude, default 0.5625 (−5 dB); frames play as stored, DC kept | init and table-position probes |
| Master volume | plain amplitude +3 dB, default 0.25 | parameter display |
| Pan | −4.5 dB law, unity at centre | pan probe |
| Random phase | one draw per note shared by oscillators A–C | OSC B probe |
| Sub level | −1.2 dB relative to a wavetable oscillator | sub probe |
| Noise | mono, −7.4 dB for uniform white noise | noise probe |
| Unison blend | scales the centre voice; 75 weighs all voices equally | blend probe |
| Filter balance | −100 (filter 1) … +100 (filter 2), default −100 | parameter oracle |

## FX findings

FX modules are not host parameters, so their tables come from state readback
(`tools/fx_defaults*.py`): Serum clamps out-of-range values (giving min/max) and
omits a field equal to its default (a candidate that disappears is the default).
171 of 187 defaults were pinned this way; the rest are marked assumed.
Mod slots address FX modules as `rack × 100 + position` (3,666/3,666 factory
slots agree).

| Behaviour | Measured |
|---|---|
| FX output level | 0.5 = unity, squared (0.25 → −12 dB) |
| Wet/dry | equal-power (partial mixes keep level) |
| Distortion | −11 dB output trim after the shaper |
| Compressor threshold | depth below 0 dB, ≈ −32·t dB; default ratio ≈ 1:1 |
| Delay | free time in seconds |
| Reverb width | 0–100 %, default 20 % |
| Bode width | spreads up-shift left, down-shift right |
| Routing slots 5, 6 | filter 1/2 outputs (destination and bus sends) |

FX probe set (30 single-module probes on Init): within 1.3 dB loudness on
average. Open: reverb decay and width at large sizes, chorus at full wet, phaser
brightness, hyper/dimension character, Bode shift scale, EQ shelf frequency and
Q, compressor detector (3 dB over-compression at limiter settings), multiband
compression (Serum −8.6 dB at defaults; not modelled).

## Warp and sample findings

| Behaviour | Measured |
|---|---|
| Sync | ratio ≈ 3 at 0.5, ≈ 17 at 1 |
| PWM | window closes fully at 1 (silent) |
| Remap 2–4 | cycle repeated k times |
| Flip | time reversal (magnitude spectrum unchanged) |
| Mirror | cycle compressed before the split point, negated after |
| Quantize | phase quantization, 2 steps at full amount |
| Odd/Even | isolated part keeps full level |
| LPF / HPF warp | amount is cutoff (LPF untouched at 1; HPF 2 octaves at 0.5) |
| Sample oscillator | matches within 0.3 score (level, pitch, start, octave) |
| Multisample regions | mapped by the oscillator's transposed pitch |

Open: cross-mod warps (FM/FME/FML/PD/AM/RM), self-PD, sync at full, folds;
multisample level (Serum 6–9 dB louder, not a constant) and velocity-layer
selection; filter-sourced warps not probed; sample warps not rendered.

## Open discrepancies (probe set, worst first)

- Filter drive: Lumen −17.7 dB at drive 80.
- Sub round-rect, triangle, square, pulse: spectra and levels differ.
- Unison: about +2 dB and −4 dB width; blend 0 still differs.
- Envelope curve parameter: Serum bends harder than the current mapping.
- High-pass and notch: centroid +0.28 octave.
- Pitch above the root: octave-spaced mip levels drop the top harmonics early.
