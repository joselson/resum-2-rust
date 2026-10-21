//! Serum's reverb as measured metrics (`reverb_table`, from impulse responses per type):
//! each type's default setting plus per-control deltas, one control moved at a time,
//! and for controls that interact (Abyss, Vintage) the measured excess of pairs on a
//! grid over the sum of their single deltas. The metrics then drive Lumen's FDN reverb.

use super::reverb_table::{RATIO_RANGE, TYPES};
use super::FxParams;

/// Deltas from the type's default at one control value: decay times as natural-log
/// ratios, level and onset (ms) as differences, the low cut in Hz (Freq only).
pub struct Point {
    pub at: f32,
    pub t60: f32,
    pub low: f32,
    pub upper: f32,
    pub high: f32,
    pub level: f32,
    pub onset: f32,
    pub corr: f32,
    pub low_cut: f32,
}

pub struct Table {
    pub key: &'static str,
    pub points: &'static [Point],
}

/// Interaction of two controls: what a pair on the grid `a_at` × `b_at` measured beyond
/// the default plus both single-control deltas, row-major in `a` (`Point::at` unused).
/// Rows and columns at a control's default are zero.
pub struct Pair {
    pub a: &'static str,
    pub b: &'static str,
    pub a_at: &'static [f32],
    pub b_at: &'static [f32],
    pub cells: &'static [Point],
}

/// Metrics of a type's default setting: T60 at 500 Hz (s), the 125 Hz, 2 kHz and
/// 8 kHz T60s relative to it, level against the dry click (dB), onset (ms), tail L/R correlation.
pub struct TypeTable {
    pub kind: &'static str,
    pub t60: f32,
    pub low: f32,
    pub upper: f32,
    pub high: f32,
    pub level: f32,
    pub onset: f32,
    pub corr: f32,
    pub controls: &'static [Table],
    pub pairs: &'static [Pair],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub t60: f32,
    pub low: f32,
    pub upper: f32,
    pub high: f32,
    pub level: f32,
    pub onset_ms: f32,
    pub corr: f32,
    pub low_cut_hz: f32,
}

/// Predelay beyond the measured range keeps the measured slope (0.92 ms per ms).
const PREDELAY_SLOPE_MS: f32 = 919.0;

pub fn type_table(kind: &str) -> &'static TypeTable {
    TYPES.iter().find(|t| t.kind == kind).copied().unwrap_or(TYPES[TYPES.len() - 1])
}

/// Parameter indices of a type's controls and pairs, in table order.
pub struct Indices {
    controls: Vec<usize>,
    pairs: Vec<(usize, usize)>,
}

pub fn indices(table: &TypeTable) -> Indices {
    let spec = &crate::params::MODULES[crate::params::kind_of("FXReverb").expect("FXReverb")];
    let index = |key| spec.index_of(key).expect("reverb control");
    Indices {
        controls: table.controls.iter().map(|c| index(c.key)).collect(),
        pairs: table.pairs.iter().map(|q| (index(q.a), index(q.b))).collect(),
    }
}

pub fn evaluate(table: &TypeTable, indices: &Indices, p: &FxParams) -> Metrics {
    let mut m = Metrics { t60: table.t60, low: table.low, upper: table.upper, high: table.high, level: table.level, onset_ms: table.onset, corr: table.corr, low_cut_hz: 0.0 };
    let (mut t60, mut low, mut upper, mut high) = (0.0, 0.0, 0.0, 0.0);
    let mut add = |d: Point| {
        t60 += d.t60;
        low += d.low;
        upper += d.upper;
        high += d.high;
        m.level += d.level;
        m.onset_ms += d.onset;
        m.corr += d.corr;
        m.low_cut_hz = m.low_cut_hz.max(d.low_cut);
    };
    for (control, &index) in table.controls.iter().zip(&indices.controls) {
        let x = p.get(index);
        let mut d = interpolate(control.points, x);
        let last = &control.points[control.points.len() - 1];
        if control.key == "kParamPreDelay" && x > last.at {
            d.onset += PREDELAY_SLOPE_MS * (x - last.at);
        }
        add(d);
    }
    for (pair, &(a, b)) in table.pairs.iter().zip(&indices.pairs) {
        add(interpolate_pair(pair, p.get(a), p.get(b)));
    }
    m.t60 *= t60.exp();
    let ratio = |base: f32, d: f32, (lo, hi): (f32, f32)| (base * d.exp()).clamp(lo, hi);
    m.low = ratio(m.low, low, RATIO_RANGE[0]);
    m.upper = ratio(m.upper, upper, RATIO_RANGE[1]);
    m.high = ratio(m.high, high, RATIO_RANGE[2]);
    m.corr = m.corr.clamp(-1.0, 1.0);
    m.onset_ms = m.onset_ms.max(0.0);
    m
}

