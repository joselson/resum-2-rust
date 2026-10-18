//! Voice filters: a topology-preserving state-variable filter (Simper/Zavalishin)
//! and a 4-pole ladder with tanh feedback saturation.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Lowpass { poles: u8 },
    Highpass { poles: u8 },
    Bandpass { poles: u8 },
    Notch,
    Peak,
    /// Transistor ladder lowpass; output tapped after `poles` stages (6 dB/oct each),
    /// resonance always fed back from the fourth.
    Ladder { poles: u8 },
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
        let u = ((x - k * sigma) / (1.0 + k * gp4)).tanh();
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

#[derive(Clone, Debug)]
pub struct Filter {
    kind: FilterKind,
    svf: [[Svf; 2]; 2],
    ladder: [Ladder; 2],
    coefs: SvfCoefs,
    ladder_g: f32,
    ladder_k: f32,
    drive: f32,
}

impl Filter {
    pub fn new(kind: FilterKind) -> Self {
        Filter {
            kind,
            svf: Default::default(),
            ladder: Default::default(),
            coefs: SvfCoefs::new(1000.0, 0.707, 48000.0),
            ladder_g: 0.0,
            ladder_k: 0.0,
            drive: 1.0,
        }
    }

    pub fn kind(&self) -> FilterKind {
        self.kind
    }

    pub fn reset(&mut self) {
        self.svf = Default::default();
        self.ladder = Default::default();
    }

    /// `resonance` and `drive` ∈ [0, 1].
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, drive: f32, sample_rate: f32) {
        let cutoff = cutoff_hz.clamp(10.0, sample_rate * 0.49);
        let r = resonance.clamp(0.0, 1.0);
        // Q from 0.5 (no resonance) to ~25 (near self-oscillation).
        let q = 0.5 * (50f32).powf(r);
        self.coefs = SvfCoefs::new(cutoff, q, sample_rate);
        self.ladder_g = (PI * (cutoff / sample_rate).min(0.49)).tan();
        self.ladder_k = 4.0 * r * 0.98;
        self.drive = 1.0 + drive.clamp(0.0, 1.0) * 7.0;
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.channel(0, l), self.channel(1, r))
    }

    #[inline]
    fn channel(&mut self, ch: usize, x: f32) -> f32 {
        let x = if self.drive > 1.0 { (x * self.drive).tanh() / self.drive.sqrt() } else { x };
        let c = self.coefs;
        let [a, b] = &mut self.svf[ch];
        match self.kind {
            FilterKind::Lowpass { poles } => {
                let y = a.tick(&c, x).0;
                if poles > 2 { b.tick(&c, y).0 } else { y }
            }
            FilterKind::Highpass { poles } => {
                let y = a.tick(&c, x).2;
                if poles > 2 { b.tick(&c, y).2 } else { y }
            }
            FilterKind::Bandpass { poles } => {
                // Scale band output by k so the peak gain is unity regardless of Q.
                let y = a.tick(&c, x).1 * c.k;
                if poles > 2 { b.tick(&c, y).1 * c.k } else { y }
            }
            FilterKind::Notch => {
                let (lo, _, hi) = a.tick(&c, x);
                lo + hi
            }
            FilterKind::Peak => {
                let (lo, _, hi) = a.tick(&c, x);
                lo - hi
            }
            // No passband compensation: measured against Serum, resonance does not raise the level.
            FilterKind::Ladder { poles } => self.ladder[ch].tick(self.ladder_g, self.ladder_k, x, poles.clamp(1, 4) as usize),
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

    #[test]
    fn resonant_filter_stays_stable() {
        for kind in [FilterKind::Lowpass { poles: 4 }, FilterKind::Ladder { poles: 4 }, FilterKind::Ladder { poles: 1 }, FilterKind::Bandpass { poles: 2 }] {
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
