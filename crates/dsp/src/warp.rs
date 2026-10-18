//! Oscillator warps: phase, sample, harmonic and cross-modulation transforms
//! applied around a table read. Two slots run in series per oscillator.
//!
//! Implementations are generic designs; their curves are calibrated against
//! render probes (docs/CALIBRATION.md).

use crate::shaper::{self, Shaper};
use std::f32::consts::TAU;

/// Modulation sources a cross-mod warp can read (previous-sample values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpSource {
    /// First other oscillator (for A: B).
    Osc,
    /// Second other oscillator (for A: C).
    Osc2,
    Noise,
    Sub,
    Filter1,
    Filter2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WarpMode {
    Off,
    Sync,
    BendPos,
    BendNeg,
    BendPosNeg,
    Pwm,
    AsymPos,
    AsymNeg,
    AsymPosNeg,
    Flip,
    Mirror,
    Remap(u8),
    Quantize,
    OddEven,
    LowPass,
    HighPass,
    Distort(Shaper),
    /// Phase modulation.
    Fm(WarpSource),
    /// Exponential frequency modulation.
    FmExp(WarpSource),
    /// Linear (through-zero) frequency modulation.
    FmLin(WarpSource),
    PhaseDistort(WarpSource),
    SelfPhaseDistort,
    Am(WarpSource),
    Rm(WarpSource),
}

impl WarpMode {
    pub fn from_name(name: &str) -> Option<WarpMode> {
        use WarpMode::*;
        let source = |s: &str| -> Option<WarpSource> {
            Some(match s {
                "OSC" => WarpSource::Osc,
                "OSC2" => WarpSource::Osc2,
                "NOISE" => WarpSource::Noise,
                "SUB" => WarpSource::Sub,
                "FILT1" => WarpSource::Filter1,
                "FILT2" => WarpSource::Filter2,
                _ => return None,
            })
        };
        Some(match name {
            "" | "kNoWarp" => Off,
            "kSync" => Sync,
            "kBendPos" => BendPos,
            "kBendNeg" => BendNeg,
            "kBendPosNeg" => BendPosNeg,
            "kPWM" => Pwm,
            "kASYMPos" => AsymPos,
            "kASYMNeg" => AsymNeg,
            "kASYMPosNeg" => AsymPosNeg,
            "kFlip" => Flip,
            "kDLM" => Mirror,
            "kRemap_1" => Remap(1),
            "kRemap_2" => Remap(2),
            "kRemap_3" => Remap(3),
            "kRemap_4" => Remap(4),
            "kQuantize" => Quantize,
            "kEvenOdd" => OddEven,
            "kFilterLPF" => LowPass,
            "kFilterHPF" => HighPass,
            "kSelfPD" => SelfPhaseDistort,
            n => {
                if let Some(d) = n.strip_prefix("kDist") {
                    return Shaper::from_name(&format!("k{d}")).map(Distort);
                }
                let (kind, src) = n.strip_prefix('k')?.split_once('_')?;
                let src = source(src)?;
                match kind {
                    "FM" => Fm(src),
                    "FMX" => FmExp(src),
                    "FMP" => FmLin(src),
                    "PD" => PhaseDistort(src),
                    "AM" => Am(src),
                    "RM" => Rm(src),
                    _ => return None,
                }
            }
        })
    }

    pub fn source(self) -> Option<WarpSource> {
        use WarpMode::*;
        match self {
            Fm(s) | FmExp(s) | FmLin(s) | PhaseDistort(s) | Am(s) | Rm(s) => Some(s),
            _ => None,
        }
    }

    /// Warps that change the phase increment rather than the read position.
    pub fn is_frequency(self) -> bool {
        matches!(self, WarpMode::FmExp(_) | WarpMode::FmLin(_))
    }

