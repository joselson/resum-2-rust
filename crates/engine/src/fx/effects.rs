//! Effect implementations. Each `process` produces the fully wet signal; the
//! rack applies enable, wet/dry (unless the effect owns it) and output level. Parameter scales marked
//! "assumed" await calibration against render probes (docs/CALIBRATION.md).

use super::{reverb_model, FxContext, FxParams};
use crate::filters::{self, cutoff_hz};
use crate::params::index;
use lumen_dsp::biquad::{Biquad, Crossover, Shape};
use lumen_dsp::convolver::Convolver;
use lumen_dsp::delay::DelayLine;
use lumen_dsp::filter::{Filter, FilterKind};
use lumen_dsp::hilbert::FrequencyShifter;
use lumen_dsp::reverb::{Reverb, ReverbParams};
use lumen_dsp::sampler::SampleData;
use lumen_dsp::shaper::{self, Shaper};
use std::f32::consts::TAU;

pub trait Effect: Send {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext);
    fn reset(&mut self) {}
    /// True when the effect applies kParamWet itself (the rack then leaves it alone).
    fn owns_wet(&self) -> bool {
        false
    }
}

pub fn build(name: &str, ctx: &FxContext, impulse: Option<&SampleData>) -> Box<dyn Effect> {
    let sr = ctx.sample_rate;
    match name {
        "FXUtils" => Box::new(Utils::default()),
        "FXEQ" => Box::new(Eq::default()),
        "FXFilter" => Box::new(FxFilter::default()),
        "FXDistortion" => Box::new(Distortion::default()),
        "FXDelay" => Box::new(Delay::new(sr)),
        "FXChorus" => Box::new(Chorus::new(sr)),
        "FXFlanger" => Box::new(Flanger::new(sr)),
        "FXPhaser" => Box::new(Phaser::default()),
        "FXComp" => Box::new(Comp::new(sr)),
        "FXReverb" => Box::new(ReverbFx { reverb: Reverb::new(sr), controls: None }),
        "FXHyperD" => Box::new(Hyper::new(sr)),
        "FXBode" => Box::new(Bode::new(sr)),
        "FXConv" => Box::new(Conv::new(impulse, sr)),
        // Splits are handled by the rack; the effect itself passes audio through.
        _ => Box::new(Passthrough),
    }
}

struct Passthrough;

impl Effect for Passthrough {
    fn process(&mut self, _: &FxParams, _: &mut [f32], _: &mut [f32], _: &FxContext) {}
}

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// Seconds per synced unit: values are fractions of a 4/4 bar (assumed).
fn bar_seconds(bpm: f32) -> f32 {
    240.0 / bpm.max(1.0)
}

// ---------------------------------------------------------------- Utility

#[derive(Default)]
struct Utils {
    hp: [Biquad; 2],
    lp: [Biquad; 2],
    lf: [lumen_dsp::biquad::Crossover; 2],
}

impl Effect for Utils {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const WIDTH: usize = index("FXUtils", "kParamWidth");
        const BALANCE: usize = index("FXUtils", "kParamBalance");
        const LF_MONO: usize = index("FXUtils", "kParamLFMono");
        const LF_XOVER: usize = index("FXUtils", "kParamLFXover");
        const HPF: usize = index("FXUtils", "kParamHPF");
        const LPF: usize = index("FXUtils", "kParamLPF");
        const POL_L: usize = index("FXUtils", "kParamPolarityL");
        const POL_R: usize = index("FXUtils", "kParamPolarityR");
        let sr = ctx.sample_rate;
        let width = p.get(WIDTH) / 100.0;
        let bal = (p.get(BALANCE) / 100.0).clamp(-1.0, 1.0);
        let (gl, gr) = ((1.0 - bal).min(1.0), (1.0 + bal).min(1.0));
        let hpf = p.get(HPF);
        let lpf = p.get(LPF);
        let lf_mono = p.flag(LF_MONO);
        for ch in 0..2 {
            if hpf > 1.0 {
                self.hp[ch].set(Shape::Highpass, hpf, 0.707, sr);
            }
            if lpf < 19_999.0 {
                self.lp[ch].set(Shape::Lowpass, lpf, 0.707, sr);
            }
            if lf_mono {
                self.lf[ch].set(p.get(LF_XOVER), sr);
            }
        }
        let (pl, pr) = (if p.flag(POL_L) { -1.0 } else { 1.0 }, if p.flag(POL_R) { -1.0 } else { 1.0 });
        for i in 0..l.len() {
            let (mut a, mut b) = (l[i], r[i]);
            if lf_mono {
                let (la, ha) = self.lf[0].split(a);
                let (lb, hb) = self.lf[1].split(b);
                let mono = 0.5 * (la + lb);
                a = mono + ha;
                b = mono + hb;
            }
            let mid = 0.5 * (a + b);
            let side = 0.5 * (a - b) * width;
            a = mid + side;
            b = mid - side;
            if hpf > 1.0 {
                a = self.hp[0].tick(a);
                b = self.hp[1].tick(b);
            }
            if lpf < 19_999.0 {
                a = self.lp[0].tick(a);
                b = self.lp[1].tick(b);
            }
            l[i] = a * gl * pl;
            r[i] = b * gr * pr;
        }
    }
}

// ---------------------------------------------------------------- EQ

#[derive(Default)]
struct Eq {
    bands: [[Biquad; 2]; 2],
}

impl Effect for Eq {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        let params = [
            (index("FXEQ", "kParamFreq1"), index("FXEQ", "kParamGain1"), index("FXEQ", "kParamReso1"), index("FXEQ", "kParamType1")),
            (index("FXEQ", "kParamFreq2"), index("FXEQ", "kParamGain2"), index("FXEQ", "kParamReso2"), index("FXEQ", "kParamType2")),
        ];
        for (b, (f, g, q, t)) in params.into_iter().enumerate() {
            // Types 0 shelf, 1 peak, 2 cut (high-pass on band 1, low-pass on band 2); each is
            // an RBJ biquad (fxr probes, within 0.03 dB) with resonance r mapped to Q as
            // 0.1 + 18.8·r⁴ for peaks and cuts and 0.4 + 1.74·r³ for shelves.
            let r = (p.get(q) / 100.0).clamp(0.0, 1.0);
            let shelf = p.get(t).round() as i32 == 0;
            let q = if shelf { 0.4 + 1.74 * r * r * r } else { 0.1 + 18.8 * r.powi(4) };
            let gain_db = p.get(g);
            let shape = match (p.get(t).round() as i32, b) {
                (1, _) => Shape::Peak { gain_db },
                (2, 0) => Shape::Highpass,
                (2, _) => Shape::Lowpass,
                (_, 0) => Shape::LowShelf { gain_db },
                _ => Shape::HighShelf { gain_db },
            };
            for ch in 0..2 {
                self.bands[b][ch].set(shape, p.get(f), q, ctx.sample_rate);
            }
        }
        let [low, high] = &mut self.bands;
        for i in 0..l.len() {
            l[i] = high[0].tick(low[0].tick(l[i]));
            r[i] = high[1].tick(low[1].tick(r[i]));
        }
    }

    fn reset(&mut self) {
        self.bands.iter_mut().flatten().for_each(Biquad::reset);
    }
}

