//! Frequency shifting with an IIR Hilbert transformer: two allpass chains whose
//! outputs stay ~90° apart over the audio band (coefficients from the standard
//! elliptic half-band design used for analytic-signal filters).

const A: [f32; 4] = [0.479_400_9, 0.876_218_9, 0.976_597_6, 0.997_499_7];
const B: [f32; 4] = [0.161_758_5, 0.733_028_7, 0.945_349_9, 0.990_599_2];

#[derive(Clone, Debug, Default)]
struct Chain {
    x1: [f32; 4],
    x2: [f32; 4],
    y1: [f32; 4],
    y2: [f32; 4],
}

impl Chain {
    /// Cascade of second-order allpasses in z⁻²: y = c·(x + y[n−2]) − x[n−2].
    #[inline]
    fn tick(&mut self, coefs: &[f32; 4], mut x: f32) -> f32 {
        for (i, &a) in coefs.iter().enumerate() {
            let c = a * a;
            let y = c * (x + self.y2[i]) - self.x2[i];
            self.x2[i] = self.x1[i];
            self.x1[i] = x;
            self.y2[i] = self.y1[i];
            self.y1[i] = y;
            x = y;
        }
        x
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrequencyShifter {
    real: Chain,
    imag: Chain,
    delayed: f32,
    phase: f32,
}

impl FrequencyShifter {
    pub fn reset(&mut self) {
        *self = FrequencyShifter { phase: self.phase, ..Default::default() };
    }

    /// Returns (up-shifted, down-shifted) by `shift_hz`.
    #[inline]
    pub fn tick(&mut self, x: f32, shift_hz: f32, sample_rate: f32) -> (f32, f32) {
        let i = self.real.tick(&A, x);
        // One-sample delay aligns the two chains.
        let q = self.imag.tick(&B, self.delayed);
        self.delayed = x;
        let (s, c) = (std::f32::consts::TAU * self.phase).sin_cos();
        self.phase = (self.phase + shift_hz / sample_rate).rem_euclid(1.0);
        (i * c + q * s, i * c - q * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dominant frequency by zero crossings.
    fn frequency(xs: &[f32], sr: f32) -> f32 {
        let crossings = xs.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 * sr / xs.len() as f32
    }

    #[test]
    fn shifts_a_tone_up_and_down() {
        let sr = 48000.0;
        let mut f = FrequencyShifter::default();
        let (mut up, mut down) = (Vec::new(), Vec::new());
        for n in 0..48000 {
            let x = (std::f32::consts::TAU * 1000.0 * n as f32 / sr).sin();
            let (u, d) = f.tick(x, 200.0, sr);
            if n > 4800 {
                up.push(u);
                down.push(d);
            }
        }
        assert!((frequency(&up, sr) - 1200.0).abs() < 15.0, "{}", frequency(&up, sr));
        assert!((frequency(&down, sr) - 800.0).abs() < 15.0, "{}", frequency(&down, sr));
    }
}