    /// Warps that need a second table read half a cycle away.
    pub fn needs_half_read(self) -> bool {
        matches!(self, WarpMode::OddEven)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WarpSlot {
    pub mode: WarpMode,
    /// [0, 1]
    pub amount: f32,
    /// [0, 1]
    pub var: f32,
}

impl Default for WarpSlot {
    fn default() -> Self {
        WarpSlot { mode: WarpMode::Off, amount: 0.0, var: 0.0 }
    }
}

/// Per-voice state for stateful warps (filters, self-PD feedback).
#[derive(Clone, Copy, Debug, Default)]
pub struct WarpState {
    lp: f32,
    prev: f32,
}

/// Pitch-relative filter coefficient: the cutoff sits `octaves` above the fundamental.
#[inline]
fn one_pole(increment: f32, octaves: f32) -> f32 {
    let hz_over_sr = (increment * octaves.exp2()).min(0.45);
    1.0 - (-TAU * hz_over_sr).exp()
}

#[inline]
fn bend(phase: f32, amount: f32) -> f32 {
    // Power curve toward the start; measured gentler than ×8 at full amount.
    phase.powf(1.0 / (1.0 + amount * 2.0))
}

impl WarpSlot {
    /// Read position for `phase` ∈ [0, 1) and an output polarity (±1).
    /// `m` is the modulator sample in [-1, 1].
    #[inline]
    pub fn phase(&self, phase: f32, m: f32, state: &WarpState) -> (f32, f32) {
        use WarpMode::*;
        let a = self.amount;
        if a <= 0.0 && !matches!(self.mode, Fm(_) | PhaseDistort(_)) {
            return (phase, 1.0);
        }
        let mut sign = 1.0;
        let p = match self.mode {
            // Sync ratio ≈ 3 at 0.5 and ≈ 17 at 1 (measured spectral peaks).
            Sync => phase * (1.0 + 2.0 * a + 14.0 * a.powi(4)),
            BendPos => bend(phase, a),
            BendNeg => 1.0 - bend(1.0 - phase, a),
            BendPosNeg => {
                if phase < 0.5 { 0.5 * bend(phase * 2.0, a) } else { 0.5 + 0.5 * (1.0 - bend(2.0 - phase * 2.0, a)) }
            }
            Pwm => {
                // Outside the window the output is silent; it closes fully at 1 (measured).
                let width = 1.0 - a;
                if width > 1e-4 && phase < width {
                    phase / width
                } else {
                    sign = 0.0;
                    0.0
                }
            }
            AsymPos | AsymNeg | AsymPosNeg => {
                // Move the cycle's midpoint; +/- swaps direction every half.
                let mid = match self.mode {
                    AsymPos => 0.5 - 0.49 * a,
                    AsymNeg => 0.5 + 0.49 * a,
                    _ => 0.5 - 0.49 * a * (TAU * phase).sin().signum(),
                };
                if phase < mid { 0.5 * phase / mid } else { 0.5 + 0.5 * (phase - mid) / (1.0 - mid) }
            }
            // Time reversal with an offset: the magnitude spectrum is unchanged (measured).
            Flip => a - phase,
            Mirror => {
                // The cycle plays compressed before the split point and negated after it
                // (measured at 0.5: odd harmonics only).
                let split = a.clamp(0.01, 0.99);
                if phase < split {
                    phase / split
                } else {
                    sign = -1.0;
                    (phase - split) / (1.0 - split)
                }
            }
            // Remap k repeats the cycle k times (measured: only every k-th harmonic).
            Remap(k) if k > 1 => phase * (1.0 + (k as f32 - 1.0) * (a * 2.0).min(1.0)),
            // Phase quantisation: steps per cycle shrink to 2 at full amount (measured).
            Quantize => {
                let steps = (2f32).powf(1.0 + (1.0 - a) * 10.0);
                (phase * steps).floor() / steps
            }
            Fm(_) => phase + m * a * 2.0,
            PhaseDistort(_) => phase + a * (0.5 + 0.5 * m) * 0.5 * (1.0 - (TAU * phase).cos()),
            SelfPhaseDistort => phase + a * state.prev * 0.5,
            _ => phase,
        };
        (p.rem_euclid(1.0), sign)
    }

    /// Phase increment multiplier for frequency-modulating warps.
    #[inline]
    pub fn increment_scale(&self, m: f32) -> f32 {
        match self.mode {
            WarpMode::FmExp(_) => (m * self.amount * 4.0).exp2(),
            WarpMode::FmLin(_) => 1.0 + m * self.amount * 8.0,
            _ => 1.0,
        }
    }

    /// Output transform. `x_half` is the table value half a cycle later (odd/even),
    /// `increment` the voice's phase increment.
    #[inline]
    pub fn sample(&self, x: f32, x_half: f32, m: f32, increment: f32, state: &mut WarpState) -> f32 {
        use WarpMode::*;
        let a = self.amount;
        let y = match self.mode {
            // Remap 1 is a gentle amplitude curve (matches Serum within 2 dB/band).
            Remap(1) => x + (x.signum() * x.abs().powf(1.0 / (1.0 + 3.0 * a)) - x) * a.clamp(0.0, 1.0).sqrt(),
            OddEven => {
                // Below 0.5 fades toward odd harmonics only, above toward even only; the
                // isolated part keeps full level (measured: even-only is not 6 dB down).
                let bal = a * 2.0 - 1.0;
                let odd = x - x_half;
                let even = x + x_half;
                if bal < 0.0 { x + (odd - x) * -bal } else { x + (even - x) * bal }
            }
            // Amount is the cutoff: 6 octaves above the note at 1 (measured: untouched).
            LowPass => {
                let c = one_pole(increment, a * 6.0);
                state.lp += (x - state.lp) * c;
                state.lp
            }
            // Cutoff 2 octaves above the note at 0.5 (−8 dB, measured), rising steeply
            // toward 1 (measured near-silent).
            HighPass => {
                let c = one_pole(increment, 2.0 * (2.0 * a).powi(3));
                state.lp += (x - state.lp) * c;
                x - state.lp
            }
            // Drive 0–36 dB, as in the FX distortion (folds measured much brighter than 18 dB).
            Distort(s) => shaper::shape(s, x * 10f32.powf(a * 36.0 / 20.0)),
            Am(_) => x * (1.0 - a + a * (0.5 + 0.5 * m)),
            Rm(_) => x + (x * m - x) * a,
            _ => x,
        };
        state.prev = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_serum_mode_name_parses() {
        let names = ["kSync", "kBendPos", "kBendNeg", "kBendPosNeg", "kPWM", "kASYMPos", "kASYMNeg", "kASYMPosNeg", "kFlip", "kDLM", "kRemap_1", "kRemap_2", "kRemap_3", "kRemap_4", "kQuantize", "kEvenOdd", "kFilterLPF", "kFilterHPF", "kDistTube", "kDistSoftClip", "kDistHardClip", "kDistDiode1", "kDistDiode2", "kDistLinFold", "kDistSinFold", "kDistZeroSquare", "kDistAsym", "kDistRectify", "kDistSineShaper", "kDistStompBox", "kDistTapeSat", "kDistSoftSat", "kFM_OSC", "kFM_OSC2", "kFM_NOISE", "kFM_SUB", "kFM_FILT1", "kFM_FILT2", "kFMX_OSC", "kFMX_FILT2", "kFMP_SUB", "kPD_NOISE", "kSelfPD", "kAM_OSC", "kRM_FILT1"];
        for n in names {
            assert!(WarpMode::from_name(n).is_some(), "{n}");
        }
        assert_eq!(WarpMode::from_name("kFM_OSC2"), Some(WarpMode::Fm(WarpSource::Osc2)));
        assert_eq!(WarpMode::from_name("kNope"), None);
    }

    #[test]
    fn zero_amount_is_identity_for_phase_and_shape_warps() {
        let mut st = WarpState::default();
        for name in ["kSync", "kBendPos", "kPWM", "kASYMPosNeg", "kQuantize", "kRemap_2"] {
            let slot = WarpSlot { mode: WarpMode::from_name(name).unwrap(), amount: 0.0, var: 0.0 };
            for i in 0..16 {
                let p = i as f32 / 16.0;
                assert!((slot.phase(p, 0.0, &st).0 - p).abs() < 1e-5, "{name} phase");
                let x = (TAU * p).sin() * 0.7;
                let y = slot.sample(x, 0.0, 0.0, 0.01, &mut st);
                assert!((y - x).abs() < 0.05, "{name} sample {x} -> {y}");
            }
        }
    }

    #[test]
    fn remap_repeats_the_cycle_and_mirror_negates_the_second_part() {
        let st = WarpState::default();
        let remap3 = WarpSlot { mode: WarpMode::Remap(3), amount: 0.5, var: 0.0 };
        assert!((remap3.phase(0.4, 0.0, &st).0 - 0.2).abs() < 1e-5);
        let mirror = WarpSlot { mode: WarpMode::Mirror, amount: 0.5, var: 0.0 };
        assert_eq!(mirror.phase(0.25, 0.0, &st), (0.5, 1.0));
        assert_eq!(mirror.phase(0.75, 0.0, &st), (0.5, -1.0));
        let pwm = WarpSlot { mode: WarpMode::Pwm, amount: 1.0, var: 0.0 };
        assert_eq!(pwm.phase(0.3, 0.0, &st), (0.0, 0.0), "fully closed window is silent");
    }

    #[test]
    fn odd_even_isolates_harmonics() {
        // Saw-like: fundamental + 2nd harmonic. Odd-only removes the 2nd.
        let f = |p: f32| (TAU * p).sin() + 0.5 * (2.0 * TAU * p).sin();
        let slot = WarpSlot { mode: WarpMode::OddEven, amount: 0.0, var: 0.0 };
        let mut st = WarpState::default();
        let p = 0.125;
        let y = slot.sample(f(p), f(p + 0.5), 0.0, 0.01, &mut st);
        assert!((y - 2.0 * (TAU * p).sin()).abs() < 1e-5);
    }
}