// ---------------------------------------------------------------- Filter

struct FxFilter {
    filter: Filter,
}

impl Default for FxFilter {
    fn default() -> Self {
        FxFilter { filter: Filter::new(FilterKind::Lowpass { poles: 2 }) }
    }
}

impl Effect for FxFilter {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const TYPE: usize = index("FXFilter", "kParamType");
        const FREQ: usize = index("FXFilter", "kParamFreq");
        const RESO: usize = index("FXFilter", "kParamReso");
        const DRIVE: usize = index("FXFilter", "kParamDrive");
        const STEREO: usize = index("FXFilter", "kParamStereo");
        const VAR: usize = index("FXFilter", "kParamVar");
        let Some(kind) = filters::model(p.choice(TYPE)).0 else { return };
        if self.filter.kind() != kind {
            self.filter.set_kind(kind);
        }
        // Stereo spread as measured on the voice filter (±2.65 octaves at 0/100), assumed here.
        self.filter.set_spread((50.0 - p.get(STEREO)) / 50.0 * 2.65);
        self.filter.set_var(p.get(VAR) / 100.0);
        // Oversampled like the voice filters' default (fxr ff probes: the top octave
        // matches at 4×, not at 1×).
        self.filter.set_oversampling(2);
        self.filter.set(cutoff_hz(p.get(FREQ)), p.get(RESO) / 100.0, p.get(DRIVE) / 100.0, ctx.sample_rate);
        for i in 0..l.len() {
            (l[i], r[i]) = self.filter.process(l[i], r[i]);
        }
    }

    fn reset(&mut self) {
        self.filter.reset();
    }
}

// ---------------------------------------------------------------- Distortion

#[derive(Default)]
struct Distortion {
    /// Pre/post filter per channel: low-pass, band-pass and high-pass sharing poles.
    pre: [[Biquad; 3]; 2],
    /// Weights of the three outputs for the LP/HP morph.
    mix: [f32; 3],
    held: [f32; 2],
    hold_count: f32,
}

/// The distortion filter's resonance Q against its "BW" control, fitted on low-,
/// band- and high-pass responses (fxr dflt probes); beyond the ends it is extrapolated.
fn distortion_filter_q(bw: f32) -> f32 {
    const BW: [f32; 4] = [0.5, 1.95, 4.0, 7.0];
    const Q: [f32; 4] = [1.2, 2.05, 3.9, 20.7];
    let i = BW.partition_point(|&b| b <= bw).clamp(1, BW.len() - 1);
    let t = (bw - BW[i - 1]) / (BW[i] - BW[i - 1]);
    (Q[i - 1] + (Q[i] - Q[i - 1]) * t).clamp(0.7, 40.0)
}

/// A measured static curve (`dist_curves`) at one drive, interpolated between the
/// two nearest measured drives.
struct MeasuredCurve {
    mode: usize,
    drive: usize,
    frac: f32,
}

impl MeasuredCurve {
    fn new(name: &str, drive: f32) -> Option<MeasuredCurve> {
        use super::dist_curves::{DRIVES, MODES};
        let mode = MODES.iter().position(|&m| m == name)?;
        let d = drive.clamp(0.0, 100.0);
        let i = DRIVES.partition_point(|&x| x <= d).clamp(1, DRIVES.len() - 1) - 1;
        let frac = ((d - DRIVES[i]) / (DRIVES[i + 1] - DRIVES[i])).clamp(0.0, 1.0);
        Some(MeasuredCurve { mode, drive: i, frac })
    }

    #[inline]
    fn apply(&self, x: f32) -> f32 {
        use super::dist_curves::{CURVES, POINTS, V0, W_MAX};
        // Beyond the measured range (±10.5) the curve holds its end value.
        let w = (x / V0).asinh().clamp(-W_MAX, W_MAX);
        let pos = (w + W_MAX) / (2.0 * W_MAX) * (POINTS - 1) as f32;
        let j = (pos as usize).min(POINTS - 2);
        let t = pos - j as f32;
        let at = |c: &[i16; POINTS]| c[j] as f32 + (c[j + 1] as f32 - c[j] as f32) * t;
        let table = &CURVES[self.mode];
        let (a, b) = (at(&table[self.drive]), at(&table[self.drive + 1]));
        (a + (b - a) * self.frac) / 8192.0
    }
}

impl Distortion {
    #[inline]
    fn filter(&mut self, ch: usize, x: f32) -> f32 {
        let [lp, bp, hp] = &mut self.pre[ch];
        lp.tick(x) * self.mix[0] + bp.tick(x) * self.mix[1] + hp.tick(x) * self.mix[2]
    }
}

