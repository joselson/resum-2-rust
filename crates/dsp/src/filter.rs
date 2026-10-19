//! Voice filters: topology-preserving state-variable and one-pole sections
//! (Simper/Zavalishin) and a 4-pole ladder with tanh feedback saturation.
//!
//! The multimode responses follow measurements of Serum 2 (docs/CALIBRATION.md):
//! one resonant 2-pole section; 24 dB types add a second, gently resonant section
//! and 18 dB types a one-pole; 6 dB types are a single one-pole. Resonant types are
//! scaled by 1/(1 + 4r), the passband loss of a ladder at the same resonance.
//! Comb and flange types are delay lines of one cutoff period; phase types are
//! allpass chains with feedback.

use crate::oversample::Oversampler;
use std::f32::consts::PI;

/// Filter cutoff for a normalized frequency: 11.4 octaves up from MIDI note 0
/// (8.1758 Hz to 22.1 kHz). Matches Serum's displayed values (0.25 → 59 Hz,
/// 0.5 → 425 Hz, 0.75 → 3064 Hz) and its comb delays, which pin the span to within
/// a cent at 87 Hz, 286 Hz and 2 kHz. Also maps `Var` to comb damping frequencies.
pub fn cutoff_hz(n: f32) -> f32 {
    const LOW: f32 = 8.175_799;
    LOW * (11.4 * n.clamp(0.0, 1.0)).exp2()
}

/// One-pole filters inside the loop of comb, flange and phase types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damping {
    None,
    /// Lowpass at the cutoff × 2^(21·(Var − 0.5)), within 8 Hz–22 kHz (fitted at
    /// three cutoffs and four Var values, within 0.6 dB).
    Low,
    /// Highpass placed like `Low` (assumed by symmetry).
    High,
    /// Lowpass and highpass at the cutoff at Var 0 (measured); how Var moves them is assumed.
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedLadder {
    Ems,
    Acid,
    Dirty,
}

/// One output of a 2-pole section; band is the raw output (peak gain Q), peak is
/// (s² + s + 1)/(s² + s/Q + 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    Low,
    High,
    Band,
    Notch,
    Peak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    /// 6, 12, 18 or 24 dB/oct (`poles` 1–4).
    Lowpass { poles: u8 },
    Highpass { poles: u8 },
    /// 12 or 24 dB/oct skirts (`poles` 2 or 4); peak gain grows with resonance.
    Bandpass { poles: u8 },
    /// Unscaled notch, one or two sections.
    Notch { poles: u8 },
    /// (s² + s + 1)/(s² + s/Q + 1) per section: a dip at low resonance, a peak above.
    Peak { poles: u8 },
    /// Transistor ladder lowpass; output tapped after `poles` stages (6 dB/oct each),
    /// resonance always fed back from the fourth. Serum's MG Low types: feedback
    /// 4.04·r and no passband compensation (fitted within 0.2 dB).
    Ladder { poles: u8 },
    /// Serum's "MG Ladder": a 4-pole ladder with the passband loss compensated
    /// (gain (1 + k) at −2 dB), feedback 4·r and the cutoff 10% higher (fitted
    /// within 0.15 dB).
    CompensatedLadder,
    /// Serum's EMS, Acid and Dirty ladders: their own nonlinear designs, rendered with
    /// the MG Low 24 response for now; only their drive stages are measured.
    NamedLadder(NamedLadder),
    /// Delay of one cutoff period with feedforward and feedback, r = resonance:
    /// comb 0.5/(1 + r)·(1 + r²z⁻ᵀ)/(1 − r·z⁻ᵀ), flange 0.5/(1 + r)·(1 + z⁻ᵀ)/(1 − r·z⁻ᵀ)
    /// (fitted within 0.1 dB up to r = 0.9); `negative` flips both signs.
    Comb { flange: bool, negative: bool, damping: Damping },
    /// `stages` first-order allpasses at half the cutoff with feedback r through a
    /// 10 µs loop delay: 0.5/(1 + r)·(1 + A)/(1 − r·A) (fitted within 0.8 dB up to
    /// r = 0.9). The negative types mix −A/(1 + r) with feedback −r over 23 µs.
    Phaser { stages: u8, negative: bool },
    /// Serum's "Exp MM" (lowpass) and "Exp BPF" (raw band): one 2-pole section with its
    /// own Q curve, cutoff ×1.15 and a fixed −0.6/−0.85 dB gain (fitted within 0.2 dB).
    Exp { band: bool },
    /// Serum's "Wsp": a 2-pole lowpass with a higher Q curve, cutoff ×0.98, −1 dB
    /// (fitted within 0.5 dB).
    Whisper,
    /// Serum's dual types (LH, LB, … NN): `first` at the cutoff, then `second` at
    /// `cutoff_hz(0.94·Var)`, both with the resonance Q and no output scaling (fitted
    /// within 0.5 dB). `poles` 1 is the 6 dB pair of one-poles (LH 6).
    Dual { first: Response, second: Response, poles: u8 },
    /// Serum's triple types (L/B/H, L/P/H, L/N/H, B/P/N): Var crossfades the three
    /// outputs of one section in amplitude (0 → first, 0.5 → second, 1 → third); the
    /// 24 dB types cascade a second crossfaded section (fitted within 0.5 dB).
    Morph { responses: [Response; 3], poles: u8 },
    /// A single 2-pole section with the Scream Q curve (the shared curve until r = 0.7,
    /// then saturating at Q 25), fixed `gain`, at the cutoff. Serum's "French LP"
    /// fits within 0.7 dB at −12.1 dB; Scream LP/BP and Dist.Comb 1 are fitted only at
    /// the default resonance.
    Scream { band: bool, gain_db: i8 },
}

