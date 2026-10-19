//! 2× polyphase IIR half-band resamplers, cascaded for 4×.
//!
//! Each half-band filter is H(z) = ½·(A₀(z²) + z⁻¹·A₁(z²)) with A₀, A₁ chains of
//! first-order allpasses, so both branches run at the low rate. Coefficients come
//! from the elliptic design of Valenzuela and Constantinides (as popularised by
//! the hiir library), computed once for a coefficient count and transition band.

use std::f64::consts::PI;
use std::sync::OnceLock;

/// Coefficients per branch at most (8 per half-band filter).
const MAX: usize = 4;

/// Allpass coefficients of a half-band filter with `count` coefficients and the
/// given transition bandwidth (fraction of the sample rate, below 0.5).
pub fn halfband_coefs(count: usize, transition: f64) -> Vec<f64> {
    let mut k = ((1.0 - transition * 2.0) * PI / 4.0).tan();
    k *= k;
    let kk = (1.0 - k * k).powf(0.25);
    let e = 0.5 * (1.0 - kk) / (1.0 + kk);
    let e4 = e.powi(4);
    let q = e * (1.0 + e4 * (2.0 + e4 * (15.0 + 150.0 * e4)));
    let order = (count * 2 + 1) as f64;
    (1..=count)
        .map(|c| {
            let c = c as f64;
            let (mut num, mut sign) = (0.0, 1.0);
            for i in 0..64 {
                let i = i as f64;
                let t = q.powf(i * (i + 1.0)) * ((2.0 * i + 1.0) * c * PI / order).sin() * sign;
                num += t;
                sign = -sign;
                if t.abs() < 1e-100 {
                    break;
                }
            }
            let (mut den, mut sign) = (0.0, -1.0);
            for i in 1..64 {
                let i = i as f64;
                let t = q.powf(i * i) * (2.0 * i * c * PI / order).cos() * sign;
                den += t;
                sign = -sign;
                if t.abs() < 1e-100 {
                    break;
                }
            }
            let ww = num * q.powf(0.25) / (den + 0.5);
            let w2 = ww * ww;
            let x = ((1.0 - w2 * k) * (1.0 - w2 / k)).sqrt() / (1.0 + w2);
            (1.0 - x) / (1.0 + x)
        })
        .collect()
}

/// Two allpass chains (even and odd coefficients) with their state; fixed-size so
/// building one never allocates (filters are rebuilt on the audio thread).
#[derive(Clone, Copy, Debug)]
struct Branches {
    coefs: [[f32; MAX]; 2],
    len: [usize; 2],
    /// Per section: previous input and output.
    state: [[(f32, f32); MAX]; 2],
}

impl Branches {
    fn new(coefs: &[f64]) -> Branches {
        assert!(coefs.len() <= 2 * MAX);
        let mut b = Branches { coefs: [[0.0; MAX]; 2], len: [0; 2], state: [[(0.0, 0.0); MAX]; 2] };
        for (i, &c) in coefs.iter().enumerate() {
            let branch = i % 2;
            b.coefs[branch][b.len[branch]] = c as f32;
            b.len[branch] += 1;
        }
        b
    }

    #[inline]
    fn run(&mut self, branch: usize, mut x: f32) -> f32 {
        let n = self.len[branch];
        for (a, s) in self.coefs[branch][..n].iter().zip(&mut self.state[branch][..n]) {
            // y[n] = a·(x[n] − y[n−1]) + x[n−1]
            let y = a * (x - s.1) + s.0;
            *s = (x, y);
            x = y;
        }
        x
    }

    fn reset(&mut self) {
        self.state = [[(0.0, 0.0); MAX]; 2];
    }
}

/// One 2× stage in each direction.
#[derive(Clone, Copy, Debug)]
struct Stage {
    up: Branches,
    down: Branches,
    /// Odd-phase input held for the next downsampling step.
    held: f32,
}

impl Stage {
    fn new(coefs: &[f64]) -> Stage {
        Stage { up: Branches::new(coefs), down: Branches::new(coefs), held: 0.0 }
    }

    #[inline]
    fn up(&mut self, x: f32) -> [f32; 2] {
        [self.up.run(0, x), self.up.run(1, x)]
    }

    #[inline]
    fn down(&mut self, x: [f32; 2]) -> f32 {
        let y = 0.5 * (self.down.run(0, x[0]) + self.down.run(1, self.held));
        self.held = x[1];
        y
    }

