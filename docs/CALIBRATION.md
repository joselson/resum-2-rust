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
| Amp envelope | runs in amplitude toward sustain² (0.5 = −12 dB); segments are curved in amplitude, not squared | parameter display, osc4 sustain probes |
| Filter cutoff | 8.1758 Hz·2^(11.4·n) (to 22.1 kHz) | parameter display; comb delays pin the span within a cent |
| Oscillator level | plain amplitude, default 0.5625 (−5 dB); frames play as stored, DC kept | init and table-position probes |
| Master volume | plain amplitude +3 dB, default 0.25 | parameter display |
| Pan | −4.5 dB law, unity at centre | pan probe |
| Random phase | one draw per note shared by oscillators A–C | OSC B probe |
| Sub level | −1.2 dB relative to a wavetable oscillator | sub probe |
| Noise | mono, −7.4 dB for uniform white noise | noise probe |
| Unison | see "Oscillator core findings" | osc2–osc4 probes |
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
| Wet/dry | wet (1 − cos πw)/2, dry 1 − wet² (fxodm- probes: delay echoes, distortion at drive 0, exact to 10⁻⁴); the compressor crossfades linearly |
| Delay | free time in seconds |
| Reverb width | 0–100 %, default 20 % |
| Bode width | spreads up-shift left, down-shift right |
| Routing slots 5, 6 | filter 1/2 outputs (destination and bus sends) |