impl FilterKind {
    fn is_delay(self) -> bool {
        matches!(self, FilterKind::Comb { .. })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug)]
struct SvfCoefs {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl SvfCoefs {
    fn new(cutoff: f32, q: f32, sample_rate: f32) -> Self {
        let g = (PI * (cutoff / sample_rate).min(0.49)).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        SvfCoefs { a1, a2: g * a1, a3: g * g * a1, k }
    }
}

impl Svf {
    /// Returns (low, band, high).
    #[inline]
    fn tick(&mut self, c: &SvfCoefs, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - c.k * v1 - v2)
    }
}

/// Resonance (0–1) → Q of the resonant section, fitted to Serum's L12/H12/B12
/// responses within 0.15 dB; interpolated in log Q.
const Q_CURVE: [(f32, f32); 12] = [(0.0, 0.5), (0.1, 1.13), (0.2, 1.5), (0.3, 1.9), (0.4, 2.41), (0.5, 3.1), (0.6, 4.13), (0.7, 5.77), (0.8, 9.06), (0.9, 18.8), (0.95, 36.3), (1.0, 97.8)];
/// Q curves of the Exp and Wsp types (fitted at 0/0.1/0.5/0.9; the value at 1 is
/// extrapolated from the last segment).
const EXP_Q: [(f32, f32); 5] = [(0.0, 0.325), (0.1, 0.419), (0.5, 0.934), (0.9, 3.73), (1.0, 5.3)];
const SCREAM_Q: [(f32, f32); 12] = [(0.0, 0.5), (0.1, 1.13), (0.2, 1.48), (0.3, 1.89), (0.4, 2.39), (0.5, 3.0), (0.6, 3.92), (0.7, 5.39), (0.8, 7.96), (0.9, 13.78), (0.95, 18.63), (1.0, 25.18)];
const WHISPER_Q: [(f32, f32); 5] = [(0.0, 1.31), (0.1, 1.65), (0.5, 3.57), (0.9, 13.6), (1.0, 19.0)];

/// Output gain of the negative phase types against resonance (fitted).
const PHASE_N_GAIN: [(f32, f32); 7] = [(0.0, 0.5), (0.1, 0.484), (0.3, 0.434), (0.5, 0.392), (0.7, 0.356), (0.9, 0.327), (1.0, 0.314)];
/// Q of the second section of 24 dB types (fitted from L24/N24); flat above 0.5.
const Q2_CURVE: [(f32, f32); 6] = [(0.0, 0.5), (0.1, 0.85), (0.2, 0.894), (0.3, 0.941), (0.4, 0.965), (0.5, 0.989)];

fn curve(points: &[(f32, f32)], r: f32) -> f32 {
    let r = r.clamp(0.0, 1.0);
    let i = points.iter().rposition(|&(x, _)| x <= r).unwrap_or(0).min(points.len() - 2);
    let ((x0, y0), (x1, y1)) = (points[i], points[i + 1]);
    if r >= x1 {
        return y1;
    }
    (y0.ln() + (y1.ln() - y0.ln()) * (r - x0) / (x1 - x0)).exp()
}