impl Effect for Distortion {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const DRIVE: usize = index("FXDistortion", "kParamDrive");
        const MODE: usize = index("FXDistortion", "kParamMode");
        const LPHP: usize = index("FXDistortion", "kParamLPHP");
        const FREQ: usize = index("FXDistortion", "kParamFreq");
        const BW: usize = index("FXDistortion", "kParamBW");
        const PREPOST: usize = index("FXDistortion", "kParamPrePost");
        let drive = p.get(DRIVE) / 100.0;
        let shaper = Shaper::from_name(p.choice(MODE)).unwrap_or(Shaper::Tube);
        // Drive 0–100 % → 0–36 dB of input gain (assumed); −11 dB output trim (measured:
        // Lumen was 10–12 dB louder than Serum across default, drive 80 and soft clip).
        let gain = db(36.0 * drive);
        const TRIM: f32 = 0.282;
        let prepost = p.get(PREPOST).round() as i32;
        if prepost != 0 {
            // A resonant two-pole filter (BW is its resonance) on the voice-filter cutoff
            // scale, morphing linearly low-pass → band-pass (unnormalised) → inverted
            // high-pass as LPHP goes 0 → 50 → 100, 0.9 dB down (fxr dflt probes, ≈ 0.8 dB).
            let m = (p.get(LPHP) / 100.0).clamp(0.0, 1.0);
            let q = distortion_filter_q(p.get(BW));
            const LEVEL: f32 = 0.9;
            self.mix = if m <= 0.5 { [1.0 - 2.0 * m, 2.0 * m * q, 0.0] } else { [0.0, (2.0 - 2.0 * m) * q, 1.0 - 2.0 * m] }.map(|w| w * LEVEL);
            let hz = cutoff_hz(p.get(FREQ));
            for ch in &mut self.pre {
                for (b, shape) in ch.iter_mut().zip([Shape::Lowpass, Shape::Bandpass, Shape::Highpass]) {
                    b.set(shape, hz, q, ctx.sample_rate);
                }
            }
        }
        // Downsample holds each sample for 1 + 96·drive² samples (7, 25 and 97 measured
        // at drive 25, 50 and 100, at 48 kHz).
        let hold = (1.0 + 96.0 * drive * drive).round();
        // Most modes are measured static curves (fxd4 probes); the rest keep the generic shapers.
        let measured = MeasuredCurve::new(p.choice(MODE), p.get(DRIVE));
        for i in 0..l.len() {
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let mut x = *s;
                if prepost == 1 {
                    x = self.filter(ch, x);
                }
                x = if shaper == Shaper::Downsample {
                    if ch == 0 {
                        self.hold_count += 1.0;
                    }
                    if self.hold_count >= hold || self.hold_count < 1.0 {
                        self.held[ch] = x;
                        if ch == 1 {
                            self.hold_count = 0.0;
                        }
                    }
                    self.held[ch]
                } else if let Some(c) = &measured {
                    c.apply(x)
                } else {
                    shaper::shape(shaper, x * gain) * TRIM
                };
                if prepost == 2 {
                    x = self.filter(ch, x);
                }
                *s = x;
            }
        }
    }

    fn reset(&mut self) {
        self.pre.iter_mut().flatten().for_each(Biquad::reset);
    }
}

// ---------------------------------------------------------------- Delay

struct Delay {
    lines: [DelayLine; 2],
    filters: [Biquad; 2],
    /// Samples since the start, for the reverse mode's chunk position.
    clock: u64,
}

/// Reverse delay read for a chunk length `t` (samples): each chunk plays the one two
/// chunks back in reverse, so output starts after 2t, at twice the input's level with
/// no beating on a steady tone (mode 2 probes). A single head fades over 5 ms at the
/// chunk edges, which is why a click right at an edge comes out silent.
fn reverse_read(line: &DelayLine, clock: u64, t: f32, sample_rate: f32) -> f32 {
    let t = t.max(2.0);
    let u = (clock as f64 % t as f64) as f32;
    let fade = (0.005 * sample_rate).min(t * 0.25).max(1.0);
    let w = (u / fade).min((t - u) / fade).min(1.0);
    2.0 * w * line.read(t + 2.0 * u)
}

/// Synced delay time in bars: the time control steps through straight divisions
/// from 1/512 bar to 4 bars (render sweeps; each threshold is the midpoint of a
/// 0.0005 bracket, and 1/256's, never caught between 0.001 and 0.0015, is assumed).
fn synced_delay_bars(v: f32) -> f32 {
    const FROM: [f32; 11] = [0.00125, 0.00125, 0.00225, 0.00625, 0.01525, 0.03225, 0.06175, 0.10925, 0.17925, 0.27925, 0.41625];
    const BARS: [f32; 12] = [1.0 / 512.0, 1.0 / 256.0, 1.0 / 128.0, 1.0 / 64.0, 1.0 / 32.0, 1.0 / 16.0, 1.0 / 8.0, 0.25, 0.5, 1.0, 2.0, 4.0];
    BARS[FROM.partition_point(|&t| t <= v)]
}

impl Delay {
    fn new(sr: f32) -> Self {
        let n = (sr * 8.0) as usize;
        Delay { lines: [DelayLine::new(n), DelayLine::new(n)], filters: Default::default(), clock: 0 }
    }
}

impl Effect for Delay {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const SYNC: usize = index("FXDelay", "kParamBeatSync");
        const LINK: usize = index("FXDelay", "kParamLink");
        const TIME_L: usize = index("FXDelay", "kParamTimeL");
        const TIME_R: usize = index("FXDelay", "kParamTimeR");
        const OFFSET_L: usize = index("FXDelay", "kParamOffsetL");
        const OFFSET_R: usize = index("FXDelay", "kParamOffsetR");
        const MODE: usize = index("FXDelay", "kParamMode");
        const FEEDBACK: usize = index("FXDelay", "kParamFeedback");
        const FREQ: usize = index("FXDelay", "kParamFreq");
        const BW: usize = index("FXDelay", "kParamBW");
        let sr = ctx.sample_rate;
        // Delay probes (a click at 100 % wet): synced times snap to straight divisions,
        // free times are the value in seconds less 1 ms, link makes both sides follow
        // the right time, and each repeat is the feedback times the last.
        let time = |v: f32| if p.flag(SYNC) { synced_delay_bars(v) * bar_seconds(ctx.bpm) } else { (v - 0.001).max(0.0) };
        let (vl, vr) = if p.flag(LINK) { (p.get(TIME_R), p.get(TIME_R)) } else { (p.get(TIME_L), p.get(TIME_R)) };
        let (dl, dr) = ((time(vl) * p.get(OFFSET_L) * sr).max(1.0), (time(vr) * p.get(OFFSET_R) * sr).max(1.0));
        let fb = (p.get(FEEDBACK) / 100.0).clamp(0.0, 0.98);
        let mode = p.get(MODE).round() as i32;
        let ping_pong = mode == 1;
        // The reverse mode's ×2 is on its output only: its repeats still fall by the
        // feedback per chunk (mode 2 probes).
        let fb = if mode == 2 { fb * 0.5 } else { fb };
        // A two-pole band-pass (0 dB peak) at Freq whose width tracks BW: 1/Q ≈ 2^(BW − 1.2)
        // (fxr dly probes, within ≈ 1 dB).
        let q = (1.2 - p.get(BW)).exp2().max(0.005);
        for f in &mut self.filters {
            f.set(Shape::Bandpass, p.get(FREQ), q, sr);
        }
        for i in 0..l.len() {
            let (yl, yr) = if mode == 2 {
                (reverse_read(&self.lines[0], self.clock, dl, sr), reverse_read(&self.lines[1], self.clock, dr, sr))
            } else {
                (self.lines[0].read(dl), self.lines[1].read(dr))
            };
            self.clock += 1;
            let [fl, fr] = &mut self.filters;
            let (fl, fr) = (fl.tick(yl), fr.tick(yr));
            if ping_pong {
                // The input repeats left, then right at the same level, then left again
                // scaled by the feedback.
                self.lines[0].push(0.5 * (l[i] + r[i]) + fr * fb);
                self.lines[1].push(fl);
            } else {
                self.lines[0].push(l[i] + fl * fb);
                self.lines[1].push(r[i] + fr * fb);
            }
            l[i] = fl;
            r[i] = fr;
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.clock = 0;
    }
}

