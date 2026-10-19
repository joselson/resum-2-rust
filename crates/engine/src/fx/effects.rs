//! Effect implementations. Each `process` produces the fully wet signal; the
//! rack applies enable, wet/dry and output level. Parameter scales marked
//! "assumed" await calibration against render probes (docs/CALIBRATION.md).

use super::{FxContext, FxParams};
use crate::filters::{self, cutoff_hz};
use crate::params::index;
use lumen_dsp::biquad::{Biquad, Shape};
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
        "FXComp" => Box::new(Comp::default()),
        "FXReverb" => Box::new(ReverbFx { reverb: Reverb::new(sr) }),
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

/// Q for a bandwidth in octaves.
fn q_for_octaves(bw: f32) -> f32 {
    let bw = bw.max(0.05);
    let e = (std::f32::consts::LN_2 / 2.0 * bw).sinh();
    1.0 / (2.0 * e)
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
            // Resonance 0–100 → Q 0.1–10 (assumed); types 0 shelf, 1 peak, 2 cut (assumed).
            let q = 0.1 * 100f32.powf(p.get(q) / 100.0);
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
    pre: [Biquad; 2],
    held: [f32; 2],
    hold_count: f32,
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
            // LPHP 0 = low-pass, 50 = band-pass, 100 = high-pass.
            let morph = p.get(LPHP);
            let shape = if morph < 33.0 { Shape::Lowpass } else if morph < 67.0 { Shape::Bandpass } else { Shape::Highpass };
            let q = q_for_octaves(p.get(BW));
            for b in &mut self.pre {
                b.set(shape, cutoff_hz(p.get(FREQ)), q, ctx.sample_rate);
            }
        }
        let hold = 1.0 + drive * 31.0;
        for i in 0..l.len() {
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let mut x = *s;
                if prepost == 1 {
                    x = self.pre[ch].tick(x);
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
                } else {
                    shaper::shape(shaper, x * gain) * TRIM
                };
                if prepost == 2 {
                    x = self.pre[ch].tick(x);
                }
                *s = x;
            }
        }
    }

    fn reset(&mut self) {
        self.pre.iter_mut().for_each(Biquad::reset);
    }
}

// ---------------------------------------------------------------- Delay

struct Delay {
    lines: [DelayLine; 2],
    filters: [[Biquad; 2]; 2],
}

impl Delay {
    fn new(sr: f32) -> Self {
        let n = (sr * 8.0) as usize;
        Delay { lines: [DelayLine::new(n), DelayLine::new(n)], filters: Default::default() }
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
        // Synced times are fractions of a bar (assumed); free times are seconds (measured).
        let unit = if p.flag(SYNC) { bar_seconds(ctx.bpm) } else { 1.0 };
        let tl = p.get(TIME_L) * unit * p.get(OFFSET_L);
        let tr = if p.flag(LINK) { p.get(TIME_L) } else { p.get(TIME_R) } * unit * p.get(OFFSET_R);
        let (dl, dr) = ((tl * sr).max(1.0), (tr * sr).max(1.0));
        let fb = (p.get(FEEDBACK) / 100.0).clamp(0.0, 0.98);
        let ping_pong = p.get(MODE).round() as i32 == 1;
        let q = q_for_octaves(p.get(BW));
        for f in self.filters.iter_mut().flatten() {
            f.set(Shape::Bandpass, p.get(FREQ), q, sr);
        }
        for i in 0..l.len() {
            let yl = self.lines[0].read(dl);
            let yr = self.lines[1].read(dr);
            let [[fl0, fl1], [fr0, fr1]] = &mut self.filters;
            let fl = fl1.tick(fl0.tick(yl));
            let fr = fr1.tick(fr0.tick(yr));
            if ping_pong {
                self.lines[0].push(0.5 * (l[i] + r[i]) + fr * fb);
                self.lines[1].push(fl * fb);
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

#[derive(Default)]
struct Comp {
    env: f32,
}

impl Effect for Comp {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        const THRESH: usize = index("FXComp", "kParamThresh");
        const RATIO: usize = index("FXComp", "kParamRatio");
        const ATTACK: usize = index("FXComp", "kParamAttack");
        const RELEASE: usize = index("FXComp", "kParamRelease");
        const MAKEUP: usize = index("FXComp", "kParamMakeup");
        let sr = ctx.sample_rate;
        // Threshold is a depth below 0 dB: −32·t dB (measured with a limiter ratio:
        // t ≤ 0.25 leaves a −10.7 dB peak untouched, t = 0.5 holds it near −16 dB).
        let thresh_db = -32.0 * p.get(THRESH);
        let ratio = p.get(RATIO).max(1.0);
        let att = (-1.0 / (p.get(ATTACK).max(0.05) * 0.001 * sr)).exp();
        let rel = (-1.0 / (p.get(RELEASE).max(0.05) * 0.001 * sr)).exp();
        let makeup = db(p.get(MAKEUP));
        for i in 0..l.len() {
            let level = l[i].abs().max(r[i].abs());
            let coef = if level > self.env { att } else { rel };
            self.env = level + coef * (self.env - level);
            let env_db = 20.0 * self.env.max(1e-6).log10();
            let over = env_db - thresh_db;
            let gain = if over > 0.0 { db(-over * (1.0 - 1.0 / ratio)) } else { 1.0 };
            l[i] *= gain * makeup;
            r[i] *= gain * makeup;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
    }
}

// ---------------------------------------------------------------- Reverb

struct ReverbFx {
    reverb: Reverb,
}

impl Effect for ReverbFx {
    fn process(&mut self, p: &FxParams, l: &mut [f32], r: &mut [f32], _ctx: &FxContext) {
        const SIZE: usize = index("FXReverb", "kParamSize");
        const DELAY: usize = index("FXReverb", "kParamDelay");
        const FREQ: usize = index("FXReverb", "kParamFreq");
        const FEEDBACK: usize = index("FXReverb", "kParamFeedback");
        const FREQ_B: usize = index("FXReverb", "kParamFreqB");
        const TYPE: usize = index("FXReverb", "kParamType");
        const WIDTH: usize = index("FXReverb", "kParamWidth");
        let kind = p.choice(TYPE);
        let fb = p.get(FEEDBACK) / 100.0;
        // Decay 0–100 → 0.3–20 s (assumed, squared for resolution at short times).
        let mut decay = 0.3 + fb * fb * 19.7;
        let mut modulation = 0.2;
        if kind == "kAbyss" {
            decay *= 2.5;
            modulation = 0.6;
        } else if kind == "kSpace" {
            modulation = 0.8;
        }
        let params = ReverbParams {
            size: p.get(SIZE) / 100.0,
            decay,
            predelay_s: p.get(DELAY) * 0.001,
            // Damping 0–100 → 1–20 kHz; low cut 0–100 → 20 Hz–1 kHz (assumed).
            damping_hz: 1000.0 * 20f32.powf(p.get(FREQ_B) / 100.0),
            low_cut_hz: 20.0 * 50f32.powf(p.get(FREQ) / 100.0),
            modulation,
            // Width 0–100 % (measured: the default 20 is much narrower than full width).
            width: p.get(WIDTH) / 100.0,
        };
        self.reverb.process(&params, l, r);
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
