# Lumen

A clean-room wavetable synthesizer in Rust that loads Serum 2 `.SerumPreset`
files. It is written from DSP fundamentals and the documented preset format; no
code is translated from the original plugin, and no Xfer content is bundled.
Wavetables are read from the user's own installation at runtime.

## Crates

| Crate | Purpose |
|---|---|
| `lumen-preset` | XferJson container + CBOR codec; byte-exact round trip |
| `lumen-dsp` | Band-limited wavetables, unison oscillator, SVF/ladder filters, AHDSR, curve LFOs |
| `lumen-engine` | Preset → patch mapping, mod matrix, voices, polyphony |
| `lumen-render` | Offline renderer and factory-preset survey |
| `lumen-plugin` | CLAP plugin, egui editor (`gui`), standalone app (`standalone`); VST3 behind `vst3`, see Licensing |
| `lumen-compare` | Render-and-compare against reference renders (see docs/CALIBRATION.md) |

## Usage

```bash
# Render a chord (Tables directory is found by walking up from the preset)
cargo run --release -p lumen-render -- "path/to/Pad.SerumPreset" -o pad.wav --notes 48,55,60,64

# Load, render and report coverage for every preset in a library
cargo run --release -p lumen-render -- --survey "path/to/Serum 2 Presets/Presets"

# Byte-exact container round trip over a preset library
LUMEN_PRESET_DIR="path/to/Presets" cargo test -p lumen-preset

# Plugin: copy target/release/liblumen_plugin.so to ~/.clap/Lumen.clap
cargo build --release -p lumen-plugin            # CLAP, no editor
cargo build --release -p lumen-plugin --features gui,vst3
cargo build --release -p lumen-plugin --features standalone --bin lumen-standalone
```

On Windows, `tools\build-windows.cmd` builds with a user-level Rust
(`x86_64-pc-windows-gnu`) and a portable MinGW-w64, and packages
`target\bundled\Lumen.clap`, `Lumen.vst3` and `lumen-standalone.exe`.
The standalone's **Audio settings** button picks the output device and MIDI
input (saved to `%APPDATA%\Lumen\standalone.json`; applying restarts the app).
It runs WASAPI at the device's own shared-mode rate: nih-plug's default 48 kHz
request fails on 44.1 kHz devices and silently falls back to a dummy backend.
Command-line arguments (`--help`) override the saved settings.
`tools/host_check.py` loads the VST3 in pedalboard and checks rendering, host
parameter changes and state restore. On Linux the `gui` feature needs the X11
and OpenGL development packages.

The editor has a preset browser (scans the loaded preset's library, or with no
preset the "Serum Presets Path" from Serum's preferences, else
`Documents\Xfer\Serum 2 Presets`; ◀ ▶ step through it), output and macro sliders, and a page of sliders per module instance
with a parameter search, and a playable keyboard (mouse, lower on a key is
louder, or `A W S E D F T G Y H U J K O L P` with `Z`/`X` for octaves). Presets load on a background thread; the audio thread
swaps the prepared patch in without allocating. The editor publishes the loaded
preset's values to the host when it opens and after each load. Without the
editor, the preset comes from saved state or `LUMEN_PRESET`, wavetables from
`LUMEN_TABLES` or the preset's library, and host parameters apply once moved.

## Status (phase 1)

Verified on the 626 Serum 2 factory presets: all load and render finite audio,
all wavetables resolve, 7,417 mod routes apply. Every preset still uses at
least one feature listed below; `--survey` ranks them.

Implemented: wavetable oscillators A–C band-limited like Serum's (flat to about
0.49·fs at every pitch) with measured unison (5 detune modes, stacks, width, blend,
table-position spread), noise, the six measured sub shapes, per-source routing and
filter balance, two
voice filters (SVF LP/HP/BP 12/24, notch, peak, ladder), four envelopes with
measured curves, ten
LFOs (curve, path, S&H, Lorenz and Rössler; measured sync divisions, dotted,
triplet, direction; trigger/free/envelope modes, delay, rise, smoothing), eight
macros, 64 mod slots with aux sources and bipolar mode, mono/legato/portamento,
pitch bend, mod wheel, aftertouch.

FX racks (main + two buses, nested splits, all 16 module types), all 70
wavetable warp modes and the sample/multisample oscillators (SFZ) render with
first-pass calibration. Filters run oversampled like Serum's (4× by default) and
the main families are measured within about 1 dB: multimode 6–24, MG ladders,
comb/flange/phase, morph duals and triples, Exp, Wsp and Scream, with drive,
level, stereo and Var (see docs/CALIBRATION.md). The plugin exposes 550 host
parameters generated from the parameter tables. Not yet implemented:
granular/spectral oscillators, arpeggiator and clip
player, the remaining exotic filter types (formant, reverb, ring mod, EQ…) and the
EMS/Acid/Dirty ladder responses, mod smoothing and audio-rate mod sources, a Serum-style editor (the
current one is a generic parameter page).

## Parameter semantics

`crates/engine/src/params/serum2.rs` is generated (`tools/gen_params.py`) from
measurements of Serum 2's host parameters: each module's parameters in Serum's
own order (so indices match mod-slot IDs), with value curve, per-instance
default, option list and evidence. See [docs/CALIBRATION.md](docs/CALIBRATION.md).

## Known issues

- nih-plug (upstream, rev `de42101`) aborts on corrupt CLAP state: it allocates
  a state buffer from an untrusted length prefix (`ext_state_load`).
  `clap-validator` reproduces it with nih-plug's own example plugin, along with
  three state round-trip failures. Lumen passes the validator's other tests.

## Licensing

Lumen's code is MIT. nih-plug's VST3 bindings are GPLv3, so binaries built with
`--features vst3` must be distributed under GPLv3. The default CLAP build has no
such constraint.