// ---------------------------------------------------------------- Chorus / Flanger / Phaser

struct Chorus {
    lines: [DelayLine; 2],
    phase: f32,
    filt: [Biquad; 2],
    fb: [f32; 2],
}

impl Chorus {
    fn new(sr: f32) -> Self {
        Chorus { lines: [DelayLine::new((sr * 0.06) as usize), DelayLine::new((sr * 0.06) as usize)], phase: 0.0, filt: Default::default(), fb: [0.0; 2] }
    }
}

impl Effect for Chorus {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const RATE: usize = index("FXChorus", "kParamRate");
        const DELAY: usize = index("FXChorus", "kParamDelay");
        const DELAY2: usize = index("FXChorus", "kParamDelay2");
        const DEPTH: usize = index("FXChorus", "kParamDepth");
        const FEEDBACK: usize = index("FXChorus", "kParamFeedback");
        const FILT: usize = index("FXChorus", "kParamFilt");
        const FILT_MODE: usize = index("FXChorus", "kParamFiltMode");
        let sr = ctx.sample_rate;
        let base = [p.get(DELAY), if p.get(DELAY2) > 0.0 { p.get(DELAY2) } else { p.get(DELAY) }];
        // Depth (ms of sweep) assumed; rate in Hz.
        let depth = p.get(DEPTH) / 100.0 * 4.0;
        let fb = (p.get(FEEDBACK) / 100.0).clamp(0.0, 0.95);
        let shape = if p.flag(FILT_MODE) { Shape::Highpass } else { Shape::Lowpass };
        for f in &mut self.filt {
            f.set(shape, p.get(FILT), 0.707, sr);
        }
        let inc = p.get(RATE) / sr;
        for i in 0..l.len() {
            self.phase = (self.phase + inc).fract();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let lfo = (TAU * (self.phase + ch as f32 * 0.25)).sin();
                let d = ((base[ch] + depth * (0.5 + 0.5 * lfo)) * 0.001 * sr).max(2.0);
                let y = self.lines[ch].read_cubic(d);
                self.lines[ch].push(*s + self.fb[ch] * fb);
                self.fb[ch] = y;
                *s = self.filt[ch].tick(y);
            }
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.fb = [0.0; 2];
    }
}

struct Flanger {
    lines: [DelayLine; 2],
    phase: f32,
}

impl Flanger {
    fn new(sr: f32) -> Self {
        Flanger { lines: [DelayLine::new((sr * 0.02) as usize), DelayLine::new((sr * 0.02) as usize)], phase: 0.0 }
    }
}

impl Effect for Flanger {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const RATE: usize = index("FXFlanger", "kParamRate");
        const DEPTH: usize = index("FXFlanger", "kParamDepth");
        const FEEDBACK: usize = index("FXFlanger", "kParamFeedback");
        const WIDTH: usize = index("FXFlanger", "kParamWidth");
        let sr = ctx.sample_rate;
        let depth = p.get(DEPTH) / 100.0;
        let fb = (p.get(FEEDBACK) / 100.0 * 0.95).clamp(-0.95, 0.95);
        let offset = p.get(WIDTH) / 360.0;
        let inc = p.get(RATE) / sr;
        for i in 0..l.len() {
            self.phase = (self.phase + inc).fract();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let lfo = 0.5 + 0.5 * (TAU * (self.phase + ch as f32 * offset)).sin();
                // 0.3 ms to 0.3 + 6 ms sweep (assumed).
                let d = ((0.3 + 6.0 * depth * lfo) * 0.001 * sr).max(2.0);
                let y = self.lines[ch].read_cubic(d);
                self.lines[ch].push(*s + y * fb);
                *s = y;
            }
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().for_each(DelayLine::clear);
    }
}

#[derive(Default)]
struct Phaser {
    state: [[f32; 18]; 2],
    phase: f32,
    fb: [f32; 2],
}

impl Effect for Phaser {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const RATE: usize = index("FXPhaser", "kParamRate");
        const DEPTH: usize = index("FXPhaser", "kParamDepth");
        const FREQ: usize = index("FXPhaser", "kParamFreq");
        const FEEDBACK: usize = index("FXPhaser", "kParamFeedback");
        const WIDTH: usize = index("FXPhaser", "kParamWidth");
        const POLES: usize = index("FXPhaser", "kParamNumPoles");
        let sr = ctx.sample_rate;
        let poles = (p.get(POLES).round() as usize).clamp(1, 18);
        let centre = p.get(FREQ);
        // Depth sweeps up to ±2 octaves around the centre (assumed).
        let range = p.get(DEPTH) / 100.0 * 2.0;
        let fb = (p.get(FEEDBACK) / 100.0 * 0.9).clamp(0.0, 0.9);
        let offset = p.get(WIDTH) / 360.0;
        let inc = p.get(RATE) / sr;
        for i in 0..l.len() {
            self.phase = (self.phase + inc).fract();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let lfo = (TAU * (self.phase + ch as f32 * offset)).sin();
                let hz = (centre * (range * lfo).exp2()).clamp(20.0, sr * 0.45);
                let t = (std::f32::consts::PI * hz / sr).tan();
                let a = (t - 1.0) / (t + 1.0);
                let mut x = *s + self.fb[ch] * fb;
                for z in self.state[ch].iter_mut().take(poles) {
                    // First-order allpass: y = a·x + z; z = x − a·y.
                    let y = a * x + *z;
                    *z = x - a * y;
                    x = y;
                }
                self.fb[ch] = x;
                *s = 0.5 * (*s + x);
            }
        }
    }

    fn reset(&mut self) {
        self.state = [[0.0; 18]; 2];
        self.fb = [0.0; 2];
    }
}