/// One output of a section (see `Response`).
#[inline]
fn respond(r: Response, s: &mut Svf, c: &SvfCoefs, x: f32) -> f32 {
    let (lo, band, hi) = s.tick(c, x);
    match r {
        Response::Low => lo,
        Response::High => hi,
        Response::Band => band,
        Response::Notch => x - c.k * band,
        Response::Peak => x + (1.0 - c.k) * band,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct OnePole {
    s: f32,
}

impl OnePole {
    /// Returns (low, high); `g` is the prewarped g/(1+g).
    #[inline]
    fn tick(&mut self, g: f32, x: f32) -> (f32, f32) {
        let v = (x - self.s) * g;
        let lp = v + self.s;
        self.s = lp + v;
        (lp, x - lp)
    }
}

/// Drive (0–1) → input gain in dB before the filter's soft clipper (measured from
/// Serum's L12/H12/EMS drive sweeps).
const DRIVE_DB: [(f32, f32); 5] = [(0.0, 0.0), (0.25, 7.8), (0.5, 13.9), (0.8, 20.2), (1.0, 23.9)];
/// Ceiling of the cubic soft clipper ahead of the filter, in Lumen's internal level
/// at the filter (fitted against the drive sweeps, 0.4 dB mean; about 1.2 at Serum's
/// output divided by the default master gain of 0.35).
const CLIP_CEILING: f32 = 3.2;

/// Drive stage per type, from output-level sweeps (rms within 0.6 dB): MG Ladder
/// and EMS stay linear (+24 dB at full drive), Acid clips at 1.25× the usual ceiling,
/// Dirty at a quarter of it with half the gain in dB. Returns (ceiling, gain scale).
fn drive_stage(kind: FilterKind) -> (Option<f32>, f32) {
    match kind {
        FilterKind::CompensatedLadder | FilterKind::NamedLadder(NamedLadder::Ems) => (None, 1.0),
        FilterKind::NamedLadder(NamedLadder::Acid) => (Some(CLIP_CEILING * 1.25), 1.0),
        FilterKind::NamedLadder(NamedLadder::Dirty) => (Some(CLIP_CEILING * 0.25), 0.5),
        _ => (Some(CLIP_CEILING), 1.0),
    }
}

/// Cubic soft clip with unity small-signal gain, flat at ±`ceiling`.
#[inline]
fn soft_clip(x: f32, ceiling: f32) -> f32 {
    let u = (x / (1.5 * ceiling)).clamp(-1.0, 1.0);
    ceiling * (1.5 * u - 0.5 * u * u * u)
}

/// Headroom of the ladder's input saturation, set so self-oscillation at full
/// resonance reaches Serum's level (MgL6–24 within 3%).
const LADDER_HEADROOM: f32 = 14.5;

#[inline]
fn saturate(x: f32) -> f32 {
    LADDER_HEADROOM * (x / LADDER_HEADROOM).tanh()
}

#[derive(Clone, Copy, Debug, Default)]
struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    /// Zero-delay-feedback ladder solved with one-step linearisation of the tanh.
    /// Returns the output of stage `tap` (1–4).
    #[inline]
    fn tick(&mut self, g: f32, k: f32, x: f32, tap: usize) -> f32 {
        let gp = g / (1.0 + g);
        let gp4 = gp * gp * gp * gp;
        let sigma = self.s.iter().enumerate().fold(0.0, |acc, (i, s)| acc + s * gp.powi(3 - i as i32)) / (1.0 + g);
        // Serum's ladders measure linear at normal levels; the saturation only bounds
        // self-oscillation (feedback reaches 4.04 at full resonance).
        let u = saturate((x - k * sigma) / (1.0 + k * gp4));
        let mut y = u;
        let mut tapped = 0.0;
        for (i, s) in self.s.iter_mut().enumerate() {
            let v = (y - *s) * gp;
            let out = v + *s;
            *s = out + v;
            y = out;
            if i + 1 == tap {
                tapped = out;
            }
        }
        tapped
    }
}

/// Samples per channel of a comb delay line: one period of the lowest cutoff
/// (8.2 Hz) at 48 kHz. Combs run at the host rate, so at higher rates the lowest
/// comb frequency rises proportionally.
const DELAY_LEN: usize = 8192;

/// A comb channel's delay line and loop damping; allocated with the filter, so
/// changing type never allocates.
#[derive(Clone, Debug)]
struct DelayLine {
    buf: Box<[f32]>,
    pos: usize,
    lp: OnePole,
    hp: OnePole,
}

impl DelayLine {
    fn new() -> Self {
        DelayLine { buf: vec![0.0; DELAY_LEN].into_boxed_slice(), pos: 0, lp: OnePole::default(), hp: OnePole::default() }
    }

    /// The value written `delay` samples ago (≥ 2), with 4-point Hermite
    /// interpolation: linear interpolation's treble loss compounds through the
    /// feedback and dulls the upper comb teeth, which Serum's do not.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let k = (delay - 1.0).clamp(1.0, (DELAY_LEN - 4) as f32);
        let i = k as usize;
        let t = k - i as f32;
        let at = |j: usize| self.buf[(self.pos + DELAY_LEN - j) % DELAY_LEN];
        let (y0, y1, y2, y3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    #[inline]
    fn write(&mut self, x: f32) {
        self.pos = (self.pos + 1) % DELAY_LEN;
        self.buf[self.pos] = x;
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.lp = OnePole::default();
        self.hp = OnePole::default();
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CombParams {
    /// Delay in samples.
    delay: f32,
    ff: f32,
    fb: f32,
    gain: f32,
    lp_g: f32,
    hp_g: f32,
}

/// A stereo filter, optionally run oversampled (Serum filters at 4× by default,
/// 2× at its lowest oversampling setting). Comb types run at the host rate.
#[derive(Clone, Debug)]
pub struct Filter {
    cores: [Core; 2],
    os: [Oversampler; 2],
    /// Requested oversampling, kept while a comb type forces 1×.
    factor: usize,
    spread: f32,
    var: f32,
    lines: [DelayLine; 2],
    comb: [CombParams; 2],
}

impl Filter {
    pub fn new(kind: FilterKind) -> Self {
        let mut f = Filter { cores: [Core::new(kind); 2], os: [Oversampler::new(1); 2], factor: 1, spread: 0.0, var: 0.0, lines: [DelayLine::new(), DelayLine::new()], comb: Default::default() };
        f.set_kind(kind);
        f
    }

    pub fn kind(&self) -> FilterKind {
        self.cores[0].kind
    }

    /// Changes the type, clearing state; never allocates.
    pub fn set_kind(&mut self, kind: FilterKind) {
        self.cores = [Core::new(kind); 2];
        self.lines.iter_mut().for_each(DelayLine::reset);
        let factor = self.factor;
        self.factor = 0;
        self.set_oversampling(factor);
    }

    /// `Var` (0–1): comb damping and the second filter of morph types. Applied by the next `set`.
    pub fn set_var(&mut self, var: f32) {
        self.var = var.clamp(0.0, 1.0);
    }

    /// Cutoff offset between the channels in octaves: left up, right down (Serum's
    /// Stereo control). Applied by the next `set`.
    pub fn set_spread(&mut self, octaves: f32) {
        self.spread = octaves;
    }

    /// 1, 2 or 4; changing it resets the resampler state.
    pub fn set_oversampling(&mut self, factor: usize) {
        if self.factor == factor {
            return;
        }
        self.factor = factor;
        let effective = if self.kind().is_delay() { 1 } else { factor };
        if self.os[0].factor() != effective {
            self.os = [Oversampler::new(effective); 2];
        }
    }

    pub fn reset(&mut self) {
        self.cores.iter_mut().for_each(Core::reset);
        self.os.iter_mut().for_each(Oversampler::reset);
        self.lines.iter_mut().for_each(DelayLine::reset);
    }

    /// `resonance` and `drive` ∈ [0, 1]; `sample_rate` is the host rate.
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, drive: f32, sample_rate: f32) {
        let rate = sample_rate * self.os[0].factor() as f32;
        let kind = self.kind();
        for (ch, side) in [1.0f32, -1.0].into_iter().enumerate() {
            // Cutoff stays below the host Nyquist whatever the internal rate.
            let hz = (cutoff_hz * (side * self.spread).exp2()).min(sample_rate * 0.49);
            self.cores[ch].set(hz, resonance, drive, self.var, rate);
            if let FilterKind::Comb { flange, negative, damping } = kind {
                let r = resonance.clamp(0.0, 0.995);
                let sign = if negative { -1.0 } else { 1.0 };
                let one_g = |hz: f32| {
                    let g = (PI * (hz / rate).min(0.49)).tan();
                    g / (1.0 + g)
                };
                // At Var 0 the "both" damping sits at the cutoff; otherwise Var moves it from
                // far below to far above the cutoff.
                let damp_hz = if damping == Damping::Both { hz } else { (hz * (21.0 * (self.var - 0.5)).exp2()).clamp(8.18, 22094.0) };
                self.comb[ch] = CombParams {
                    delay: (rate / hz.max(10.0)).min((DELAY_LEN - 2) as f32),
                    ff: sign * if flange { 1.0 } else { r * r },
                    fb: sign * r,
                    gain: 0.5 / (1.0 + r),
                    lp_g: one_g(damp_hz),
                    hp_g: one_g(damp_hz),
                };
            }
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if let FilterKind::Comb { damping, .. } = self.kind() {
            return (self.comb_tick(0, l, damping), self.comb_tick(1, r, damping));
        }
        let [cl, cr] = &mut self.cores;
        let [a, b] = &mut self.os;
        (a.process(l, |x| cl.process(x)), b.process(r, |x| cr.process(x)))
    }

    #[inline]
    fn comb_tick(&mut self, ch: usize, x: f32, damping: Damping) -> f32 {
        let x = self.cores[ch].drive_in(x);
        let p = self.comb[ch];
        let line = &mut self.lines[ch];
        let mut d = line.read(p.delay);
        if matches!(damping, Damping::Low | Damping::Both) {
            d = line.lp.tick(p.lp_g, d).0;
        }
        if matches!(damping, Damping::High | Damping::Both) {
            d = line.hp.tick(p.hp_g, d).1;
        }
        // The saturation only bounds the loop near full resonance.
        let v = saturate(x + p.fb * d);
        line.write(v);
        p.gain * (v + p.ff * d)
    }
}

/// One channel of a filter.
#[derive(Clone, Copy, Debug)]
struct Core {
    kind: FilterKind,
    svf: [Svf; 2],
    one: OnePole,
    ladder: Ladder,
    coefs: SvfCoefs,
    /// Second section of 24 dB types.
    coefs2: SvfCoefs,
    one_g: f32,
    /// Output gain of the resonant multimode types.
    gain: f32,
    ladder_g: f32,
    ladder_k: f32,
    drive: f32,
    /// Phaser allpass states, loop-delay ring and coefficients.
    ap: [OnePole; 8],
    ring: [f32; 8],
    ring_pos: usize,
    ap_g: f32,
    loop_delay: usize,
    fb: f32,
    mix: f32,
    /// Var (0–1) for the morph types, and the second one-pole of LH 6.
    var: f32,
    one2: OnePole,
    one_g2: f32,
    /// Drive clip ceiling for this type (None: linear).
    clip: Option<f32>,
}

impl Core {
    fn new(kind: FilterKind) -> Self {
        Core {
            kind,
            svf: Default::default(),
            one: Default::default(),
            ladder: Default::default(),
            coefs: SvfCoefs::new(1000.0, 0.707, 48000.0),
            coefs2: SvfCoefs::new(1000.0, 0.707, 48000.0),
            one_g: 0.0,
            gain: 1.0,
            ladder_g: 0.0,
            ladder_k: 0.0,
            drive: 1.0,
            ap: [OnePole::default(); 8],
            ring: [0.0; 8],
            ring_pos: 0,
            ap_g: 0.0,
            loop_delay: 1,
            fb: 0.0,
            mix: 1.0,
            var: 0.0,
            one2: OnePole::default(),
            one_g2: 0.0,
            clip: drive_stage(kind).0,
        }
    }

    /// Drive gain into the soft clipper; bypassed at drive 0 (measured: no clipping).
    #[inline]
    fn drive_in(&self, x: f32) -> f32 {
        if self.drive <= 1.0 {
            return x;
        }
        match self.clip {
            Some(c) => soft_clip(x * self.drive, c),
            None => x * self.drive,
        }
    }

    fn reset(&mut self) {
        self.svf = Default::default();
        self.one = Default::default();
        self.ladder = Default::default();
        self.ap = [OnePole::default(); 8];
        self.ring = [0.0; 8];
        self.one2 = OnePole::default();
    }

    fn set(&mut self, cutoff_hz: f32, resonance: f32, drive: f32, var: f32, sample_rate: f32) {
        let cutoff = cutoff_hz.clamp(10.0, sample_rate * 0.49);
        let r = resonance.clamp(0.0, 1.0);
        self.coefs = SvfCoefs::new(cutoff, curve(&Q_CURVE, r), sample_rate);
        self.coefs2 = SvfCoefs::new(cutoff, curve(&Q2_CURVE, r), sample_rate);
        self.gain = 1.0 / (1.0 + 4.0 * r);
        let g = (PI * (cutoff / sample_rate).min(0.49)).tan();
        self.one_g = g / (1.0 + g);
        self.ladder_g = g;
        self.ladder_k = 4.04 * r;
        self.var = var;
        match self.kind {
            FilterKind::Dual { .. } => {
                let second = crate::filter::cutoff_hz(0.94 * var).min(sample_rate * 0.49);
                self.coefs2 = SvfCoefs::new(second, curve(&Q_CURVE, r), sample_rate);
                let g2 = (PI * (second / sample_rate).min(0.49)).tan();
                self.one_g2 = g2 / (1.0 + g2);
            }
            FilterKind::Morph { .. } => {}
            FilterKind::Exp { band } => {
                self.coefs = SvfCoefs::new((cutoff * 1.15).min(sample_rate * 0.49), curve(&EXP_Q, r), sample_rate);
                self.gain = if band { 0.906 } else { 0.933 };
            }
            FilterKind::Whisper => {
                self.coefs = SvfCoefs::new(cutoff * 0.98, curve(&WHISPER_Q, r), sample_rate);
                self.gain = 0.891;
            }
            FilterKind::Scream { gain_db, .. } => {
                self.coefs = SvfCoefs::new(cutoff, curve(&SCREAM_Q, r), sample_rate);
                self.gain = 10f32.powf(gain_db as f32 / 20.0);
            }
            _ => {}
        }
        if let FilterKind::Phaser { negative, .. } = self.kind {
            let r = r.min(0.995);
            let g = (PI * (cutoff * 0.507 / sample_rate).min(0.49)).tan();
            self.ap_g = g / (1.0 + g);
            // Loop delay 10 µs (negative types 23 µs): one or two samples at 2×/4×.
            let delay_s = if negative { 23e-6 } else { 10.4e-6 };
            self.loop_delay = ((delay_s * sample_rate).round() as usize).clamp(1, 7);
            if negative {
                self.fb = -r;
                self.mix = -1.0 / (1.0 + r);
                self.gain = curve(&PHASE_N_GAIN, r);
            } else {
                self.fb = r;
                self.mix = 1.0;
                self.gain = 0.5 / (1.0 + r);
            }
        }
        if self.kind == FilterKind::CompensatedLadder {
            self.ladder_g = (PI * (cutoff * 1.1 / sample_rate).min(0.49)).tan();
            self.ladder_k = 4.0 * r;
            self.gain = (1.0 + self.ladder_k) * 0.794;
        }
        let d = drive.clamp(0.0, 1.0);
        let i = DRIVE_DB.iter().rposition(|&(x, _)| x <= d).unwrap_or(0).min(DRIVE_DB.len() - 2);
        let ((x0, y0), (x1, y1)) = (DRIVE_DB[i], DRIVE_DB[i + 1]);
        self.drive = 10f32.powf((y0 + (y1 - y0) * (d - x0) / (x1 - x0)) * drive_stage(self.kind).1 / 20.0);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let x = self.drive_in(x);
        let (c, c2, g, gain) = (self.coefs, self.coefs2, self.one_g, self.gain);
        let [a, b] = &mut self.svf;
        let one = &mut self.one;
        let notch = |s: &mut Svf, c: &SvfCoefs, x: f32| x - c.k * s.tick(c, x).1;
        let peak = |s: &mut Svf, c: &SvfCoefs, x: f32| x + (1.0 - c.k) * s.tick(c, x).1;
        match self.kind {
            FilterKind::Lowpass { poles: 1 } => one.tick(g, x).0,
            FilterKind::Highpass { poles: 1 } => one.tick(g, x).1,
            FilterKind::Lowpass { poles } => {
                let y = a.tick(&c, x).0;
                gain * match poles {
                    2 => y,
                    3 => one.tick(g, y).0,
                    _ => b.tick(&c2, y).0,
                }
            }
            FilterKind::Highpass { poles } => {
                let y = a.tick(&c, x).2;
                gain * match poles {
                    2 => y,
                    3 => one.tick(g, y).1,
                    _ => b.tick(&c2, y).2,
                }
            }
            FilterKind::Bandpass { poles } => {
                // The raw band output: its peak gain is Q, as in Serum.
                let y = a.tick(&c, x).1;
                gain * if poles > 2 { b.tick(&c2, y).1 } else { y }
            }
            FilterKind::Notch { poles } => {
                let y = notch(a, &c, x);
                if poles > 2 { notch(b, &c2, y) } else { y }
            }
            FilterKind::Peak { poles } => {
                let y = peak(a, &c, x);
                gain * if poles > 2 { peak(b, &c2, y) } else { y }
            }
            // No passband compensation: measured against Serum, resonance does not raise the level.
            FilterKind::Ladder { poles } => self.ladder.tick(self.ladder_g, self.ladder_k, x, poles.clamp(1, 4) as usize),
            FilterKind::NamedLadder(_) => self.ladder.tick(self.ladder_g, self.ladder_k, x, 4),
            FilterKind::CompensatedLadder => gain * self.ladder.tick(self.ladder_g, self.ladder_k, x, 4),
            FilterKind::Phaser { stages, .. } => {
                // ring[ring_pos] holds the allpass output one sample back.
                let d = self.ring[(self.ring_pos + 8 - (self.loop_delay - 1)) % 8];
                let v = saturate(x + self.fb * d);
                let mut a = v;
                for s in &mut self.ap[..stages.clamp(1, 8) as usize] {
                    let (lp, hp) = s.tick(self.ap_g, a);
                    a = lp - hp;
                }
                self.ring_pos = (self.ring_pos + 1) % 8;
                self.ring[self.ring_pos] = a;
                gain * (v + self.mix * a)
            }
            FilterKind::Exp { band } => {
                let (lo, b, _) = a.tick(&c, x);
                gain * if band { b } else { lo }
            }
            FilterKind::Whisper => gain * a.tick(&c, x).0,
            FilterKind::Scream { band, .. } => {
                let (lo, b, _) = a.tick(&c, x);
                gain * if band { b } else { lo }
            }
            FilterKind::Dual { first, second, poles: 1 } => {
                let pick = |r: Response, (lo, hi): (f32, f32)| if r == Response::High { hi } else { lo };
                let y = pick(first, one.tick(g, x));
                pick(second, self.one2.tick(self.one_g2, y))
            }
            FilterKind::Dual { first, second, .. } => {
                let y = respond(first, a, &c, x);
                respond(second, b, &c2, y)
            }
            FilterKind::Morph { responses, poles } => {
                let t = self.var * 2.0;
                let (i, w) = if t < 1.0 { (0, t) } else { (1, t - 1.0) };
                let mix = |s: &mut Svf, c: &SvfCoefs, x: f32| {
                    let (lo, band, hi) = s.tick(c, x);
                    let out = |r: Response| match r {
                        Response::Low => lo,
                        Response::High => hi,
                        Response::Band => band,
                        Response::Notch => x - c.k * band,
                        Response::Peak => x + (1.0 - c.k) * band,
                    };
                    out(responses[i]) * (1.0 - w) + out(responses[i + 1]) * w
                };
                let y = mix(a, &c, x);
                if poles > 2 { mix(b, &c2, y) } else { y }
            }
            // Handled by `Filter` on its delay lines.
            FilterKind::Comb { .. } => x,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(kind: FilterKind, cutoff: f32, freq: f32) -> f32 {
        let sr = 48000.0;
        let mut f = Filter::new(kind);
        f.set(cutoff, 0.0, 0.0, sr);
        let mut peak = 0f32;
        for i in 0..48000 {
            let x = (2.0 * PI * freq * i as f32 / sr).sin() * 0.25;
            let (y, _) = f.process(x, x);
            if i > 24000 {
                peak = peak.max(y.abs());
            }
        }
        peak / 0.25
    }

    #[test]
    fn lowpass_passes_low_and_cuts_high() {
        assert!(gain_at(FilterKind::Lowpass { poles: 2 }, 1000.0, 100.0) > 0.95);
        assert!(gain_at(FilterKind::Lowpass { poles: 2 }, 1000.0, 10000.0) < 0.05);
        assert!(gain_at(FilterKind::Lowpass { poles: 4 }, 1000.0, 10000.0) < gain_at(FilterKind::Lowpass { poles: 2 }, 1000.0, 10000.0));
    }

    #[test]
    fn highpass_and_ladder_behave() {
        assert!(gain_at(FilterKind::Highpass { poles: 2 }, 1000.0, 100.0) < 0.05);
        assert!(gain_at(FilterKind::Highpass { poles: 2 }, 1000.0, 10000.0) > 0.95);
        assert!(gain_at(FilterKind::Ladder { poles: 4 }, 1000.0, 100.0) > 0.8);
        assert!(gain_at(FilterKind::Ladder { poles: 4 }, 1000.0, 10000.0) < 0.01);
        let two = gain_at(FilterKind::Ladder { poles: 2 }, 1000.0, 10000.0);
        assert!(two > gain_at(FilterKind::Ladder { poles: 4 }, 1000.0, 10000.0) && two < 0.05, "12 dB/oct tap: {two}");
    }

    fn gain_with(kind: FilterKind, resonance: f32, freq: f32) -> f32 {
        let sr = 48000.0;
        let mut f = Filter::new(kind);
        f.set(1000.0, resonance, 0.0, sr);
        let mut peak = 0f32;
        for i in 0..48000 {
            let x = (2.0 * PI * freq * i as f32 / sr).sin() * 0.25;
            let (y, _) = f.process(x, x);
            if i > 24000 {
                peak = peak.max(y.abs());
            }
        }
        peak / 0.25
    }

    #[test]
    fn multimode_types_follow_the_measured_structure() {
        // Passband scales by 1/(1 + 4r); the notch is unscaled.
        assert!((gain_with(FilterKind::Lowpass { poles: 2 }, 0.5, 30.0) - 1.0 / 3.0).abs() < 0.01);
        assert!((gain_with(FilterKind::Notch { poles: 2 }, 0.5, 30.0) - 1.0).abs() < 0.01);
        // The peak type dips 6 dB at the cutoff without resonance and peaks with it.
        assert!((gain_with(FilterKind::Peak { poles: 2 }, 0.0, 1000.0) - 0.5).abs() < 0.02);
        assert!(gain_with(FilterKind::Peak { poles: 2 }, 0.9, 1000.0) > 3.0);
        // Raw bandpass: peak gain is Q × 1/(1 + 4r), 0.5 without resonance.
        assert!((gain_with(FilterKind::Bandpass { poles: 2 }, 0.0, 1000.0) - 0.5).abs() < 0.02);
        // Slopes: the octave from 3× to 6× the cutoff costs about 6/12/18/24 dB.
        for (poles, db) in [(1u8, 6.0f32), (2, 12.0), (3, 18.0), (4, 24.0)] {
            let k = Filter::new(FilterKind::Lowpass { poles });
            let drop = 20.0 * (gain_with(k.kind(), 0.0, 3000.0) / gain_with(k.kind(), 0.0, 6000.0)).log10();
            assert!((drop - db).abs() < 3.0, "{poles} poles: {drop} dB/oct");
        }
    }

    #[test]
    fn q_curve_interpolates_monotonically() {
        assert_eq!(curve(&Q_CURVE, 0.0), 0.5);
        assert_eq!(curve(&Q_CURVE, 1.0), 97.8);
        assert_eq!(curve(&Q2_CURVE, 0.9), 0.989);
        let mut last = 0.0;
        for i in 0..=100 {
            let q = curve(&Q_CURVE, i as f32 / 100.0);
            assert!(q >= last, "{i}");
            last = q;
        }
    }

    #[test]
    fn resonant_filter_stays_stable() {
        for kind in [FilterKind::Lowpass { poles: 4 }, FilterKind::Ladder { poles: 4 }, FilterKind::Ladder { poles: 1 }, FilterKind::Bandpass { poles: 2 }, FilterKind::Peak { poles: 4 }, FilterKind::Lowpass { poles: 3 }] {
            let mut f = Filter::new(kind);
            f.set(2000.0, 1.0, 1.0, 44100.0);
            let mut max = 0f32;
            for i in 0..44100 {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let (y, _) = f.process(x, x);
                assert!(y.is_finite());
                max = max.max(y.abs());
            }
            assert!(max < 10.0, "{kind:?} peaked at {max}");
        }
    }
}