Step 4 measurements (`fxr-` responses through `tools/filter_response.py --prefix fxr`,
`fxd*` distortion transfer curves through `tools/fit_distortion.py` (runs fxd4 and,
for folds, rectify, X-shapers and sine shaper, fxd5 with `tools/fit_folds.py`), `fxdl*` delay
taps from a click, `fxrv*` reverb impulses, `fxc-`/`fxcr-` compressor static curves
through `tools/fit_comp.py` and `fxct-`/`fxcr-step-` level steps; `fxmb-`/`fxmbsq-`
multiband static curves of sines and squares at notes 48, 72, 96 — on and either side
of the crossovers, so band shares and leakage separate the bands — and `fxmbt-`
multiband level steps; `fxrv6-` reverb impulse responses per type and control through
`tools/fit_reverb.py`, `fxrv8-` grids of two Abyss or Vintage controls (runs fxrv8–10,
fxrv12), `fxrv11-` Vintage on a held saw, `fxrvf-` factory reverb modules read at
run time; `fxod-` overdrive and tape sat at 12 input levels through
`tools/fit_overdrive.py`, `fxodt-` an overdrive level step, `fxodf-` a quiet saw for
overdrive's response, `fxodw-` its 50 % wet at C1 and C2, `fxodm-` wet laws):

| Behaviour | Measured | Fit |
|---|---|---|
| EQ | RBJ shelf/peak/cut biquads; Q = 0.1 + 18.8·r⁴ (peak, cut), 0.4 + 1.74·r³ (shelf) for resonance r | 0.02 dB mean over 39 responses |
| Distortion curves | memoryless for tube, soft/hard clip, diodes, asym, rectify, stomp box, tape sat, soft sat, zero-square: tabled F(mode, drive, x) at 9 drives over ±10.5 inside the rack (output ÷ master gain); probes aligned by the chain's 6-sample latency, which every tabled mode's quietest probe shows (a few samples off, dense folds trace open loops that looked level-dependent; rectified output barely correlates with its input) | harmonics within 0.1–2 dB; tape sat output peaks 0.06 dB over 12 levels |
| Folds | static: 0.83·P(φ), φ = ln(1 + G·\|x\|)/ln r signed, G = G0·(1 + 2130·(drive/100)³); linear fold r = 1.081, G0 = 0.0235, P a triangle (zeros at integers, ±1 at ±½); sine fold r = 1.25, G0 = 0.0671, P = 2φ up to ±½ and sin(πφ) beyond; both pass x/2 at drive 0 below the first fold; where folds are dense Serum's output is smeared toward zero (Lumen: first-order antiderivative anti-aliasing in φ). The negative half folds with G 0.15–0.3 % higher (not modelled) | every zero crossing within 0.1 % (9 drives × 3 ranges); fxd5 probe score 4.9/6.1 → 0.51/0.43 |
| Sine shaper | F(sin(ω·x)) then a one-pole high-pass at 11.95 Hz; ω = 0.738·(1 + 11.83·(drive/100)²), F odd, tabled per drive on \|sin\| (no closed form found: A·sin(k·sin) and a tanh-warped inner sine fit only below drive 50) | ω within 0.3 %; fxd5 5.9 → 0.43 (the loudest range, densely folded, keeps 4–6 dB spectral errors) |
| X-shaper (both) | the default drawn curve is x/2 clipped at ±0.83, then the same 11.95 Hz high-pass; drive does nothing and asym is identical with the default curve; the curves are stored per module (`flex`: points, values, tensions, two curves) and not read yet | high-pass fit 4·10⁻⁵ of peak; fxd5 4.4 → 0.10 |
| Overdrive | 3rd-order Butterworth high-pass at 33.9 Hz (−2.9 dB, +137° at 33 Hz; its phase made C1 sines look inverted), flat gain −8.6 + 0.39·drive dB, compression 10.2:1 above a knee of 1.42 + 0.050·drive dB on the filtered peak, hard clip at ±1.091 inside the rack; after a 26 dB drop the gain recovers at ≈ 22 dB/s (peak detector, 0.33 s release; the first 100 ms recover faster, not modelled) | gains 0.03 dB (fit), 0.4 dB in Lumen at C1 (the high-pass is 0.6 dB steeper there); response within 0.1 dB, 2° above 65 Hz |
| Distortion drive 0 | diode 1, stomp box, tape sat pass x/2 unshaped | renders |
| Downsample | holds 1 + 96·drive² samples | exact |
| Distortion filter | resonant 2-pole (BW is resonance: Q 1.2, 2.05, 3.9, 20.7 at BW 0.5, 1.95, 4, 7), LP → unnormalised BP → −HP morph, −0.9 dB | 1.2 dB mean (top-octave shape) |
| Delay times | synced: straight divisions 1/512–4 bars by threshold table; free: value − 1 ms; link uses the right time | sweep at 0.0005 |
| Delay feedback / ping-pong | each repeat × feedback; ping-pong hands L → R at full level, R → L × feedback | taps |
| Delay filter | 2-pole band-pass, 0 dB peak, 1/Q ≈ 2^(BW − 1.2) | ≈ 1 dB |
| Delay mode 2 | reverse: each chunk plays the one two back reversed, ×2 output, 5 ms edge fades | ≈ 1 dB envelope |
| FX filter | the voice-filter models, best at 2× oversampling | 0.67 dB mean |
| Compressor threshold | 3.354·(1 − t)³ in amplitude inside the rack (+10.5 dB at 0, −7.55 dB at the default 0.5), floored 120 dB down | 0.1 dB |
| Compressor defaults | ratio 4, attack ≈ 90, release ≈ 90 (equal attack and release render alike; the limiter's lookahead pins attack) | 0.05 dB |
| Compressor (ratio < 100) | per-sample overshoot 20·log10(max \|L\|,\|R\|) − threshold, floored at 0, smoothed in dB: attack τ = attack/ln 10 (90 % in the set time), release τ = 0.96·release; gain −(1 − 1/ratio)·overshoot; no knee | 0.1 dB static and steps |
| Compressor (ratio ≥ 100) | brickwall limiter: lookahead 1 ms + attack/200 (output and dry delayed), window peak, instant attack, release τ ≈ 0.35·release; 50 still compresses | 0.1 dB static, ≈ 1 dB release |
| Compressor makeup | linear gain 1 + 1.033·(m − 1), none up to 1 | 0.03 dB |
| Compressor output | hard clip at ±3.313 inside the rack, then a one-pole 12.0 Hz high-pass (wet path) | 2·10⁻⁸ relative error |
| Compressor wet | linear crossfade with the aligned dry, compensated or not | 0.03 dB |
| Multiband split | LR4 crossovers, defaults 128 Hz and 2.5 kHz; the low band passes an allpass at the upper crossover; band gains (±24 dB) apply after detection | 0.03 dB |
| Multiband detector | per band (stereo link assumed): \|x\| through a 0.43 ms one-pole, then a follower with a fixed 1.5 ms attack (kParamAttack unused) and release τ = 0.97·max(release, 10 ms) | onset 0.4 dB, release ramps 0.2 dB |
| Multiband downward | hard knee at −23.35 / −22.48 / −30.01 dB (low/mid/high) on the detector level at Thresh 0.5; ThreshUD 0.24 dB per unit from 100; slope (1 − 1/ratio)·Ratio*b*, no limiter path; reduction at most 40 dB | 0.13 dB mean over 1463 points |
| Multiband threshold | mid-band shift +22.7, +19.4, +10.9, +5.7, 0, −6.4, −13.4, −37.8, −67.5 dB at Thresh 0.25–0.9 (not a single log law); low and high bands move ≈ 1.1× and 1.2× as far | 0.05–0.2 dB per point |
| Multiband upward | below threshold − Deadband*b* (+ −1.1 / +0.2 / +0.9 dB), slope RatioBelow·(2·RatioBelow*b* − 1), lift at most 35.2 dB; RatioBelow*b* < 0.5 expands; leakage into the other bands is lifted too (+2.4 dB on a mid-band sine) | 0.2 dB, ≈ 1 dB at Deadband 3/24 |
| Multiband makeup, output, wet | as single-band; the dry for the wet mix is the band sum (no cancellation at the crossovers) | 0.1 dB |
| Reverb input sleep | the reverb stops once its input is silent (a −171 dB floor counts, −131 dB does not) before its tail emerges: a lone 2 ms click into Hall above size ≈ 10, Vintage or Space renders nothing after the click; impulse probes sustain the click at −131 dB | renders |
| Reverb types | an omitted kParamType is a fifth type (Lumen: kUnnamed), not Hall; per type the controls differ: Hall — Delay sets the decay (0 → 1.2 s, default 2.6 s, 100 → 15 s at 500 Hz), Size lengthens it above 50, FreqB damps highs, an inverted burst (≈ −13 dB) in the first milliseconds, then a gap until the tail (17 ms at Size 0, 42 ms at the default), decorrelated at any width; Vintage — Size sets the decay (0.5–14.5 s) but barely below VintageScale 25, Delay 0 cuts it to 0.5 s at any Size or VintageScale (a step: Delay 2 acts like 10), VintageScale sets the decay and a gap before the tail (8 ms at 0, 13 ms at 50, 70 ms at 60, ≈ 120 ms at 65–71; above ≈ 72 an early part at 10–20 ms and a burst near 200 ms), Mode 0 adds ≈ 40 ms at any VintageScale, Feedback unused; Abyss — swells in over ≈ 200 ms, then falls 20 dB quickly onto a slow plateau; onset ≈ 110 ms, Delay, Mode (0 long, 100 ≈ 0.02–0.3 s) and Size set the decay; predelay lengthens the decay 3–4× and lowers the level ≈ 4 dB except at Mode 100, and from 0.3 s the onset is the predelay alone; Space — mono, onset ≈ 6 + 1.85·Size ms, Delay sets the decay; unnamed — onset 40 ms, Size sets the decay, Feedback damps, FreqB 100 silences, Delay unused; Freq is a low cut steeper than one pole | one control at a time |
| Reverb predelay | 0.919 × the stored seconds on every type | 0.1 ms |
| Reverb start-up copy | Vintage at VintageScale ≈ 65–71 without predelay (< 10 ms) outputs a copy of its first input (0.62×, 6 samples late) for ≈ 8 ms, then silence until the tail; a held saw shows nothing after it, so it is a start-up transient: separated from the metrics, not emulated | click and held saw |
| Reverb model | per type the default's T60 at 125 Hz / 500 Hz / 2 kHz / 8 kHz, level against the dry click, onset and tail L/R correlation, plus per-control deltas at 9–16 values (`reverb_table`), and for Abyss and Vintage the measured excess of 13 control pairs on grids over their single deltas (bilinear, zero at a default; every grid value is also swept alone, or overlapping pairs each re-correct the single sweep's interpolation); band ratios clamped to the measured 1st–99th percentiles; Freq as a fitted one-pole low cut plus the level it misses; drives an 8-line FDN with band-split loop gains solved per block, lines shortened so a decay spans at least 4 passes, an early tap at the onset, energy normalised as g²/(1 − g²) and a fitted level term | sweeps and grids: level 0.12–0.44 dB, onset < 0.1 ms, T60 0.01–0.2 octave per band (Abyss grids 0.37 at 8 kHz), correlation < 0.1; 15 factory modules: level within 0.6 (Vintage), 1.3–2.2 (Hall, Space, unnamed), 1.6 dB (Abyss); T60 at 500 Hz within 0.3 octave except Abyss (0.6); Vintage onsets 12 ms, Abyss 22 ms |
| Sub volume | level ∝ √volume (0.5625 and 1.0 measured) | 0.0 dB |
| Amp envelope | evaluated per sample (a 2 ms click lost its peak at block rate) | click energy exact |

Open: X-shaper drawn curves (5 factory presets), the folds' small asymmetry;
overdrive's detector attack and its fast first release; the distortion filter's top-octave shape; reverb
interactions of three or more controls (Abyss factory modules still 0.6–1.5 octave
off per band), the input sleep (not emulated), beat-synced predelay, the level on held
tones (the level term is fitted on clicks: Unnamed at Size 90 and 50 % wet is 1.4 dB loud on a saw),
the tail's character beyond these metrics; the compressor's detector on non-sine material (a square measured 0.3 dB off
a sine at equal peak), ratios between 50 and 100, the limiter's release just after a
deep drop (a quick partial release, then a slower one); multiband extreme Deadband
values, moved crossovers (0.9 dB at 300 Hz), the expansion limit, stereo linking;
chorus, phaser, flanger, hyper/dimension, Bode,
convolution levels.

FX probe set (30 single-module probes on Init): within 0.3 dB loudness on
average (score 0.43). Open: reverb width on sustained input (the default probe is 6.4 dB
wider than Serum, 16 dB before the measured wet law), chorus at full wet, phaser
brightness, hyper/dimension character, Bode shift scale, EQ shelf frequency and
Q.

Factory set (60 presets): 6.79 → 6.54 with the single-band compressor (loudness
error 10.7 → 9.7 dB), 6.43 with the multiband (9.4 dB); against single-band only, 11
presets better and 4 worse (most "BA - Lately Bass 2025", "LD - Grunging Grime" and
"EP - Time Capsule", whose compressors sit behind long distortion/filter chains).
The reverb model then 6.43 → 6.23 (19 presets better, 15 worse; FX probe set
0.53 → 0.48). Reverb pair grids and the short-decay FDN: 6.23 → 6.24 (11 better, 5
worse; FX probe set 0.49). Nearly all of the loss is "MDL - Slippery Snake" (+1.6),
whose Vintage onset moved from 12 to 26 ms: Lumen's level on that held note swings
between −20 and −24 dB with the onset alone, as the wet and dry interfere per harmonic,
so the score on presets that mix wet and dry over steady tones partly tracks phase.
Overdrive and tape sat measured: 6.24 → 6.29; with the measured wet law (replacing
equal power, for every effect) 6.08 (28 better, 23 worse against 6.24; FX probe set
0.49 → 0.43, default reverb 2.46 → 0.97, convolution 1.62 → 0.39). Folds, rectify, sine shaper and X-shaper
measured: 6.08 → 6.04 (3 better, none worse: "PL - Noisy Glass" 7.0 → 5.0); the older
single-level fxd probe set 1.67 → 0.27.

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

## Oscillator core findings

Probes `osc2-`, `osc3-` and `osc4-` (C3, random phase 0 so unison voices start
coherent) are analysed by least-squares fits of sinusoids at each voice's known
frequency, which separates voices far closer than an FFT bin; resolved peaks gave
the frequencies first.

| Behaviour | Measured | Fit |
|---|---|---|
| Band limit | every harmonic below ~0.49·fs kept flat at every pitch, no aliasing above −92 dB | quarter-octave mip levels (every count below 32), fold limit 0.54·fs; top band within 0.45 dB, same top harmonic at all 12 pitches |
| Unison detune | outer voices at ±range·detune^1.25 semitones (range default 2) | detune 0.05–1, range 2/12/48, within 0.01 st |
| Detune modes | position s ∈ [−1, 1]: Linear s, Exp s·\|s\|, Inv sin(π/2·s), Super 0.654·s + 0.346·s⁴ (odd) | 3–16 voices, within 0.003 of the outer offset |
| Unison pans | pitch order alternates R, L, R … from the bottom and L, R, L … from the top; an odd count's middle voice is centred | every voice at 2–16 |
| Unison pan law | R − L = p, R + L = 0.7915 + 0.2085·p² (p = ±width) | width 0–100, within 0.003 |
| Unison levels | per voice S(n)·w: centre w = 1.109·(1 − 0.552b²), others 0.049 + 1.394·b²·(1 − 0.552b²) for blend b; S = 1.056, 1, 0.973, 0.946, 0.895, 0.869, 0.690 at 2, 3, 4, 5, 7, 8, 16 voices | ≤ 0.3 dB per voice |
| Unison stacks | slot = distance from voice n/2 (centre voices 0); lists repeat by slot: Octave n [0, 12 … 12n], Oct+5th 1 [0, 0, 7], 2 [0, 12, 7, 12], 3 [0, 12, 31]; Centre 12/24 drop the centre voice(s) | 3, 4, 5, 7, 8 voices, every voice |
| Sub shapes | sine; round-rect = sine + harmonics 2–12 ≤ −33 dB (table); triangle peak 1.218; rising saw ±1.1435; square ±0.680; pulse low −1.089 from phase 0.5 for 27.1%, high 0.405 (no DC) | harmonics within 0.2 dB down to −40 dB |
| Sub phase | starts half a cycle into each shape at phase 0 | h1 phase within 0.005 cycles |
| Envelope curves | each segment (e^{k·u} − 1)/(e^k − 1) in amplitude; k odd about 50, tabled: x = \|50 − c\|/50 → 0.1: 1.11, 0.2: 2.12, 0.5: 5.41, 0.8: 11.0, 0.9: 14.9, 1: ≈ 53 | 23 curve values × 3 segments, rms ≤ 0.003 |

Probe scores: osc2 1.02 → 0.08, osc3 0.30 → 0.03, osc4 1.33 → 0.02; also the
original probe set 0.61 → 0.19, FX 0.89 → 0.63, compressor 0.78 → 0.64, warp
1.90 → 1.79, sample 0.78 → 0.72; factory set 7.33 → 7.21.

Open: Random detune spacing (not measured), stacks above 8 voices (lists assumed
to repeat), a constant ~76 µs latency in Serum's renders (h1 phase at 523 Hz),
unison span.

## Modulation findings

Probes `lfo-` and `mod-` route a source to OSC A's volume from 0, so the output level
traces the modulation (level = 0.2035·host², so host = √(level/0.2035)); a 0.5 Hz
triangle LFO sweeps curves, Env 2 steps, and the Fixed source gives static depths.
`tools/display_sweep.py` reads division lists from a host parameter's display with
hidden flags set by a loaded probe.

| Behaviour | Measured | Fit |
|---|---|---|
| LFO smooth | default 0 (the fitted table had 31.7, smoothing every LFO) | depth at 0.25–32 Hz within 0.003 |
| Default LFO shape | triangle from 0 at phase 0 | point for point |
| LFO free rate | stored value is Hz; ×10 multiplies the free rate only | 0.25–32 Hz |
| LFO sync | 15 cells ⌊14x + 0.16⌋ (plain = 100x⁴): 32 bars … 1/256, fast; dotted gives the cell's last two thirds to the next division dotted, triplets its last third to its own triplet | 19 rates, dotted, triplet at 120 BPM |
| LFO direction | 1 reverse, 2 forward then backward over two periods | trace |
| Lorenz | σ 10, ρ 28, β 8/3; 2 units/s/Hz; from (0.405, 0.405, 0); X (x + 20)/40, Y (z − 2.16)/50.5 | rms ≤ 0.014 for 1–4 s |
| Rössler | a = b = 0.2, c 5.7; 5.85 units/s/Hz; from (0, 0.32, 0); X (x + 10.1)/22.5, Y (y + 11.7)/22.5 | rms ≤ 0.014 for 4 s |
| Path LFO | equal time per segment, raw XY, closed paths return; tension bows (approximate) | 3 paths |
| One curve law | envelope segments, breakpoint tension (amount 2·(0.5 − t)) and slot curve-in (c/100) all use the same k table | k within 0.1 across all three |
| Bipolar | spans the amount peak to peak (amount 50 swings ±0.25) | trace |
| Slot out | scales the output (50% halves) | trace |
| Aux curve | the aux source's input curve, like curve-in | trace |
| Master tuning | ±64 semitones around 0.5 | 0.05 cents |
| Voice amp | linear gain, 0.5 unity | 0.0 dB |
| Envelope start / end | attack starts at start; release settles at end | trace |
| Envelope sync | 47 cells round(46x) (plain = 32x⁵): fast, then 1/1024 … 32 bars in threes (division, next triplet, dotted) | 3 attacks at 120 BPM |

Mod routes applied across the factory library: 7,417 → 12,930. Factory set 7.21 → 6.95.

Open: slot smoothing and curve-out (no effect found in single-note probes),
LFO mono and anchored (no single-note effect), LFO point modulation, the discrete
random and audio-rate (oscillator, filter) sources, slot delay.

## Open discrepancies (probe set, worst first)

- Multisample level (see warp and sample findings).
