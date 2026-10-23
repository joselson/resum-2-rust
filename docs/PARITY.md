# Feature parity with Serum 2 (2.0.22)

Option lists come from the plugin binary's name tables and enums; usage counts
are factory presets (of 626) that store the feature. "Approx." means Lumen maps
the option onto a generic implementation; nothing is sound-matched yet.

## Architecture

| Serum 2 (class layout) | Lumen | Status |
|---|---|---|
| `SerumDSP` processor + `SerumController` (VST3 split) | single nih-plug plugin | kept; nih-plug owns the split |
| `SerumParameter` + 26 scaled types | `params`: generated per-module tables (263 parameters) with measured curves, defaults, options | done |
| 22 `XferModule` state objects, one per preset section | `params::Values` over every module instance; `Patch::set` edits one parameter live | done |
| `XferLiveVoiceUser` per-voice runtime objects | `Voice` runtime state + per-voice `Offsets` | done |
| `Oscillator` (Noise, Sub) / `WarpOscillator` (WT, Sample, MultiSample, Granular, Spectral) | `dsp::warp` stage (two slots) on the WT oscillator; sample and multisample engines | warp done for WT; sample warps, granular, spectral open |
| `XferFilterBase`, 17 families, shared by voice and FX filters | `filters::model` maps all 107 types to models with fidelity (26 matched, 51 approximate, 30 missing) | multimode, MG, delay and morph families measured; others open |
| `XferModSource`, `ModMatrix` | routes to any module parameter; slot amount/flags are ModSlot parameters | done |
| `Arp`, `ClipPlayer` as MIDI processors before voices | `notes::NoteProcessor` stage (passthrough) | stage done |
| `NoteExpressionVoice` | none | open |
| `FXRack` of `XferFX`; splits nest sub-chains; 3 racks | `fx`: 3 racks parsed into effect/split trees, ids rack×100+position, bus routing | done (first pass) |
| thread pool + async tasks | single thread | measure first |

## Options

| Group | Serum 2 | Lumen |
|---|---|---|
| Oscillator types | WT, Sample, MultiSample, Granular, Spectral, Noise, Sub | WT, Sample, MultiSample (SFZ); noise (generated, mono); sub |
| WT warp modes | 70 (sync, bend ×3, PWM, asym ×3, flip, mirror, remap ×4, quantize, odd/even, LPF/HPF, 14 distortions, FM/FME/FML/PD/AM/RM × 6 sources, self PD) | all 70, two slots + unison warp; calibrated by probes (mean score 1.9) |
| Spectral warps | 88 | none |
| Sub shapes | sine, round-rect, triangle, saw, square, pulse | all six, measured (harmonics within 0.2 dB) |
| Noise | sample-based noise types | white with tilt |
| Unison detune | Linear, Super, Exp, Inv, Random | all five; spacing, pans, blend and width measured per voice (Random spacing approximate) |
| Unison stacks | none + 8 (octave, octave+fifth, centre 12/24) | all eight (measured to 8 voices) |
| Granular windows / density modes | 10 / 3 | none |
| Sample loop modes | forward, reverse, ping-pong, tailed | all, with crossfade |
| Filter types | 107 (MG ladder, L/H/B/P/N 6–24, dual/triple combos, comb ×8, flange ×8, phase ×16, EQ, ring mod, S&H, formant, scream, dist-comb, diffuser…) | measured models (within ~1 dB) for L/H/B/N/P 6–24, MG Low 6–24, MG Ladder, comb/flange ±, phase P, morph duals and triples, Exp MM/BPF, Wsp, Scream; drive per type, level, stereo, Var; EMS/Acid/Dirty ladders, damped combs, negative phase, German LP, Scream LP/BP, Dist.Comb 1 approximate; formant, reverb, ring mod, S&H, EQ and other exotic types missing |
| Routing destinations | Filter, Master, Direct, None | Filter, Master, None |
| Envelopes | 4 × AHDSR, 3 curves, start/end, beat sync, legato invert | 4 × AHDSR, 3 measured curves, start/end, measured beat sync; legato invert open |
| LFO types | Curve, Path, RandomSH, Lorenz, Rossler | all five (Lorenz/Rössler and their Y outputs fitted; path tension approximate) |
| LFO timing | sync with dotted/triplet/×10, delay, rise, smooth, anchored, direction, mono | measured sync divisions, dotted, triplet, ×10, direction, delay, rise, smooth; anchored and mono open |
| Mod sources | 58 | 36 (LFO Y added; audio-rate, MPE, discrete random open) |
| Mod slot options | amount, aux, bipolar, curves in/out, aux curve, out, smoothing, delay, bypass | amount, aux, bipolar (measured), drawn and input curves, aux curves, out, bypass; smoothing, curve-out, delay open |
| Distortion modes | 18 | all measured: static curves for 11 modes, overdrive's dynamics, fold and sine-shaper laws, downsample and the pre/post filter; X-shaper drawn curves not read (default curve only) |
| Reverb types | Hall, Vintage, Abyss, Space | one FDN reverb with per-type decay/modulation |
| FX modules | Distortion, Flanger, Phaser, Chorus, Delay, Comp, Reverb, EQ, Filter, HyperDimension, Bode, Convolve, Utility + Split, Split3, SplitMS | all 16; EQ, delay, FX filter, compressor (single-band and multiband), the wet law and most distortion (overdrive and tape sat included) measured; reverb fitted per type as decay, level, onset and width metrics, with pair interactions for Abyss and Vintage; modulation effects still first implementations |
| Arp / Clip player | yes | none |
| MPE / note expression | yes | none |
| Oversampling | `kParamOversampling` 0–2; filters measure as 2× at 0 and 4× at 1–2 (default 1) | voice filters at 2×/4× with polyphase IIR half-band resamplers |

## Behaviour verification

See [CALIBRATION.md](CALIBRATION.md): Serum 2 renders headlessly and the
comparison is automated. Probe set (41 single-change presets): mean score 0.77,
loudness within 1.4 dB, spectral balance within 1.5 dB per band, pitch within
10 cents on 38/38.