// ---------------------------------------------------------------- Compressor

/// Compressor (measured with sines and squares at stepped levels and level steps;
/// docs/CALIBRATION.md). Single-band: ratios below 100 compress from a smoothed
/// overshoot; higher ratios switch to a lookahead brickwall limiter. Multiband: three
/// LR4 bands, each with downward and upward compression. Both paths end in a hard clip
/// and a 12 Hz DC blocker, and the module mixes its own wet signal (linear crossfade
/// against the dry path: delayed by the limiter's lookahead, or the band sum).
struct Comp {
    /// Smoothed overshoot above the threshold (dB).
    over: f32,
    lookahead: [DelayLine; 2],
    /// Lookahead window maximum: (sample index, level), levels decreasing.
    peaks: std::collections::VecDeque<(u64, f32)>,
    clock: u64,
    dc: [(f32, f32); 2],
    /// Multiband split per channel: low crossover, the allpass that keeps the low
    /// band in phase with the upper crossover, upper crossover.
    split: [[Crossover; 3]; 2],
    /// Multiband detector per band (stereo-linked): smoothed |x|, then its follower.
    level: [f32; 3],
    env: [f32; 3],
}

/// Multiband detector coefficients for one block.
struct MbTimes {
    level: f32,
    att: f32,
    rel: f32,
}

/// Per-band multiband settings for one block.
struct MbBand {
    /// Downward threshold (dB, on the detector level) and slope.
    td: f32,
    kd: f32,
    /// Upward threshold and slope (negative: expansion below the threshold).
    tu: f32,
    ku: f32,
    gain: f32,
}

/// Multiband detector: |x| smoothed with a 0.43 ms one-pole, then a follower with a
/// fixed 1.5 ms attack (kParamAttack has no effect) and 0.97 × release, at least 10 ms.
const MB_LEVEL_MS: f32 = 0.43;
const MB_ATTACK_MS: f32 = 1.52;
const MB_RELEASE_SCALE: f32 = 0.969;
const MB_MIN_RELEASE_MS: f32 = 10.0;
/// Multiband thresholds at kParamThresh 0.5 on the detector level; low/mid/high differ.
const MB_THRESH_DB: [f32; 3] = [-23.35, -22.48, -30.01];
/// Mid-band threshold shift (dB) from kParamThresh 0.5, measured at these values; the
/// low and high bands move about 1.1× and 1.2× as far.
const MB_THRESH_SHIFT: [(f32, f32); 9] = [
    (0.25, 22.68),
    (0.3, 19.35),
    (0.4, 10.88),
    (0.45, 5.74),
    (0.5, 0.0),
    (0.55, -6.38),
    (0.6, -13.37),
    (0.75, -37.82),
    (0.9, -67.5),
];
const MB_THRESH_BAND_SCALE: [f32; 3] = [1.1, 1.0, 1.2];
/// dB per kParamThreshUD unit away from 100.
const MB_THRESH_UD_DB: f32 = 0.24;
const MB_MAX_DOWN_DB: f32 = 40.0;
const MB_MAX_UP_DB: f32 = 35.2;
/// Upward thresholds sit Deadband below the downward ones, plus these per band.
const MB_UP_OFFSET_DB: [f32; 3] = [-1.08, 0.17, 0.89];

/// Mid-band threshold shift for kParamThresh `t`: the measured table, interpolated in
/// t and extended past both ends along the end segment in log10(1 − t).
fn mb_thresh_shift(t: f32) -> f32 {
    let n = MB_THRESH_SHIFT.len();
    let i = MB_THRESH_SHIFT.iter().position(|&(at, _)| at > t).unwrap_or(n).clamp(1, n - 1);
    let ((t0, s0), (t1, s1)) = (MB_THRESH_SHIFT[i - 1], MB_THRESH_SHIFT[i]);
    if (t0..=t1).contains(&t) {
        return s0 + (s1 - s0) * (t - t0) / (t1 - t0);
    }
    let x = |t: f32| (1.0 - t).max(1e-6).log10();
    let (ta, sa) = if t < t0 { (t0, s0) } else { (t1, s1) };
    sa + (s1 - s0) / (x(t1) - x(t0)) * (x(t) - x(ta))
}

/// Threshold at kParamThresh 0 (dB inside the rack); it falls as 60·log10(1 − t),
/// floored 120 dB down.
const COMP_THRESH_TOP_DB: f32 = 10.51;
/// Ratios at or above this act as a limiter (50 still compresses; 100 limits).
const COMP_LIMIT_RATIO: f32 = 100.0;
const COMP_CLIP: f32 = 3.3126;
const COMP_DC_HZ: f32 = 11.985;
const COMP_MAX_LOOKAHEAD_S: f32 = 0.0065;

impl Comp {
    fn new(sr: f32) -> Self {
        let n = (COMP_MAX_LOOKAHEAD_S * sr).ceil() as usize + 2;
        Comp {
            over: 0.0,
            lookahead: [DelayLine::new(n), DelayLine::new(n)],
            peaks: std::collections::VecDeque::with_capacity(n + 1),
            clock: 0,
            dc: [(0.0, 0.0); 2],
            split: Default::default(),
            level: [0.0; 3],
            env: [0.0; 3],
        }
    }

    /// Output clip and DC blocker, shared by both paths.
    fn output(&mut self, ch: usize, y: f32, dc: f32) -> f32 {
        let y = y.clamp(-COMP_CLIP, COMP_CLIP);
        let (x1, y1) = self.dc[ch];
        let hp = 0.5 * (1.0 + dc) * (y - x1) + dc * y1;
        self.dc[ch] = (y, hp);
        hp
    }

