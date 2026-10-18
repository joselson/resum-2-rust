//! Breakpoint-curve LFO.

/// A periodic shape defined by breakpoints over x ∈ [0, 1].
#[derive(Clone, Debug)]
pub struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Tension per segment, [0, 1] with 0.5 linear.
    tensions: Vec<f32>,
}

impl Curve {
    /// Points are sorted by x and clamped to [0, 1]. Returns None for fewer than one point.
    pub fn new(points: &[(f32, f32)], tensions: &[f32]) -> Option<Curve> {
        if points.is_empty() {
            return None;
        }
        let mut pts: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (xs, ys) = pts.into_iter().unzip();
        let tensions = tensions.iter().map(|t| t.clamp(0.0, 1.0)).collect();
        Some(Curve { xs, ys, tensions })
    }

    /// Triangle starting at the top, the shape of a freshly initialised LFO.
    pub fn triangle() -> Curve {
        Curve::new(&[(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)], &[]).unwrap()
    }

    pub fn sine() -> Curve {
        let points: Vec<(f32, f32)> = (0..=64).map(|i| {
            let x = i as f32 / 64.0;
            (x, 0.5 + 0.5 * (std::f32::consts::TAU * x).sin())
        }).collect();
        Curve::new(&points, &[]).unwrap()
    }

    /// Value in [0, 1] at x ∈ [0, 1).
    pub fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if n == 1 {
            return self.ys[0];
        }
        // Segment i spans xs[i]..xs[i+1]; before the first point the shape wraps from the last.
        let i = self.xs.partition_point(|&p| p <= x);
        let (x0, y0, x1, y1, seg) = if i == 0 {
            (self.xs[n - 1] - 1.0, self.ys[n - 1], self.xs[0], self.ys[0], n - 1)
        } else if i >= n {
            (self.xs[n - 1], self.ys[n - 1], self.xs[0] + 1.0, self.ys[0], n - 1)
        } else {
            (self.xs[i - 1], self.ys[i - 1], self.xs[i], self.ys[i], i - 1)
        };
        let width = x1 - x0;
        if width <= 1e-9 {
            return y1;
        }
        let t = ((x - x0) / width).clamp(0.0, 1.0);
        let tension = self.tensions.get(seg).copied().unwrap_or(0.5);
        y0 + (y1 - y0) * bend(t, tension)
    }
}

/// Tension 0.5 is linear; toward 0 or 1 the segment bows like a power curve.
#[inline]
fn bend(t: f32, tension: f32) -> f32 {
    let k = (tension - 0.5) * 2.0;
    if k.abs() < 1e-4 {
        t
    } else {
        t.powf(4f32.powf(-k))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoMode {
    /// Phase restarts on each note.
    Trigger,
    /// Phase runs continuously regardless of notes.
    Free,
    /// One cycle per note, then holds the final value.
    Envelope,
}

#[derive(Clone, Debug)]
pub struct Lfo {
    phase: f32,
    done: bool,
    elapsed: f32,
}

impl Default for Lfo {
    fn default() -> Self {
        Lfo { phase: 0.0, done: false, elapsed: 0.0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LfoTiming {
    pub hz: f32,
    pub mode: LfoMode,
    /// Start phase in cycles.
    pub phase: f32,
    /// Seconds before the LFO starts moving.
    pub delay: f32,
    /// Seconds to fade in after the delay.
    pub rise: f32,
}

impl Lfo {
    pub fn trigger(&mut self, timing: &LfoTiming) {
        if timing.mode != LfoMode::Free {
            self.phase = timing.phase.rem_euclid(1.0);
        }
        self.done = false;
        self.elapsed = 0.0;
    }

    /// Current phase in cycles, [0, 1).
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Advances by `dt` seconds and returns the unipolar value in [0, 1].
    pub fn advance(&mut self, curve: &Curve, timing: &LfoTiming, dt: f32) -> f32 {
        self.elapsed += dt;
        let moving = self.elapsed > timing.delay;
        if moving && !self.done {
            self.phase += timing.hz * dt;
            if self.phase >= 1.0 {
                if timing.mode == LfoMode::Envelope {
                    self.phase = 1.0 - 1e-6;
                    self.done = true;
                } else {
                    self.phase -= self.phase.floor();
                }
            }
        }
        let value = curve.eval(self.phase);
        if timing.rise > 0.0 {
            let fade = ((self.elapsed - timing.delay) / timing.rise).clamp(0.0, 1.0);
            value * fade
        } else {
            value
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_interpolates_and_wraps() {
        let c = Curve::new(&[(0.25, 0.0), (0.75, 1.0)], &[]).unwrap();
        assert!((c.eval(0.5) - 0.5).abs() < 1e-6);
        // Wrap segment from x=0.75 to 1.25 descends 1 → 0; x=0 is its midpoint.
        assert!((c.eval(0.0) - 0.5).abs() < 1e-6);
        assert!((c.eval(0.999) - 0.5).abs() < 0.01);
    }

    #[test]
    fn tension_bows_segments() {
        let low = Curve::new(&[(0.0, 0.0), (1.0, 1.0)], &[0.2]).unwrap();
        let high = Curve::new(&[(0.0, 0.0), (1.0, 1.0)], &[0.8]).unwrap();
        assert!(low.eval(0.5) < 0.5 && high.eval(0.5) > 0.5);
    }

    #[test]
    fn envelope_mode_stops_after_one_cycle() {
        let curve = Curve::new(&[(0.0, 0.0), (1.0, 1.0)], &[]).unwrap();
        let timing = LfoTiming { hz: 1.0, mode: LfoMode::Envelope, phase: 0.0, delay: 0.0, rise: 0.0 };
        let mut lfo = Lfo::default();
        lfo.trigger(&timing);
        for _ in 0..30 {
            lfo.advance(&curve, &timing, 0.1);
        }
        assert!(lfo.advance(&curve, &timing, 0.1) > 0.99);
    }

    #[test]
    fn delay_and_rise_fade_in() {
        let curve = Curve::new(&[(0.0, 1.0)], &[]).unwrap();
        let timing = LfoTiming { hz: 1.0, mode: LfoMode::Trigger, phase: 0.0, delay: 1.0, rise: 1.0 };
        let mut lfo = Lfo::default();
        lfo.trigger(&timing);
        assert_eq!(lfo.advance(&curve, &timing, 0.5), 0.0);
        assert!((lfo.advance(&curve, &timing, 1.0) - 0.5).abs() < 1e-6);
    }
}
