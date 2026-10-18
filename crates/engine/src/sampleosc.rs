//! Sample and multisample oscillators: loading, region selection and playback.

use crate::params::{ParamRef as P, Values};
use crate::sfz::{self, SfzRegion};
use crate::tables::TableStore;
use lumen_dsp::env::{EnvParams, Envelope};
use lumen_dsp::sampler::{LoopMode, Playhead, Region, SampleData};
use lumen_dsp::Rng;
use lumen_preset::Value;
use std::sync::Arc;

/// What an oscillator slot plays besides wavetables.
#[derive(Clone, Default)]
pub enum SampleSource {
    #[default]
    None,
    Sample { data: Arc<SampleData>, root: f32 },
    Multi(Arc<Vec<LoadedRegion>>),
}

pub struct LoadedRegion {
    pub sfz: SfzRegion,
    pub data: Arc<SampleData>,
}

/// Loads oscillator `i`'s sample or multisample per its type; errors go to `errors`.
pub fn load(payload: &Value, i: usize, kind: &str, tables: &mut TableStore, errors: &mut Vec<String>) -> SampleSource {
    let osc = payload.get(&format!("Oscillator{i}"));
    match kind {
        "kOsc_Sample" => {
            let Some(s) = osc.and_then(|o| o.get(&format!("SampleOsc{i}"))) else { return SampleSource::None };
            let Some(path) = s.get("samplePathRelative").and_then(Value::as_str) else { return SampleSource::None };
            match tables.audio("Samples", path) {
                Ok(data) => SampleSource::Sample { data, root: s.get("baseNote").and_then(Value::as_f64).unwrap_or(60.0) as f32 },
                Err(e) => {
                    errors.push(e);
                    SampleSource::None
                }
            }
        }
        "kOsc_MultiSample" => {
            let Some(m) = osc.and_then(|o| o.get(&format!("MultiSampleOsc{i}"))) else { return SampleSource::None };
            let Some(text) = m.get("embedded_sfz").and_then(Value::as_str) else { return SampleSource::None };
            // Region samples are relative to the .sfz file's directory under Multisamples.
            let dir = m.get("sfzPathRelative").and_then(Value::as_str).map(|p| p.replace('\\', "/")).and_then(|p| p.rsplit_once('/').map(|(d, _)| d.to_owned())).unwrap_or_default();
            let mut regions = Vec::new();
            for r in sfz::parse(text) {
                match tables.audio("Multisamples", &format!("{dir}/{}", r.sample)) {
                    Ok(data) => regions.push(LoadedRegion { sfz: r, data }),
                    Err(e) => errors.push(e),
                }
            }
            if regions.is_empty() { SampleSource::None } else { SampleSource::Multi(Arc::new(regions)) }
        }
        _ => SampleSource::None,
    }
}

const START: P = P::new("Oscillator", 0, "kParamStart");
const END: P = P::new("Oscillator", 0, "kParamEnd");
const LOOP_START: P = P::new("Oscillator", 0, "kParamLoopStart");
const LOOP_END: P = P::new("Oscillator", 0, "kParamLoopEnd");
const LOOP_MODE: P = P::new("Oscillator", 0, "kParamLoopMode");
const LOOP_XFADE: P = P::new("Oscillator", 0, "kParamLoopCrossfade");
const REVERSE: P = P::new("Oscillator", 0, "kParamReverse");
const RANDOM_START: P = P::new("Oscillator", 0, "kParamRandomStart");
const MS_ENV_OVERRIDE: P = P::new("MultiSampleOsc", 0, "kParamEnvOverride");
const MS_ENV: [P; 5] = [
    P::new("MultiSampleOsc", 0, "kParamEnvAttack"),
    P::new("MultiSampleOsc", 0, "kParamEnvHold"),
    P::new("MultiSampleOsc", 0, "kParamEnvDecay"),
    P::new("MultiSampleOsc", 0, "kParamEnvSustain"),
    P::new("MultiSampleOsc", 0, "kParamEnvRelease"),
];
const MS_VEL_OVERRIDE: P = P::new("MultiSampleOsc", 0, "kParamVelTrackOverride");
const MS_VEL: P = P::new("MultiSampleOsc", 0, "kParamVelTrack");

