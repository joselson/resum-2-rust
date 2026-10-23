# Next steps

The parity roadmap, in order. Steps 1–3 are done; step 4 is in progress. Every item
is matched to Serum 2 by black-box measurement (see [CALIBRATION.md](CALIBRATION.md)
for the method and what is already fitted, [PARITY.md](PARITY.md) for feature status).

## 4. FX character (in progress)

Done: EQ, delay (times, feedback, ping-pong, reverse, filter), FX filter, distortion
(11 static modes with tape sat and rectify, the folds and sine shaper, the X-shapers'
default curve, downsample, pre/post filter, overdrive's high-pass, gain, 10:1
compression and clip), the FX wet law, compressor: single-band
(threshold, ratio and limiter paths, attack/release, makeup, output clip, wet) and
multiband (crossovers, detector, per-band thresholds, ratios, upward compression,
gains); reverb per-type metrics and Abyss/Vintage pair interactions; chorus (voices,
LFO, depth, feedback, filter).

Remaining, roughly by factory usage:

1. **Reverb leftovers** (425 presets). Per-type metrics are fitted (`fxrv6-` sweeps,
   `fxrv8-` pair grids, `tools/fit_reverb.py`). Open:
   - Abyss interactions of three or more controls (factory modules 0.6–1.5 octave
     off per band; predelay changes the whole structure): more factory modules or a
     structural Abyss model.
   - Serum's reverb stops when its input falls silent before the tail emerges; not
     emulated. Vintage's start-up copy of the input (also not emulated).
   - Level on held tones (Unnamed Size 90 at 50 % wet) and width: the level term is
     fitted on clicks.
   - Beat-synced predelay; the tail's character (Lumen's FDN only matches the
     metrics); Wet 0 modules (238 factory reverbs) rely on modulated wet.
2. **Modulation effects**: flanger, phaser (brightness), hyper/dimension (width
   −9.5 dB), Bode shift scale; the chorus's high-pass shape and synced rates.
3. **Convolution** level and IR handling; **FX Utility** remaining controls.
4. Distortion filter top-octave shape (Serum falls more gently than an RBJ biquad);
   FX filter top-octave differences by type. X-shaper drawn curves (module `flex`
   data; 5 factory presets): probe a few drawn shapes against the default.
5. **Compressor leftovers**: single-band ratios between 50 and 100, its detector on
   non-sine material and the limiter release after deep drops; multiband extreme
   Deadband values (≈ 1 dB off at 3 and 24), moved crossovers (0.9 dB at 300 Hz), the
   multiband expansion limit (band RatioBelow < 0.5; capped like the upward lift,
   unmeasured), stereo linking. Factory presets that still differ through long
   chains: BA - Lately Bass 2025, LD - Grunging Grime, EP - Time Capsule.

## 5. Arpeggiator and clip player

`Arp` is used by 237 factory presets, `ClipPlayer` by 178. Not implemented.

## 6. Granular and spectral oscillators

Not implemented; also sample/multisample warps and multisample level
(Serum 6–9 dB louder, not constant).

## 7. MPE and oversampling

MPE / note expression sources; global oversampling beyond the voice and FX filters.

## 8. Serum-style editor

The current editor is a generic parameter page plus preset browser and keyboard.

## Leftovers from earlier steps

- **Filters:** formants, reverb, combs/allpasses/comb 2, ring mod, S&H, EQ types,
  diffuser smearing, dist-comb 2, EMS/Acid/Dirty ladder responses, PZ SVF and DJ
  mixer at non-default Var, full resonance of comb/phase types.
- **Oscillators:** Random detune spacing, stacks above 8 voices, unison span, a
  ~76 µs latency in Serum's renders.
- **Modulation:** mod-slot smoothing and curve-out (no effect found in single-note
  probes), LFO mono and anchored, LFO point modulation, discrete random and
  audio-rate (oscillator, filter) sources, slot delay, envelope legato invert.
- **Tooling:** `tools/make_probes.mjs` and `tools/preset_keys.mjs` import
  `serumpreset.mjs` from outside the repository; move it into `tools/` so a fresh
  clone can generate probes.