    /// Splits one stereo sample into three bands, compresses each and sums them.
    /// Returns (compressed, dry band sum): wet/dry mixes against the band sum, which
    /// carries the crossovers' phase.
    fn multiband(&mut self, bands: &[MbBand; 3], times: &MbTimes, x: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let mut split = [[0.0; 3]; 2];
        for (ch, s) in split.iter_mut().enumerate() {
            let [low_xo, allpass, high_xo] = &mut self.split[ch];
            let (low, rest) = low_xo.split(x[ch]);
            let (a, b) = allpass.split(low);
            let (mid, high) = high_xo.split(rest);
            *s = [a + b, mid, high];
        }
        let dry = split.map(|s| s.iter().sum());
        let mut y = [0.0; 2];
        for (b, band) in bands.iter().enumerate() {
            let rect = split[0][b].abs().max(split[1][b].abs());
            self.level[b] = rect + times.level * (self.level[b] - rect);
            let level = self.level[b];
            let k = if level > self.env[b] { times.att } else { times.rel };
            self.env[b] = level + k * (self.env[b] - level);
            let e = 20.0 * self.env[b].max(1e-12).log10();
            let down = (band.kd * (e - band.td)).clamp(0.0, MB_MAX_DOWN_DB);
            let up = (band.ku * (band.tu - e).max(0.0)).clamp(-MB_MAX_UP_DB, MB_MAX_UP_DB);
            let g = db(up - down) * band.gain;
            y[0] += split[0][b] * g;
            y[1] += split[1][b] * g;
        }
        (y, dry)
    }
}

impl Effect for Comp {
    fn owns_wet(&self) -> bool {
        true
    }

    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const WET: usize = index("FXComp", "kParamWet");
        const THRESH: usize = index("FXComp", "kParamThresh");
        const RATIO: usize = index("FXComp", "kParamRatio");
        const ATTACK: usize = index("FXComp", "kParamAttack");
        const RELEASE: usize = index("FXComp", "kParamRelease");
        const MAKEUP: usize = index("FXComp", "kParamMakeup");
        const MULTIBAND: usize = index("FXComp", "kParamMultiband");
        const XOVER_LOW: usize = index("FXComp", "kParamXoverLow");
        const XOVER_HIGH: usize = index("FXComp", "kParamXoverHi");
        const RATIO_BELOW: usize = index("FXComp", "kParamRatioBelow");
        const THRESH_UD: [usize; 3] =
            [index("FXComp", "kParamThreshUD0"), index("FXComp", "kParamThreshUD1"), index("FXComp", "kParamThreshUD2")];
        const RATIO_B: [usize; 3] =
            [index("FXComp", "kParamRatio0"), index("FXComp", "kParamRatio1"), index("FXComp", "kParamRatio2")];
        const RATIO_BELOW_B: [usize; 3] = [
            index("FXComp", "kParamRatioBelow0"),
            index("FXComp", "kParamRatioBelow1"),
            index("FXComp", "kParamRatioBelow2"),
        ];
        const DEADBAND: [usize; 3] =
            [index("FXComp", "kParamDeadband0"), index("FXComp", "kParamDeadband1"), index("FXComp", "kParamDeadband2")];
        const GAIN: [usize; 3] = [index("FXComp", "kParamGain0"), index("FXComp", "kParamGain1"), index("FXComp", "kParamGain2")];
        let sr = ctx.sample_rate;
        let wet = (p.get(WET) / 100.0).clamp(0.0, 1.0);
        let t = p.get(THRESH).clamp(0.0, 1.0);
        let thresh_db = COMP_THRESH_TOP_DB + 20.0 * (1.0 - t).powi(3).max(1e-6).log10();
        let ratio = p.get(RATIO).max(1.0);
        let multiband = p.get(MULTIBAND) >= 0.5;
        let limiter = !multiband && ratio >= COMP_LIMIT_RATIO;
        let slope = 1.0 - 1.0 / ratio;
        let attack_ms = p.get(ATTACK).max(0.1);
        let release_ms = p.get(RELEASE).max(0.1);
        // Attack reaches 90 % of a step in the set time; release is one time constant
        // (0.96×; the limiter releases about three times faster).
        let att = (-std::f32::consts::LN_10 / (attack_ms * 0.001 * sr)).exp();
        let rel = (-1.0 / ((if limiter { 0.35 } else { 0.963 }) * release_ms * 0.001 * sr)).exp();
        // Makeup is a linear gain: values up to 1 leave the level, then 1.033 per unit.
        let makeup = 1.0 + 1.033 * (p.get(MAKEUP) - 1.0).max(0.0);
        // The limiter looks ahead 1 ms + attack/200 (output and dry delayed alike).
        let ahead = if limiter { ((0.001 + attack_ms * 0.000005) * sr).round() as usize } else { 0 };
        let ahead = ahead.min(self.lookahead[0].capacity() - 1);
        let dc = (-TAU * COMP_DC_HZ / sr).exp();
        let bands = multiband.then(|| {
            let (low_hz, high_hz) = (p.get(XOVER_LOW), p.get(XOVER_HIGH));
            for [low_xo, allpass, high_xo] in &mut self.split {
                low_xo.set(low_hz, sr);
                allpass.set(high_hz, sr);
                high_xo.set(high_hz, sr);
            }
            let coef = |ms: f32| (-1.0 / (ms * 0.001 * sr)).exp();
            let times = MbTimes {
                level: coef(MB_LEVEL_MS),
                att: coef(MB_ATTACK_MS),
                rel: coef(MB_RELEASE_SCALE * release_ms.max(MB_MIN_RELEASE_MS)),
            };
            let shift = mb_thresh_shift(t);
            let bands: [MbBand; 3] = std::array::from_fn(|b| {
                let td = MB_THRESH_DB[b] + MB_THRESH_BAND_SCALE[b] * shift + MB_THRESH_UD_DB * (p.get(THRESH_UD[b]) - 100.0);
                MbBand {
                    td,
                    kd: slope * p.get(RATIO_B[b]).max(0.0),
                    tu: td - p.get(DEADBAND[b]) + MB_UP_OFFSET_DB[b],
                    // Band RatioBelow 0.5 is flat; below it expands.
                    ku: p.get(RATIO_BELOW) * (2.0 * p.get(RATIO_BELOW_B[b]) - 1.0),
                    gain: db(p.get(GAIN[b])),
                }
            });
            (bands, times)
        });
        for i in 0..l.len() {
            let (x_l, x_r) = (l[i], r[i]);
            if let Some((bands, times)) = &bands {
                let (y, dry) = self.multiband(bands, times, [x_l, x_r]);
                for (ch, out) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                    *out = dry[ch] + wet * (self.output(ch, y[ch] * makeup, dc) - dry[ch]);
                }
                continue;
            }
            self.lookahead[0].push(x_l);
            self.lookahead[1].push(x_r);
            let level = x_l.abs().max(x_r.abs());
            let over_now = if limiter {
                while self.peaks.back().is_some_and(|&(_, v)| v <= level) {
                    self.peaks.pop_back();
                }
                self.peaks.push_back((self.clock, level));
                while self.peaks.front().is_some_and(|&(at, _)| at + (ahead as u64) < self.clock) {
                    self.peaks.pop_front();
                }
                let peak = self.peaks.front().map_or(0.0, |&(_, v)| v);
                (20.0 * peak.max(1e-9).log10() - thresh_db).max(0.0)
            } else {
                (20.0 * level.max(1e-9).log10() - thresh_db).max(0.0)
            };
            self.clock += 1;
            if over_now > self.over {
                self.over = if limiter { over_now } else { over_now + att * (self.over - over_now) };
            } else {
                self.over = over_now + rel * (self.over - over_now);
            }
            let reduction = if limiter { self.over } else { slope * self.over };
            let gain = db(-reduction.min(200.0)) * makeup;
            let dry = [self.lookahead[0].read(ahead as f32), self.lookahead[1].read(ahead as f32)];
            for (ch, out) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                *out = dry[ch] + wet * (self.output(ch, dry[ch] * gain, dc) - dry[ch]);
            }
        }
    }

    fn reset(&mut self) {
        self.over = 0.0;
        self.lookahead.iter_mut().for_each(DelayLine::clear);
        self.peaks.clear();
        self.dc = [(0.0, 0.0); 2];
        self.split = Default::default();
        self.level = [0.0; 3];
        self.env = [0.0; 3];
    }
}

