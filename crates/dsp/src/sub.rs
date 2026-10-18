//! Sub oscillator: simple shapes, band-limited with polyBLEP where they have edges.

use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubShape {
    Sine,
    /// Square with softened edges.
    RoundRect,
    Triangle,
    Saw,
    Square,
    /// 25% duty cycle.
    Pulse,
}

#[derive(Clone, Debug, Default)]
pub struct SubOscillator {
    phase: f32,
    increment: f32,
    /// Leaky integrator state for the triangle (integrated band-limited square).
    tri: f32,
}

/// Two-sample polynomial band-limited step residual.
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

impl SubOscillator {
    pub fn trigger(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
        self.tri = 0.0;
    }

    pub fn set_frequency(&mut self, hz: f32, sample_rate: f32) {
        self.increment = (hz / sample_rate).clamp(0.0, 0.45);
    }

    #[inline]
    pub fn next(&mut self, shape: SubShape) -> f32 {
        let (t, dt) = (self.phase, self.increment);
        let square = |duty: f32| -> f32 {
            let mut v = if t < duty { 1.0 } else { -1.0 };
            v += blep(t, dt);
            v -= blep((t - duty).rem_euclid(1.0), dt);
            v
        };
        let out = match shape {
            SubShape::Sine => (TAU * t).sin(),
            SubShape::RoundRect => ((TAU * t).sin() * 4.0).tanh() / 4f32.tanh(),
            SubShape::Saw => 1.0 - 2.0 * t + blep(t, dt),
            SubShape::Square => square(0.5),
            SubShape::Pulse => square(0.25),
            SubShape::Triangle => {
                // Integrate the band-limited square; the leak keeps DC from drifting.
                self.tri = self.tri * 0.9995 + square(0.5) * 4.0 * dt;
                self.tri
            }
        };
        let p = self.phase + dt;
        self.phase = p - p.floor();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_are_bounded_and_periodic() {
        for shape in [SubShape::Sine, SubShape::RoundRect, SubShape::Triangle, SubShape::Saw, SubShape::Square, SubShape::Pulse] {
            let mut o = SubOscillator::default();
            o.trigger(0.0);
            o.set_frequency(100.0, 48000.0);
            let mut peak = 0f32;
            let mut sum = 0f32;
            for i in 0..48000 {
                let v = o.next(shape);
                if i > 4800 {
                    peak = peak.max(v.abs());
                    sum += v;
                }
            }
            assert!(peak > 0.5 && peak < 1.3, "{shape:?} peak {peak}");
            let dc = sum / 43200.0;
            assert!(dc.abs() < if shape == SubShape::Pulse { 0.55 } else { 0.05 }, "{shape:?} dc {dc}");
        }
    }
}
