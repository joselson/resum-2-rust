//! Sub oscillator: six fixed shapes read from band-limited single-frame tables.
//!
//! The shapes are Serum's as measured (osc2 sub probes, harmonic amplitudes and
//! phases fitted at two octaves): levels are relative to the sine, all share the
//! sine's phase except where noted, and none has a DC offset.

use crate::wavetable::{Wavetable, FRAME_LEN};
use realfft::num_complex::Complex;
use realfft::RealFftPlanner;
use std::f32::consts::PI;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubShape {
    Sine,
    /// A sine with a flattened top: measured harmonics 2–12 at −33 dB and below.
    RoundRect,
    /// Peak 1.218.
    Triangle,
    /// Rising ramp, ±1.1435, through zero at phase 0.
    Saw,
    /// ±0.680.
    Square,
    /// Low (−1.089) from phase 0.5 for 27.1% of the cycle, high (0.405) otherwise.
    Pulse,
}

const SHAPES: [SubShape; 6] = [SubShape::Sine, SubShape::RoundRect, SubShape::Triangle, SubShape::Saw, SubShape::Square, SubShape::Pulse];

/// RoundRect harmonics 1–12 as (cos, sin) coefficients of a unit-sine fundamental.
const ROUND_RECT: [(f32, f32); 12] = [
    (0.0, 1.0), (-0.0172, -0.0122), (0.0008, -0.0029), (0.0057, -0.0003), (-0.0003, -0.0208), (0.0016, 0.0013),
    (-0.0003, 0.0023), (-0.0021, 0.0004), (0.0003, 0.0046), (0.0001, -0.0010), (0.0, -0.0026), (0.0006, 0.0005),
];
const PULSE_WIDTH: f32 = 0.271;
const PULSE_HIGH: f32 = 0.405;
const PULSE_LOW: f32 = -1.089;

/// Fourier coefficients (cos, sin) of harmonic k ≥ 1 for x(φ) = Σ a cos 2πkφ + b sin 2πkφ.
fn coefficients(shape: SubShape, k: usize) -> (f32, f32) {
    let kf = k as f32;
    let odd = k % 2 == 1;
    match shape {
        SubShape::Sine => (0.0, if k == 1 { 1.0 } else { 0.0 }),
        SubShape::RoundRect => ROUND_RECT.get(k - 1).copied().unwrap_or((0.0, 0.0)),
        SubShape::Triangle if odd => (0.0, 1.218 * 8.0 / (PI * PI) / (kf * kf) * if k % 4 == 1 { 1.0 } else { -1.0 }),
        SubShape::Saw => (0.0, 1.1435 * 2.0 / (PI * kf) * if odd { 1.0 } else { -1.0 }),
        SubShape::Square if odd => (0.0, 0.680 * 4.0 / (PI * kf)),
        SubShape::Pulse => {
            // high + (low − high) · [0.5 ≤ φ < 0.5 + width): the step's harmonics.
            let (a, b) = (2.0 * PI * kf * 0.5, 2.0 * PI * kf * (0.5 + PULSE_WIDTH));
            let step = PULSE_LOW - PULSE_HIGH;
            (step * (b.sin() - a.sin()) / (PI * kf), step * (a.cos() - b.cos()) / (PI * kf))
        }
        _ => (0.0, 0.0),
    }
}

fn build(shape: SubShape) -> Wavetable {
    let inverse = RealFftPlanner::<f32>::new().plan_fft_inverse(FRAME_LEN);
    let mut spectrum = inverse.make_input_vec();
    // x[n] = Σ X[k] e^{i2πkn/N} over the full spectrum, so X[k] = N/2 · (a − i·b).
    for (k, bin) in spectrum.iter_mut().enumerate().take(FRAME_LEN / 2).skip(1) {
        let (a, b) = coefficients(shape, k);
        *bin = Complex::new(a, -b) * (FRAME_LEN as f32 / 2.0);
    }
    let mut cycle = inverse.make_output_vec();
    inverse.process(&mut spectrum, &mut cycle).expect("FFT sizes match");
    cycle.iter_mut().for_each(|x| *x /= FRAME_LEN as f32);
    Wavetable::from_cycles(&[cycle])
}

fn table(shape: SubShape) -> &'static Wavetable {
    static TABLES: OnceLock<Vec<Wavetable>> = OnceLock::new();
    let tables = TABLES.get_or_init(|| SHAPES.iter().map(|&s| build(s)).collect());
    &tables[SHAPES.iter().position(|&s| s == shape).unwrap_or(0)]
}

#[derive(Clone, Debug, Default)]
pub struct SubOscillator {
    phase: f32,
    increment: f32,
    level: usize,
}

impl SubOscillator {
    pub fn trigger(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    pub fn set_frequency(&mut self, hz: f32, sample_rate: f32) {
        self.increment = (hz / sample_rate).clamp(0.0, 0.5);
        self.level = Wavetable::level_for(self.increment);
    }

    #[inline]
    pub fn next(&mut self, shape: SubShape) -> f32 {
        let out = table(shape).read(self.level, 0.0, self.phase);
        let p = self.phase + self.increment;
        self.phase = p - p.floor();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycle(shape: SubShape) -> Vec<f32> {
        let mut o = SubOscillator::default();
        o.trigger(0.0);
        o.set_frequency(48.0, 48000.0);
        (0..1000).map(|_| o.next(shape)).collect()
    }

    #[test]
    fn shapes_have_measured_levels_and_no_dc() {
        for shape in SHAPES {
            let c = cycle(shape);
            let dc = c.iter().sum::<f32>() / c.len() as f32;
            assert!(dc.abs() < 0.01, "{shape:?} dc {dc}");
            // The fundamental is a unit sine for all but the pulse.
            let h1 = c.iter().enumerate().map(|(i, x)| x * (2.0 * PI * i as f32 / 1000.0).sin()).sum::<f32>() * 2.0 / 1000.0;
            if shape != SubShape::Pulse {
                let expect = match shape {
                    SubShape::Triangle => 1.218 * 8.0 / (PI * PI),
                    SubShape::Saw => 1.1435 * 2.0 / PI,
                    SubShape::Square => 0.680 * 4.0 / PI,
                    _ => 1.0,
                };
                assert!((h1 - expect).abs() < 0.01, "{shape:?} h1 {h1} vs {expect}");
            }
        }
        let pulse = cycle(SubShape::Pulse);
        assert!((pulse[100] - PULSE_HIGH).abs() < 0.05 && (pulse[630] - PULSE_LOW).abs() < 0.05, "{} {}", pulse[100], pulse[630]);
        let saw = cycle(SubShape::Saw);
        assert!(saw[1].abs() < 0.05 && saw[250] > 0.5 && saw[750] < -0.5, "rising through zero at phase 0");
    }
}
