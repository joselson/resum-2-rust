//! Second-order sections from the RBJ audio-EQ cookbook, and a Linkwitz–Riley crossover.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Lowpass,
    Highpass,
    Bandpass,
    Peak { gain_db: f32 },
    LowShelf { gain_db: f32 },
    HighShelf { gain_db: f32 },
    Allpass,
}

impl Biquad {
    pub fn new(shape: Shape, hz: f32, q: f32, sample_rate: f32) -> Self {
        let mut b = Biquad::default();
        b.set(shape, hz, q, sample_rate);
        b
    }

    /// Recomputes coefficients and keeps the state (safe for smooth sweeps).
    pub fn set(&mut self, shape: Shape, hz: f32, q: f32, sample_rate: f32) {
        let w = 2.0 * PI * (hz / sample_rate).clamp(1e-5, 0.49);
        let (s, c) = w.sin_cos();
        let q = q.max(0.05);
        let alpha = s / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Lowpass => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            Shape::Highpass => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            Shape::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            Shape::Allpass => (1.0 - alpha, -2.0 * c, 1.0 + alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            Shape::Peak { gain_db } => {
                let a = 10f32.powf(gain_db / 40.0);
                (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
            }
            Shape::LowShelf { gain_db } => {
                let a = 10f32.powf(gain_db / 40.0);
                let k = 2.0 * a.sqrt() * alpha;
                (a * ((a + 1.0) - (a - 1.0) * c + k), 2.0 * a * ((a - 1.0) - (a + 1.0) * c), a * ((a + 1.0) - (a - 1.0) * c - k), (a + 1.0) + (a - 1.0) * c + k, -2.0 * ((a - 1.0) + (a + 1.0) * c), (a + 1.0) + (a - 1.0) * c - k)
            }
            Shape::HighShelf { gain_db } => {
                let a = 10f32.powf(gain_db / 40.0);
                let k = 2.0 * a.sqrt() * alpha;
                (a * ((a + 1.0) + (a - 1.0) * c + k), -2.0 * a * ((a - 1.0) + (a + 1.0) * c), a * ((a + 1.0) + (a - 1.0) * c - k), (a + 1.0) - (a - 1.0) * c + k, 2.0 * ((a - 1.0) - (a + 1.0) * c), (a + 1.0) - (a - 1.0) * c - k)
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Transposed direct form II.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// 4th-order Linkwitz–Riley band split: low + high sum flat in magnitude.
#[derive(Clone, Debug, Default)]
pub struct Crossover {
    low: [Biquad; 2],
    high: [Biquad; 2],
}

impl Crossover {
    pub fn set(&mut self, hz: f32, sample_rate: f32) {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        for b in &mut self.low {
            b.set(Shape::Lowpass, hz, q, sample_rate);
        }
        for b in &mut self.high {
            b.set(Shape::Highpass, hz, q, sample_rate);
        }
    }

    /// Returns (low, high).
    #[inline]
    pub fn split(&mut self, x: f32) -> (f32, f32) {
        let l = self.low[0].tick(x);
        let l = self.low[1].tick(l);
        let h = self.high[0].tick(x);
        let h = self.high[1].tick(h);
        (l, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(mut f: impl FnMut(f32) -> f32, hz: f32) -> f32 {
        let sr = 48000.0;
        let mut peak = 0f32;
        for i in 0..48000 {
            let y = f((2.0 * PI * hz * i as f32 / sr).sin());
            if i > 24000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn shelves_and_peaks_hit_their_gain() {
        let mut p = Biquad::new(Shape::Peak { gain_db: 6.0 }, 1000.0, 1.0, 48000.0);
        assert!((20.0 * gain(|x| p.tick(x), 1000.0).log10() - 6.0).abs() < 0.2);
        let mut s = Biquad::new(Shape::LowShelf { gain_db: -12.0 }, 500.0, 0.707, 48000.0);
        assert!((20.0 * gain(|x| s.tick(x), 50.0).log10() + 12.0).abs() < 0.5);
    }

    #[test]
    fn crossover_bands_split_and_sum_near_flat() {
        let mut x = Crossover::default();
        x.set(1000.0, 48000.0);
        let low = gain(|s| x.split(s).0, 100.0);
        let mut x2 = Crossover::default();
        x2.set(1000.0, 48000.0);
        let high = gain(|s| x2.split(s).1, 100.0);
        assert!(low > 0.95 && high < 0.05);
        let mut x3 = Crossover::default();
        x3.set(1000.0, 48000.0);
        let sum = gain(|s| { let (l, h) = x3.split(s); l + h }, 1000.0);
        assert!((sum - 1.0).abs() < 0.05, "{sum}");
    }
}