    fn reset(&mut self) {
        self.up.reset();
        self.down.reset();
        self.held = 0.0;
    }
}

/// Runs a per-sample process at 1×, 2× or 4× the host rate.
#[derive(Clone, Copy, Debug)]
pub struct Oversampler {
    factor: usize,
    /// Host rate ↔ 2×, and 2× ↔ 4× (needs a wider transition, so fewer coefficients).
    stages: [Stage; 2],
}

impl Oversampler {
    pub fn new(factor: usize) -> Oversampler {
        // The first stage has about 100 dB of image rejection above 22 kHz at 48 kHz.
        // The second stage's input is already band-limited to the host Nyquist (0.25
        // of its output rate), so passing 22 kHz needs only a 0.13 transition band.
        static COEFS: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();
        let (first, second) = COEFS.get_or_init(|| (halfband_coefs(8, 0.04), halfband_coefs(4, 0.13)));
        Oversampler { factor: factor.clamp(1, 4).next_power_of_two().min(4), stages: [Stage::new(first), Stage::new(second)] }
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn reset(&mut self) {
        self.stages.iter_mut().for_each(Stage::reset);
    }

    /// Upsamples `x`, applies `f` to each oversampled value and returns the
    /// downsampled result.
    #[inline]
    pub fn process(&mut self, x: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let [s1, s2] = &mut self.stages;
        match self.factor {
            1 => f(x),
            2 => {
                let u = s1.up(x);
                s1.down([f(u[0]), f(u[1])])
            }
            _ => {
                let u = s1.up(x);
                let a = s2.up(u[0]);
                let b = s2.up(u[1]);
                let d0 = s2.down([f(a[0]), f(a[1])]);
                let d1 = s2.down([f(b[0]), f(b[1])]);
                s1.down([d0, d1])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady-state peak of a sine at `freq` (Hz at 48 kHz) through `os` with `f`.
    fn peak(factor: usize, freq: f32, f: impl Fn(f32) -> f32 + Copy) -> f32 {
        let mut os = Oversampler::new(factor);
        let mut peak = 0f32;
        for i in 0..24000 {
            let x = (2.0 * std::f32::consts::PI * freq * i as f32 / 48000.0).sin();
            let y = os.process(x, f);
            if i > 12000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn coefficients_are_stable_allpasses() {
        let c = halfband_coefs(8, 0.04);
        assert_eq!(c.len(), 8);
        assert!(c.iter().all(|&a| a > 0.0 && a < 1.0), "{c:?}");
        assert!(c.windows(2).all(|w| w[0] < w[1]), "increasing: {c:?}");
    }

    #[test]
    fn passband_is_flat_and_round_trip_is_transparent() {
        for factor in [2, 4] {
            for freq in [100.0, 1000.0, 10000.0, 18000.0] {
                let g = peak(factor, freq, |x| x);
                assert!((g - 1.0).abs() < 0.01, "{factor}× at {freq} Hz: {g}");
            }
        }
    }

    #[test]
    fn images_of_a_nonlinearity_are_rejected() {
        // A hard clipper makes harmonics far above the host Nyquist; at 4× they are
        // removed before downsampling instead of folding back as inharmonic aliases.
        let clip = |x: f32| (x * 4.0).clamp(-1.0, 1.0);
        let alias_energy = |factor: usize| {
            let mut os = Oversampler::new(factor);
            let n = 4096;
            let f0 = 48000.0 * 37.0 / n as f32; // bin-centred, ~434 Hz
            let y: Vec<f32> = (0..n * 2).map(|i| os.process((2.0 * std::f32::consts::PI * f0 * i as f32 / 48000.0).sin(), clip)).skip(n).collect();
            // Energy outside the harmonics of f0 (DFT bins not multiples of 37).
            let mut off = 0.0f64;
            for k in (1..n / 2).filter(|k| !matches!(k % 37, 0 | 1 | 36)) {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, v) in y.iter().enumerate() {
                    let ph = 2.0 * std::f64::consts::PI * (k * i) as f64 / n as f64;
                    re += *v as f64 * ph.cos();
                    im -= *v as f64 * ph.sin();
                }
                off += re * re + im * im;
            }
            off
        };
        let (one, four) = (alias_energy(1), alias_energy(4));
        assert!(four < one * 0.01, "aliasing at 4× ({four}) not well below 1× ({one})");
    }
}
