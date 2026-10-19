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
| Filter cutoff | 8.1758 Hz·2^(11.4·n) (to 22.1 kHz) | parameter display; comb delays pin the span within a cent |
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

## Filter findings

Filter probes play C1 (32.7 Hz) so the harmonics sample |H(f)| every 32.7 Hz;
`tools/filter_response.py RUN` divides each probe's harmonics by the `filt-off`
render, separately for Serum and Lumen, and ranks the response error (RMS dB,
40 Hz–16 kHz, within 50 dB of the peak). Prototypes were fitted to Serum's
curves before implementing them.

```bash
node tools/make_probes.mjs <init state> <lumen-ref>/probes-filt '^filt-'
lumen-compare jobs --presets <lumen-ref>/probes-filt --dir <run> --count 1000 --notes 24
python3 tools/filter_response.py <run> [--plot out.png 'REGEX']
```

| Behaviour | Measured | Fit |
|---|---|---|
| Oversampling | filters run at 4× by default (`kParamOversampling` 1 or 2), 2× at 0; the default is 1 | top-octave response vs bilinear at 96/192 kHz |
| Multimode L/H/B/N/P | one resonant 2-pole section; Q 0.5→97.8 over resonance (table); output × 1/(1 + 4r) except notch | 0.05–0.15 dB |
| 24 dB types | second section Q 0.5 → 0.99 by r = 0.5 | 0.1–0.2 dB |
| 18 / 6 dB types | resonant section × one-pole / one-pole alone | 0.1–0.2 dB |
| Band | raw band output (peak gain Q) | 0.1 dB |
| Peak | (s² + s + 1)/(s² + s/Q + 1): −6 dB dip at r = 0 | 0.1–1.4 dB |
| MG Low 6–24 | linear ladder, feedback 4.04·r, tapped after 1–4 stages, no compensation | 0.1 dB |
| MG Ladder | ladder with (1 + k)·−2 dB compensation, feedback 4·r, cutoff ×1.1 | 0.01–0.13 dB |
| Ladder saturation | headroom 14.5 (internal level): self-oscillation at r = 1 within 3% of Serum | level probes |
| Drive | input gain 0/7.8/13.9/20.2/23.9 dB at 0/25/50/80/100, cubic soft clip before the filter; bypassed at drive 0 | 0.37 dB mean |
| Drive per type | MG Ladder and EMS linear (+24 dB at 100); Acid clips at 1.25× and Dirty at 0.25× the ceiling with half the gain | output-level sweeps, 0.6 dB |
| Comb ± | delay one cutoff period; 0.5/(1 + r)·(1 ± r²z⁻ᵀ)/(1 ∓ r·z⁻ᵀ) | 0.14 dB mean (r ≤ 0.9) |
| Flange ± | 0.5/(1 + r)·(1 ± z⁻ᵀ)/(1 ∓ r·z⁻ᵀ) | 0.5 / 0.75 dB |
| Comb/flange damping | one-pole in the loop at cutoff·2^(21·(Var − 0.5)), 8 Hz–22 kHz | 3 cutoffs × 4 Var, 0.6 dB |
| Phase nP | n first-order allpasses at cutoff·0.507, feedback r over 10 µs, 0.5/(1 + r)·(loop + allpass); negative types mix −A/(1 + r), feedback −r over 23 µs | 0.5 / 0.9 dB |
| Morph duals (LH, HP, …) | first response at the cutoff, then the second at cutoff_hz(0.94·Var), resonance Q, no scaling | 0.2–0.5 dB at 3 cutoffs |
| Morph triples (L/B/H, …) | Var crossfades the three outputs in amplitude; 24 dB types cascade a second section (second-section Q) | 0.1–0.5 dB |
| Exp MM / Exp BPF | 2-pole low/raw band, own Q curve, cutoff ×1.15, −0.6/−0.85 dB | 0.2 dB |
| Wsp | 2-pole lowpass, own Q curve, cutoff ×0.98, −1 dB | 0.5 dB |
| Scream ("French LP") | 2-pole lowpass, shared Q curve saturating above r = 0.7, −12.1 dB | 0.7 dB |
| Level | squared: 0.1 → −28 dB, 1.0 → +12 dB (display and render) | 0.2 dB |
| Stereo | cutoff offset ±2.65 octaves at 0/100 (left up at 0), linear assumed | 0.3 dB |

Response error after these changes: multimode 0.36–0.45 dB mean (worst under
1 dB), MG family 0.19 dB, drive 0.37 dB, delay families 0.14–0.9 dB, morph types
0.58 dB. Probe-set response error: 12.5 → 5.0 dB (filt), 13.2 → 5.3 dB (filt2),
10.2 → 2.4 dB (filt3, delay families); the rest is types still missing or nonlinear.
Factory set (60 presets, same day): 7.49 before the filter work, 7.33 after;
21 presets better, 10 worse. The two large regressions are interactions, not filter
errors: "LD - B-Organ" loses its 33 Hz component because the sub oscillator's
level depends on unimplemented mod sources (the old lowpass-only LH12 hid it), and
"BA - Funk Me Electric" now matches loudness and spectrum but its pitch is detected
an octave off.

Open: EMS, Acid and Dirty ladder responses (nonlinear; drive measured, response
uses MG Low 24); formants, reverb, combs/allpasses/comb 2, ring mod, sample & hold,
EQ types (shelves/peaks whose gain Var sets: 50 flat, 100 ≈ +19 dB), diffuser
(magnitude flat, so pass-through matches the response but not the smearing),
dist-comb 2, add-bass, PZ SVF and DJ mixer at non-default Var; full resonance
(r = 1) of comb/phase types; the HL6 damping's Var mapping; FX filter drive and
stereo (assumed equal to the voice filter); stereo spread between 0, 50 and 100.

## Open discrepancies (probe set, worst first)

- Sub round-rect, triangle, square, pulse: spectra and levels differ.
- Unison: about +2 dB and −4 dB width; blend 0 still differs.
- Envelope curve parameter: Serum bends harder than the current mapping.
- Pitch above the root: octave-spaced mip levels drop the top harmonics early.
