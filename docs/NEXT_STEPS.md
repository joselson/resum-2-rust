# Next steps

The parity roadmap, in order. Steps 1–3 are done; step 4 is in progress. Every item
is matched to Serum 2 by black-box measurement (see [CALIBRATION.md](CALIBRATION.md)
for the method and what is already fitted, [PARITY.md](PARITY.md) for feature status).

## 4. FX character (in progress)

Done: EQ, delay (times, feedback, ping-pong, reverse, filter), FX filter, distortion
(9 static modes, downsample, pre/post filter).

Remaining, roughly by factory usage:

1. **Compressor** (540 instances in factory presets, 205 of them multiband).
   - Single band: static curve (threshold, ratio, knee, makeup) from a sine at
     stepped input levels (sub sine plus FX Utility boost stages, as the
     distortion probes do); attack/release from level steps.
   - Known gaps: ~3 dB over-compression at limiter settings; default ratio ≈ 1:1.
   - Multiband: crossovers (`kParamXoverLow/Hi`), per-band upward/downward
     thresholds (`kParamThreshUD0–2`), ratios above/below, gains, deadband.
     Serum is −8.6 dB at defaults; Lumen does not model it.
2. **Reverb** (425 presets).
   - The type stored when none is set is a fifth, unnamed type (101 presets);
     Hall/Vintage/Abyss/Space are stored explicitly. Lumen maps "default" to Hall.
   - Feedback does not set the decay for Hall, Abyss or the default type; size does.
   - Several isolated `fxrv` probes render silence after a few ms in the headless
     rig (Vintage and Space always, Hall at some sizes) while factory presets with
     those types do not. Find a probe setup that renders like the factory presets
     (e.g. start from a factory reverb module and change one control) before fitting.
   - Then fit decay per octave band, predelay, early level, width per type.
3. **Overdrive and tape sat distortion**: not static. Overdrive normalises its input
   by the signal level (curves coincide once x is divided by the input amplitude);
   tape sat folds level-dependently. Model the level detector, then refit.
4. **Rarer distortion modes**: linear/sine fold, X-shaper (asym), rectify, sine
   shaper (show filtering/hysteresis in the transfer curves).
5. **Modulation effects**: chorus (+3.3 dB, −6.7 dB width at 100 % wet),
   flanger, phaser (brightness), hyper/dimension (width −9.5 dB), Bode shift scale.
6. **Convolution** level and IR handling; **FX Utility** remaining controls.
7. Distortion filter top-octave shape (Serum falls more gently than an RBJ biquad);
   FX filter top-octave differences by type.

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