// ---------------------------------------------------------------- Reverb

/// Serum's reverb types as measured metrics (`reverb_model`) driving the FDN.
struct ReverbFx {
    reverb: Reverb,
    /// Control indices for the type last seen.
    controls: Option<(&'static str, reverb_model::Indices)>,
}

/// Lumen's FDN level against a dry click (dB), fitted over the fxrv6- to fxrv10-
/// probes: a constant plus terms in ln T60 (500 Hz) and ln of the 125 Hz decay ratio
/// (its normalisation follows the mid band, while a click's energy is mostly low). The
/// measured level is applied relative to it.
const REVERB_LEVEL_DB: f32 = 5.63;
const REVERB_LEVEL_PER_LN_T60: f32 = -0.24;
const REVERB_LEVEL_PER_LN_LOW: f32 = 2.54;

impl Effect for ReverbFx {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], _ctx: &FxContext) {
        const SIZE: usize = index("FXReverb", "kParamSize");
        const TYPE: usize = index("FXReverb", "kParamType");
        let kind = p.choice(TYPE);
        let table = reverb_model::type_table(kind);
        if self.controls.as_ref().is_none_or(|(k, _)| *k != table.kind) {
            self.controls = Some((table.kind, reverb_model::indices(table)));
        }
        let m = reverb_model::evaluate(table, &self.controls.as_ref().unwrap().1, p);
        let modulation = match kind {
            "kAbyss" => 0.6,
            "kSpace" => 0.8,
            _ => 0.2,
        };
        let c = m.corr.clamp(-0.95, 1.0);
        let params = ReverbParams {
            size: p.get(SIZE) / 100.0,
            decay: m.t60,
            decay_low: m.low,
            decay_upper: m.upper,
            decay_high: m.high,
            predelay_s: m.onset_ms * 0.001,
            low_cut_hz: m.low_cut_hz.max(5.0),
            modulation,
            // Lumen's tail correlation is (1 − w²)/(1 + w²) for width w.
            width: ((1.0 - c) / (1.0 + c)).sqrt(),
        };
        self.reverb.process(&params, l, r);
        let fdn_db = REVERB_LEVEL_DB + REVERB_LEVEL_PER_LN_T60 * m.t60.max(0.02).ln() + REVERB_LEVEL_PER_LN_LOW * m.low.max(0.05).ln();
        let gain = db(m.level - fdn_db);
        for x in l.iter_mut().chain(r.iter_mut()) {
            *x *= gain;
        }
    }

    fn reset(&mut self) {
        self.reverb.reset();
    }
}

// ---------------------------------------------------------------- Hyper/Dimension

const HYPER_VOICES: usize = 7;

struct Hyper {
    lines: [DelayLine; 2],
    phases: [f32; HYPER_VOICES],
    dim: [DelayLine; 2],
}

impl Hyper {
    fn new(sr: f32) -> Self {
        let n = (sr * 0.05) as usize;
        Hyper { lines: [DelayLine::new(n), DelayLine::new(n)], phases: std::array::from_fn(|i| i as f32 / HYPER_VOICES as f32), dim: [DelayLine::new(n), DelayLine::new(n)] }
    }
}

impl Effect for Hyper {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const RATE: usize = index("FXHyperD", "kParamRate");
        const DETUNE: usize = index("FXHyperD", "kParamDetune");
        const UNISON: usize = index("FXHyperD", "kParamUnison");
        const DIM_SIZE: usize = index("FXHyperD", "kParamDimESize");
        const DIM_WET: usize = index("FXHyperD", "kParamDimEWet");
        let sr = ctx.sample_rate;
        let voices = (p.get(UNISON).round() as usize).clamp(1, HYPER_VOICES);
        // Detune → modulation depth (ms); rate 0–100 → 0.1–5 Hz (assumed).
        let depth = p.get(DETUNE) / 100.0 * 3.0;
        let rate = 0.1 * 50f32.powf(p.get(RATE) / 100.0);
        let dim_wet = p.get(DIM_WET) / 100.0;
        let dim_ms = 5.0 + p.get(DIM_SIZE) / 100.0 * 20.0;
        let norm = 1.0 / (voices as f32).sqrt();
        for i in 0..l.len() {
            self.lines[0].push(l[i]);
            self.lines[1].push(r[i]);
            let (mut wl, mut wr) = (0.0, 0.0);
            for v in 0..voices {
                let ph = &mut self.phases[v];
                *ph = (*ph + rate * (1.0 + 0.13 * v as f32) / sr).fract();
                let d = ((8.0 + depth * (0.5 + 0.5 * (TAU * *ph).sin())) * 0.001 * sr).max(2.0);
                let pan = if v % 2 == 0 { 0 } else { 1 };
                let y = self.lines[pan].read_cubic(d);
                if pan == 0 { wl += y } else { wr += y }
            }
            let (hl, hr) = (wl * norm, wr * norm);
            // Dimension: cross-fed short delays widen the image.
            self.dim[0].push(hl);
            self.dim[1].push(hr);
            let d = dim_ms * 0.001 * sr;
            let (xl, xr) = (self.dim[1].read(d), self.dim[0].read(d * 1.17));
            l[i] = hl + (xl - hl * 0.5) * dim_wet;
            r[i] = hr + (xr - hr * 0.5) * dim_wet;
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().chain(self.dim.iter_mut()).for_each(DelayLine::clear);
    }
}

