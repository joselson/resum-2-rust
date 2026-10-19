//! Breakpoint-curve LFO and the chaotic (Lorenz, Rössler) LFO types.

/// A periodic shape defined by breakpoints over x ∈ [0, 1].
#[derive(Clone, Debug, PartialEq)]
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

    /// Serum's default LFO shape: 0 at phase 0, 1 at phase 0.5 (lfo probes).
    pub fn triangle() -> Curve {
        Curve::new(&[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)], &[]).unwrap()
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

/// Tension 0.5 is linear; below it a segment starts fast and above it slowly, rising
/// or falling (mod curve and LFO shape probes), on the shared curve law.
#[inline]
fn bend(t: f32, tension: f32) -> f32 {
    crate::curve::shape(t, (0.5 - tension) * 2.0)
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

/// Which way the shape is read (direction probes: 1 plays it backwards, 2 forwards
/// then backwards over two periods).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
    PingPong,
}

#[derive(Clone, Debug)]
pub struct Lfo {
    phase: f32,
    done: bool,
    elapsed: f32,
    /// Completed cycles, for ping-pong.
    cycles: u32,
    direction: Direction,
}

impl Default for Lfo {
    fn default() -> Self {
        Lfo { phase: 0.0, done: false, elapsed: 0.0, cycles: 0, direction: Direction::Forward }
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
    pub direction: Direction,
}

impl Lfo {
    pub fn trigger(&mut self, timing: &LfoTiming) {
        if timing.mode != LfoMode::Free {
            self.phase = timing.phase.rem_euclid(1.0);
        }
        self.done = false;
        self.elapsed = 0.0;
        self.cycles = 0;
        self.direction = timing.direction;
    }

    /// Current phase in cycles, [0, 1).
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Where the shape is read, [0, 1): the phase after the direction is applied.
    pub fn position(&self) -> f32 {
        let backwards = match self.direction {
            Direction::Forward => false,
            Direction::Reverse => true,
            Direction::PingPong => self.cycles % 2 == 1,
        };
        if backwards {
            (1.0 - self.phase).min(1.0 - 1e-6)
        } else {
            self.phase
        }
    }

    /// Advances by `dt` seconds and returns the unipolar value in [0, 1].
    pub fn advance(&mut self, curve: &Curve, timing: &LfoTiming, dt: f32) -> f32 {
        self.elapsed += dt;
        let moving = self.elapsed > timing.delay;
        self.direction = timing.direction;
        if moving && !self.done {
            self.phase += timing.hz * dt;
            if self.phase >= 1.0 {
                if timing.mode == LfoMode::Envelope {
                    self.phase = 1.0 - 1e-6;
                    self.done = true;
                } else {
                    self.cycles = self.cycles.wrapping_add(self.phase as u32);
                    self.phase -= self.phase.floor();
                }
            }
        }
        let value = curve.eval(self.position());
        if timing.rise > 0.0 {
            let fade = ((self.elapsed - timing.delay) / timing.rise).clamp(0.0, 1.0);
            value * fade
        } else {
            value
        }
    }
}

/// Path LFO: a polygon in XY traversed once per cycle, X driving the LFO and Y the
/// "LFO Y" source. Every segment takes an equal share of the cycle whatever its length,
/// coordinates are used as stored, and a closed path returns to its first point
/// (path probes). Tension bows a segment sideways as a quadratic curve, sagging
/// (0.5 − tension)·0.83·length at its middle to the right of travel; this is fitted
/// to one probe and approximate.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    points: Vec<(f32, f32)>,
    tensions: Vec<f32>,
    closed: bool,
}

impl Path {
    pub fn new(points: &[(f32, f32)], tensions: &[f32], closed: bool) -> Option<Path> {
        (!points.is_empty()).then(|| Path { points: points.to_vec(), tensions: tensions.to_vec(), closed })
    }