/// Parameters the sample engine renders (for coverage).
pub(crate) const RENDERED: &[(&str, &str)] = &[
    ("Oscillator", "kParamStart"), ("Oscillator", "kParamEnd"), ("Oscillator", "kParamLoopStart"), ("Oscillator", "kParamLoopEnd"),
    ("Oscillator", "kParamLoopMode"), ("Oscillator", "kParamLoopCrossfade"), ("Oscillator", "kParamReverse"), ("Oscillator", "kParamRandomStart"),
    ("MultiSampleOsc", "kParamEnvOverride"), ("MultiSampleOsc", "kParamEnvAttack"), ("MultiSampleOsc", "kParamEnvHold"),
    ("MultiSampleOsc", "kParamEnvDecay"), ("MultiSampleOsc", "kParamEnvSustain"), ("MultiSampleOsc", "kParamEnvRelease"),
    ("MultiSampleOsc", "kParamVelTrackOverride"), ("MultiSampleOsc", "kParamVelTrack"),
];

const MAX_LAYERS: usize = 8;

#[derive(Clone, Copy, Default)]
struct Layer {
    head: Playhead,
    region: Region,
    /// Index into the multisample's regions; 0 for a single sample.
    index: usize,
    root: f32,
    /// The sample's own rate, Hz.
    source_rate: f64,
    gain: [f32; 2],
    env: EnvParams,
    use_env: bool,
}

/// Per-voice sample playback for one oscillator slot.
#[derive(Clone, Default)]
pub struct SampleVoice {
    layers: [Layer; MAX_LAYERS],
    envs: [Envelope; MAX_LAYERS],
    count: usize,
}

fn pct(values: &Values, p: P, i: usize) -> f64 {
    (values.get(p.on(i)) as f64 / 100.0).clamp(0.0, 1.0)
}

fn sfz_loop(mode: Option<&str>, has_points: bool) -> LoopMode {
    match mode {
        Some("loop_continuous") => LoopMode::Forward,
        Some("loop_sustain") => LoopMode::Tailed,
        Some("no_loop") | Some("one_shot") => LoopMode::OneShot,
        // Explicit loop points without a mode loop continuously.
        _ if has_points => LoopMode::Forward,
        _ => LoopMode::OneShot,
    }
}

fn oscillator_loop(name: &str) -> LoopMode {
    match name {
        "kForward" => LoopMode::Forward,
        "kReverse" => LoopMode::Reverse,
        "kPingPong" => LoopMode::PingPong,
        "kTailed" => LoopMode::Tailed,
        _ => LoopMode::OneShot,
    }
}

impl SampleVoice {
    /// Selects and starts layers. `note` is the oscillator's transposed pitch (regions
    /// map by it, measured: an octave-down oscillator picks the lower regions);
    /// `velocity` ∈ [0, 1].
    pub fn trigger(&mut self, source: &SampleSource, values: &Values, i: usize, note: u8, velocity: f32, rng: &mut Rng) {
        self.count = 0;
        match source {
            SampleSource::None => {}
            SampleSource::Sample { data, root } => {
                let len = data.len() as f64;
                let start = pct(values, START, i) * len;
                let end = (pct(values, END, i).max(pct(values, START, i)) * len).max(start + 1.0);
                let end = if values.get(END.on(i)) <= 0.0 { len } else { end };
                let region = Region {
                    start,
                    end,
                    loop_start: start.max(pct(values, LOOP_START, i) * len),
                    loop_end: (pct(values, LOOP_END, i) * len).min(end).max(start + 2.0),
                    loop_mode: oscillator_loop(values.choice(LOOP_MODE.on(i))),
                    crossfade: pct(values, LOOP_XFADE, i) * 0.5 * len * 0.1,
                    reverse: values.flag(REVERSE.on(i)),
                };
                let offset = pct(values, RANDOM_START, i) * rng.unit() as f64 * (end - start);
                self.push(Layer { region, root: *root, source_rate: data.rate as f64, gain: [1.0, 1.0], ..Layer::default() }, offset);
            }
            SampleSource::Multi(regions) => {
                let vel = (velocity * 127.0).round() as u8;
                // Round robin: one position per note, chosen at random among the sequence.
                let seq_pick = rng.next_u32();
                for (index, lr) in regions.iter().enumerate() {
                    let r = &lr.sfz;
                    if note < r.lokey || note > r.hikey || vel < r.lovel || vel > r.hivel {
                        continue;
                    }
                    if r.seq_length > 1 && r.seq_position != seq_pick % r.seq_length + 1 {
                        continue;
                    }
                    let len = lr.data.len() as f64;
                    let end = r.end.map(|e| (e + 1) as f64).unwrap_or(len).min(len);
                    let has_points = r.loop_start.is_some() && r.loop_end.is_some();
                    let region = Region {
                        start: (r.offset as f64).min(end - 1.0),
                        end,
                        loop_start: r.loop_start.unwrap_or(0) as f64,
                        loop_end: r.loop_end.map(|e| (e + 1) as f64).unwrap_or(end).min(end),
                        loop_mode: sfz_loop(r.loop_mode.as_deref(), has_points),
                        crossfade: (r.loop_crossfade * lr.data.rate) as f64,
                        reverse: false,
                    };
                    let veltrack = if values.flag(MS_VEL_OVERRIDE.on(i)) { values.get(MS_VEL.on(i)) } else { r.amp_veltrack } / 100.0;
                    let amp = 10f32.powf(r.volume / 20.0) * (1.0 - veltrack + veltrack * velocity * velocity);
                    let pan = (r.pan / 100.0).clamp(-1.0, 1.0);
                    let env = if values.flag(MS_ENV_OVERRIDE.on(i)) {
                        let e = MS_ENV.map(|p| values.get(p.on(i)));
                        EnvParams { attack: e[0], hold: e[1], decay: e[2], sustain: e[3], release: e[4], curves: [0.0; 3] }
                    } else {
                        EnvParams { attack: r.ampeg[0], hold: r.ampeg[1], decay: r.ampeg[2], sustain: r.ampeg[3], release: r.ampeg[4], curves: [0.0; 3] }
                    };
                    let layer = Layer {
                        region,
                        index,
                        root: r.pitch_keycenter as f32 - r.tune / 100.0,
                        source_rate: lr.data.rate as f64,
                        gain: [amp * (1.0 - pan).min(1.0), amp * (1.0 + pan).min(1.0)],
                        env,
                        use_env: true,
                        head: Playhead::default(),
                    };
                    self.push(layer, 0.0);
                    if self.count == MAX_LAYERS {
                        break;
                    }
                }
            }
        }
    }