fn lerp(a: &Point, b: &Point, u: f32) -> Point {
    Point {
        at: a.at + (b.at - a.at) * u,
        t60: a.t60 + (b.t60 - a.t60) * u,
        low: a.low + (b.low - a.low) * u,
        upper: a.upper + (b.upper - a.upper) * u,
        high: a.high + (b.high - a.high) * u,
        level: a.level + (b.level - a.level) * u,
        onset: a.onset + (b.onset - a.onset) * u,
        corr: a.corr + (b.corr - a.corr) * u,
        low_cut: a.low_cut + (b.low_cut - a.low_cut) * u,
    }
}

/// The segment of sorted `at` values around x and the position in it, held at the ends.
fn locate(at: impl ExactSizeIterator<Item = f32> + Clone, x: f32) -> (usize, usize, f32) {
    let n = at.len();
    let i = at.clone().position(|a| a > x).unwrap_or(n);
    if i == 0 || i == n {
        let j = i.saturating_sub(1).min(n - 1);
        return (j, j, 0.0);
    }
    let (a, b) = (at.clone().nth(i - 1).unwrap(), at.clone().nth(i).unwrap());
    (i - 1, i, (x - a) / (b - a))
}

/// Linear interpolation between table points, held at the ends.
fn interpolate(points: &[Point], x: f32) -> Point {
    let (i, j, u) = locate(points.iter().map(|q| q.at), x);
    lerp(&points[i], &points[j], u)
}

/// Bilinear interpolation on a pair's grid, held at the edges.
fn interpolate_pair(pair: &Pair, a: f32, b: f32) -> Point {
    let (i0, i1, u) = locate(pair.a_at.iter().copied(), a);
    let (j0, j1, v) = locate(pair.b_at.iter().copied(), b);
    let cell = |i: usize, j: usize| &pair.cells[i * pair.b_at.len() + j];
    lerp(&lerp(cell(i0, j0), cell(i0, j1), v), &lerp(cell(i1, j0), cell(i1, j1), v), u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{kind_of, MODULES};

    #[test]
    fn unset_type_is_unnamed_and_controls_move_the_metrics() {
        let spec = &MODULES[kind_of("FXReverb").unwrap()];
        let mut values: Vec<f32> = spec.params.iter().map(|p| p.default_for(0)).collect();
        let offsets = vec![0.0; values.len()];
        let eval = |values: &[f32]| {
            let p = FxParams { spec, values, offsets: &offsets };
            let table = type_table(p.choice(spec.index_of("kParamType").unwrap()));
            (table.kind, evaluate(table, &indices(table), &p))
        };
        // A preset without kParamType renders as the unnamed type, at its measured defaults.
        let (kind, m) = eval(&values);
        assert_eq!(kind, "kUnnamed");
        let base = type_table("kUnnamed");
        assert!((m.t60 - base.t60).abs() < 1e-4 && (m.onset_ms - base.onset).abs() < 1e-3, "{m:?}");
        // Hall's Delay sets its decay (fxrv6 probes: ≈ 2.6 s at the default, ≈ 15 s at 100).
        values[spec.index_of("kParamType").unwrap()] = 0.0;
        let hall = eval(&values).1.t60;
        values[spec.index_of("kParamDelay").unwrap()] = 100.0;
        let long = eval(&values).1.t60;
        assert!(long > 4.0 * hall, "{hall} → {long}");
    }
}