    /// (X, Y) at `phase` ∈ [0, 1).
    pub fn eval(&self, phase: f32) -> (f32, f32) {
        let n = self.points.len();
        let segments = if self.closed { n } else { n - 1 };
        if segments == 0 {
            return self.points[0];
        }
        let pos = phase.rem_euclid(1.0) * segments as f32;
        let seg = (pos as usize).min(segments - 1);
        let u = pos - seg as f32;
        let (a, b) = (self.points[seg], self.points[(seg + 1) % n]);
        let sag = (0.5 - self.tensions.get(seg).copied().unwrap_or(0.5)) * 0.83;
        // Quadratic Bézier whose control point sits off the chord's middle; at u = 0.5
        // the curve is half the control offset from the chord.
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (cx, cy) = ((a.0 + b.0) * 0.5 + dy * sag * 2.0, (a.1 + b.1) * 0.5 - dx * sag * 2.0);
        let w = (1.0 - u) * (1.0 - u);
        let (x, y) = (w * a.0 + 2.0 * u * (1.0 - u) * cx + u * u * b.0, w * a.1 + 2.0 * u * (1.0 - u) * cy + u * u * b.1);
        (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
    }
}

/// Chaotic LFO: a Lorenz or Rössler system restarted from a fixed point on every
/// trigger and integrated at a speed proportional to the LFO rate; the X output (the
/// LFO itself) and the Y output (the "LFO Y" mod source) are two coordinates. Constants
/// fitted to Serum's renders at 1, 2 and 4 Hz: the trajectories match within rms
/// 0.005–0.015 for seconds before chaos amplifies integration differences.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChaosKind {
    /// σ 10, ρ 28, β 8/3; 2 time units per second per Hz; from (0.405, 0.405, 0);
    /// X (x + 20)/40, Y (z − 2.16)/50.5.
    Lorenz,
    /// a 0.2, b 0.2, c 5.7; 5.85 time units per second per Hz; from (0, 0.32, 0);
    /// X (x + 10.1)/22.5, Y (y + 11.7)/22.5 (scale from the saturated attractor's range).
    Rossler,
}

#[derive(Clone, Copy, Debug)]
pub struct Chaos {
    state: [f64; 3],
}

impl Default for Chaos {
    fn default() -> Self {
        Chaos { state: [0.0; 3] }
    }
}

impl ChaosKind {
    fn start(self) -> [f64; 3] {
        match self {
            ChaosKind::Lorenz => [0.405, 0.405, 0.0],
            ChaosKind::Rossler => [0.0, 0.32, 0.0],
        }
    }

    fn speed(self) -> f64 {
        match self {
            ChaosKind::Lorenz => 2.0,
            ChaosKind::Rossler => 5.85,
        }
    }

    fn derivative(self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        match self {
            ChaosKind::Lorenz => [10.0 * (y - x), x * (28.0 - z) - y, x * y - 8.0 / 3.0 * z],
            ChaosKind::Rossler => [-y - z, x + 0.2 * y, 0.2 + z * (x - 5.7)],
        }
    }

    fn outputs(self, [x, y, z]: [f64; 3]) -> [f32; 2] {
        let v = match self {
            ChaosKind::Lorenz => [(x + 20.0) / 40.0, (z - 2.16) / 50.5],
            ChaosKind::Rossler => [(x + 10.1) / 22.5, (y + 11.7) / 22.5],
        };
        v.map(|v| (v as f32).clamp(0.0, 1.0))
    }
}

impl Chaos {
    pub fn trigger(&mut self, kind: ChaosKind) {
        self.state = kind.start();
    }