// ---------------------------------------------------------------- Bode frequency shifter

struct Bode {
    shifters: [FrequencyShifter; 2],
    lines: [DelayLine; 2],
}

impl Bode {
    fn new(sr: f32) -> Self {
        let n = (sr * 5.5) as usize;
        Bode { shifters: Default::default(), lines: [DelayLine::new(n), DelayLine::new(n)] }
    }
}

impl Effect for Bode {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const SHIFT: usize = index("FXBode", "kParamShift");
        const RANGE: usize = index("FXBode", "kParamRange");
        const DELAY: usize = index("FXBode", "kParamDelayTime");
        const FEEDBACK: usize = index("FXBode", "kParamFeedback");
        const MIX: usize = index("FXBode", "kParamOutputMix");
        const WIDTH: usize = index("FXBode", "kParamOutputWidth");
        let sr = ctx.sample_rate;
        // Shift ±100 % of Range Hz (assumed).
        let hz = p.get(SHIFT) / 100.0 * p.get(RANGE);
        // Mix −100 = down only, +100 = up only; width moves the up-shift left and the
        // down-shift right (measured: Serum's default output is wide).
        let mix = (p.get(MIX) / 100.0 + 1.0) * 0.5;
        let width = p.get(WIDTH) / 100.0;
        let side_mix = [(mix + 0.5 * width).clamp(0.0, 1.0), (mix - 0.5 * width).clamp(0.0, 1.0)];
        let fb = (p.get(FEEDBACK) / 100.0 * 0.95).clamp(0.0, 0.95);
        let delay = (p.get(DELAY) * sr).max(1.0);
        for i in 0..l.len() {
            let mut out = [0f32; 2];
            for (ch, x) in [l[i], r[i]].into_iter().enumerate() {
                let input = x + self.lines[ch].read(delay) * fb;
                let (up, down) = self.shifters[ch].tick(input, hz, sr);
                let m = side_mix[ch];
                out[ch] = up * m + down * (1.0 - m);
                self.lines[ch].push(out[ch]);
            }
            l[i] = out[0];
            r[i] = out[1];
        }
    }

    fn reset(&mut self) {
        self.shifters.iter_mut().for_each(FrequencyShifter::reset);
        self.lines.iter_mut().for_each(DelayLine::clear);
    }
}

// ---------------------------------------------------------------- Convolution

struct Conv {
    convolver: Option<Convolver>,
    predelay: [DelayLine; 2],
    tone: [Biquad; 2],
}

impl Conv {
    fn new(impulse: Option<&SampleData>, sr: f32) -> Self {
        let convolver = impulse.map(|ir| {
            // Resample linearly to the engine rate.
            let ratio = ir.rate / sr;
            let channels: Vec<Vec<f32>> = ir
                .channels
                .iter()
                .map(|ch| {
                    let n = (ch.len() as f32 / ratio) as usize;
                    (0..n)
                        .map(|i| {
                            let x = i as f32 * ratio;
                            let j = x as usize;
                            let f = x - j as f32;
                            let a = ch[j.min(ch.len() - 1)];
                            let b = ch[(j + 1).min(ch.len() - 1)];
                            a + (b - a) * f
                        })
                        .collect()
                })
                .collect();
            Convolver::new(&channels)
        });
        let n = (sr * 3.0) as usize;
        Conv { convolver, predelay: [DelayLine::new(n), DelayLine::new(n)], tone: Default::default() }
    }
}

impl Effect for Conv {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const TRIM: usize = index("FXConv", "kParamIpTrim");
        const TONE: usize = index("FXConv", "kParamTone");
        const PREDELAY: usize = index("FXConv", "kParamPredelay");
        let Some(conv) = &mut self.convolver else {
            l.fill(0.0);
            r.fill(0.0);
            return;
        };
        let trim = db(p.get(TRIM));
        // Tone ±100 → ±12 dB tilt around 1 kHz (assumed).
        let tone = p.get(TONE) / 100.0 * 12.0;
        for t in &mut self.tone {
            t.set(Shape::HighShelf { gain_db: tone }, 1000.0, 0.707, ctx.sample_rate);
        }
        let pd = p.get(PREDELAY) * ctx.sample_rate;
        for i in 0..l.len() {
            self.predelay[0].push(l[i] * trim);
            self.predelay[1].push(r[i] * trim);
            l[i] = self.predelay[0].read(pd);
            r[i] = self.predelay[1].read(pd);
        }
        conv.process(l, r);
        for i in 0..l.len() {
            l[i] = self.tone[0].tick(l[i]);
            r[i] = self.tone[1].tick(r[i]);
        }
    }

    fn reset(&mut self) {
        if let Some(c) = &mut self.convolver {
            c.reset();
        }
        self.predelay.iter_mut().for_each(DelayLine::clear);
    }
}

#[cfg(test)]
mod dist_tests {
    use super::MeasuredCurve;

    #[test]
    fn measured_curves_reproduce_probe_points() {
        // Inside the rack (output ÷ the default master gain 0.354): tube at drive 50
        // hard-limits at 0.293/0.354 = 0.829 for a ±2.6 input (fxd4 probes).
        let c = MeasuredCurve::new("kTube", 50.0).unwrap();
        assert!((c.apply(2.5) - 0.829).abs() < 0.02, "{}", c.apply(2.5));
        assert!((c.apply(-2.5) + 0.829).abs() < 0.02);
        // Drive 0 passes small inputs at half level.
        assert!((MeasuredCurve::new("kSoftClip", 0.0).unwrap().apply(0.1) - 0.05).abs() < 0.003);
    }
}