    fn push(&mut self, mut layer: Layer, offset: f64) {
        if self.count == MAX_LAYERS {
            return;
        }
        layer.head.trigger(&layer.region, offset);
        self.envs[self.count] = Envelope::default();
        self.envs[self.count].trigger(false);
        self.layers[self.count] = layer;
        self.count += 1;
    }

    pub fn release(&mut self) {
        for e in &mut self.envs[..self.count] {
            e.release();
        }
    }

    /// Next stereo sample at fractional MIDI `pitch` (oscillator pitch incl. bend/tuning).
    #[inline]
    pub fn next(&mut self, source: &SampleSource, pitch: f32, held: bool, sample_rate: f32) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for k in 0..self.count {
            let layer = &mut self.layers[k];
            let data = match source {
                SampleSource::Sample { data, .. } => data,
                SampleSource::Multi(regions) => &regions[layer.index].data,
                SampleSource::None => return (0.0, 0.0),
            };
            let step = layer.source_rate / sample_rate as f64 * (((pitch - layer.root) / 12.0) as f64).exp2();
            let (a, b) = layer.head.next(data, &layer.region, step, held);
            let env = if layer.use_env { self.envs[k].advance(&layer.env, sample_rate, 1) } else { 1.0 };
            l += a * layer.gain[0] * env;
            r += b * layer.gain[1] * env;
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multisample_selects_by_key_and_velocity() {
        let data = Arc::new(SampleData { rate: 48000.0, channels: vec![vec![0.5; 4800]] });
        let regions = sfz::parse("<region> lokey=0 hikey=59 hivel=63 sample=a.wav\n<region> lokey=0 hikey=59 lovel=64 sample=b.wav\n<region> lokey=60 hikey=127 sample=c.wav\n");
        let source = SampleSource::Multi(Arc::new(regions.into_iter().map(|sfz| LoadedRegion { sfz, data: data.clone() }).collect()));
        let values = Values::default();
        let mut v = SampleVoice::default();
        let mut rng = Rng::new(1);
        v.trigger(&source, &values, 0, 40, 0.9, &mut rng);
        assert_eq!((v.count, v.layers[0].index), (1, 1), "loud low note picks region b");
        v.trigger(&source, &values, 0, 72, 0.2, &mut rng);
        assert_eq!((v.count, v.layers[0].index), (1, 2));
        let (l, _) = v.next(&source, 60.0, true, 48000.0);
        assert!(l.abs() > 0.0 && l.is_finite());
    }
}
