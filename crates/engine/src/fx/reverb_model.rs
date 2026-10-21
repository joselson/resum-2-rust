//! Serum's reverb as measured metrics (`reverb_table`, from impulse responses per type):
//! each type's default setting plus per-control deltas, one control moved at a time
//! and combined here as independent. The metrics then drive Lumen's FDN reverb.

use super::reverb_table::TYPES;
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

/// Parameter indices of a type's controls, in table order.
pub fn control_indices(table: &TypeTable) -> Vec<usize> {
    let spec = &crate::params::MODULES[crate::params::kind_of("FXReverb").expect("FXReverb")];
    table.controls.iter().map(|c| spec.index_of(c.key).expect("reverb control")).collect()
}

pub fn evaluate(table: &TypeTable, indices: &[usize], p: &FxParams) -> Metrics {
    let mut m = Metrics { t60: table.t60, low: table.low, upper: table.upper, high: table.high, level: table.level, onset_ms: table.onset, corr: table.corr, low_cut_hz: 0.0 };
    let (mut t60, mut low, mut upper, mut high) = (0.0, 0.0, 0.0, 0.0);
    for (control, &index) in table.controls.iter().zip(indices) {
        let x = p.get(index);
        let d = interpolate(control.points, x);
        t60 += d.t60;
        low += d.low;
        upper += d.upper;
        high += d.high;
        m.level += d.level;
        m.onset_ms += d.onset;
        m.corr += d.corr;
        m.low_cut_hz = m.low_cut_hz.max(d.low_cut);
        let last = &control.points[control.points.len() - 1];
        if control.key == "kParamPreDelay" && x > last.at {
            m.onset_ms += PREDELAY_SLOPE_MS * (x - last.at);
        }
    }
    m.t60 *= t60.exp();
    m.low *= low.exp();
    m.upper *= upper.exp();
    m.high *= high.exp();
    m.corr = m.corr.clamp(-1.0, 1.0);
    m.onset_ms = m.onset_ms.max(0.0);
    m
}

/// Linear interpolation between table points, held at the ends.
fn interpolate(points: &[Point], x: f32) -> Point {
    let lerp = |a: &Point, b: &Point, u: f32| Point {
        at: x,
        t60: a.t60 + (b.t60 - a.t60) * u,
        low: a.low + (b.low - a.low) * u,
        upper: a.upper + (b.upper - a.upper) * u,
        high: a.high + (b.high - a.high) * u,
        level: a.level + (b.level - a.level) * u,
        onset: a.onset + (b.onset - a.onset) * u,
        corr: a.corr + (b.corr - a.corr) * u,
        low_cut: a.low_cut + (b.low_cut - a.low_cut) * u,
    };
    let i = points.iter().position(|q| q.at > x).unwrap_or(points.len());
    if i == 0 {
        return lerp(&points[0], &points[0], 0.0);
    }
    if i == points.len() {
        return lerp(&points[i - 1], &points[i - 1], 0.0);
    }
    let (a, b) = (&points[i - 1], &points[i]);
    lerp(a, b, (x - a.at) / (b.at - a.at))
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
            (table.kind, evaluate(table, &control_indices(table), &p))
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