    /// Advances by `dt` seconds at `hz` and returns the X and Y outputs in [0, 1].
    pub fn advance(&mut self, kind: ChaosKind, hz: f32, dt: f32) -> [f32; 2] {
        let span = kind.speed() * hz.max(0.0) as f64 * dt as f64;
        // RK4 steps of at most 0.005 time units keep both systems on their attractors.
        let steps = ((span / 0.005).ceil() as usize).clamp(1, 256);
        let h = span / steps as f64;
        let add = |a: [f64; 3], b: [f64; 3], k: f64| [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k];
        for _ in 0..steps {
            let s = self.state;
            let k1 = kind.derivative(s);
            let k2 = kind.derivative(add(s, k1, h / 2.0));
            let k3 = kind.derivative(add(s, k2, h / 2.0));
            let k4 = kind.derivative(add(s, k3, h));
            self.state = std::array::from_fn(|i| s[i] + h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]));
        }
        if self.state.iter().any(|v| !v.is_finite()) {
            self.state = kind.start();
        }
        kind.outputs(self.state)
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
        // Tension 0.2 measured 1 − 2^(−10t): 0.969 halfway.
        assert!((low.eval(0.5) - 0.969).abs() < 0.01 && high.eval(0.5) < 0.05, "{} {}", low.eval(0.5), high.eval(0.5));
    }

    #[test]
    fn envelope_mode_stops_after_one_cycle() {
        let curve = Curve::new(&[(0.0, 0.0), (1.0, 1.0)], &[]).unwrap();
        let timing = LfoTiming { hz: 1.0, mode: LfoMode::Envelope, phase: 0.0, delay: 0.0, rise: 0.0, direction: Direction::Forward };
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
        let timing = LfoTiming { hz: 1.0, mode: LfoMode::Trigger, phase: 0.0, delay: 1.0, rise: 1.0, direction: Direction::Forward };
        let mut lfo = Lfo::default();
        lfo.trigger(&timing);
        assert_eq!(lfo.advance(&curve, &timing, 0.5), 0.0);
        assert!((lfo.advance(&curve, &timing, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn directions_reverse_and_ping_pong() {
        let ramp = Curve::new(&[(0.0, 0.0), (1.0, 1.0)], &[]).unwrap();
        let run = |direction| {
            let timing = LfoTiming { hz: 1.0, mode: LfoMode::Trigger, phase: 0.0, delay: 0.0, rise: 0.0, direction };
            let mut lfo = Lfo::default();
            lfo.trigger(&timing);
            (0..20).map(|_| lfo.advance(&ramp, &timing, 0.1)).collect::<Vec<f32>>()
        };
        let (fwd, rev, pp) = (run(Direction::Forward), run(Direction::Reverse), run(Direction::PingPong));
        assert!(fwd[2] < fwd[3] && rev[2] > rev[3], "{fwd:?} {rev:?}");
        assert!(pp[2] < pp[3] && pp[12] > pp[13], "{pp:?}");
    }

    #[test]
    fn paths_give_each_segment_an_equal_share() {
        let tri = Path::new(&[(0.1, 0.5), (0.9, 0.5), (0.5, 0.6)], &[0.5, 0.5, 0.5], true).unwrap();
        // Measured: x 0.1 → 0.9 over the first third, y rising to 0.6 at two thirds.
        let (x, y) = tri.eval(1.0 / 6.0);
        assert!((x - 0.5).abs() < 1e-4 && (y - 0.5).abs() < 1e-4);
        assert!((tri.eval(2.0 / 3.0).1 - 0.6).abs() < 1e-4 && (tri.eval(0.999).0 - 0.1).abs() < 0.01);
        let bowed = Path::new(&[(0.1, 0.5), (0.9, 0.5), (0.5, 0.6)], &[0.2, 0.5, 0.5], true).unwrap();
        assert!((bowed.eval(1.0 / 6.0).1 - 0.30).abs() < 0.01, "measured 0.30: {:?}", bowed.eval(1.0 / 6.0));
        let open = Path::new(&[(0.1, 0.2), (0.9, 0.5), (0.3, 0.9)], &[], false).unwrap();
        assert!((open.eval(0.5).0 - 0.9).abs() < 1e-4 && (open.eval(0.999).1 - 0.9).abs() < 0.01);
    }

    #[test]
    fn lorenz_escapes_the_origin_then_circles_a_lobe() {
        // Serum at 1 Hz: first peak 0.99 at 0.21 s, then a lobe centred near 0.29.
        let mut c = Chaos::default();
        c.trigger(ChaosKind::Lorenz);
        let dt = 1.0 / 1000.0;
        let trace: Vec<f32> = (0..2000).map(|_| c.advance(ChaosKind::Lorenz, 1.0, dt)[0]).collect();
        let (peak_i, peak) = trace[..500].iter().enumerate().fold((0, 0f32), |b, (i, &v)| if v > b.1 { (i, v) } else { b });
        assert!((peak - 0.99).abs() < 0.02 && (peak_i as f32 * dt - 0.208).abs() < 0.02, "{peak} at {peak_i}");
        let lobe = trace[1000..2000].iter().sum::<f32>() / 1000.0;
        assert!((lobe - 0.29).abs() < 0.02, "{lobe}");
    }

    #[test]
    fn chaos_stays_bounded_at_extreme_rates() {
        for kind in [ChaosKind::Lorenz, ChaosKind::Rossler] {
            let mut c = Chaos::default();
            c.trigger(kind);
            for _ in 0..10000 {
                let v = c.advance(kind, 1000.0, 1.0 / 48000.0 * 32.0);
                assert!(v.iter().all(|v| (0.0..=1.0).contains(v)));
            }
        }
    }
}
