//! Static waveshapers for distortion.

use std::f32::consts::{FRAC_PI_2, PI};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shaper {
    Tube,
    SoftClip,
    HardClip,
    Diode1,
    Diode2,
    LinearFold,
    SineFold,
    ZeroSquare,
    /// Handled as sample-rate reduction by the caller; passes through here.
    Downsample,
    Asym,
    Rectify,
    XShaper,
    XShaperAsym,
    SineShaper,
    StompBox,
    TapeSat,
    Overdrive,
    SoftSat,
}

/// Applies the shaper to a pre-gained sample.
#[inline]
pub fn shape(s: Shaper, x: f32) -> f32 {
    match s {
        // Asymmetric soft saturation: positive half saturates harder.
        Shaper::Tube => if x >= 0.0 { x.tanh() } else { (0.7 * x).tanh() / 0.7f32.tanh() * 0.95 },
        Shaper::SoftClip => x.tanh(),
        Shaper::HardClip => x.clamp(-1.0, 1.0),
        Shaper::Diode1 => if x >= 0.0 { 1.0 - (-x).exp() } else { -0.3 * (1.0 - (x * 3.0).exp()) },
        Shaper::Diode2 => if x >= 0.0 { 1.0 - (-x).exp() } else { 0.0 },
        Shaper::LinearFold => {
            // Triangle fold: reflect at ±1.
            let t = (x + 1.0).rem_euclid(4.0);
            if t < 2.0 { t - 1.0 } else { 3.0 - t }
        }
        Shaper::SineFold => (x * FRAC_PI_2).sin(),
        Shaper::ZeroSquare => (x * x.abs()).clamp(-1.0, 1.0),
        Shaper::Downsample => x,
        Shaper::Asym => (x + 0.25 * x * x).tanh(),
        Shaper::Rectify => x.abs().min(1.0) * 2.0 - 1.0,
        Shaper::XShaper => {
            let c = x.clamp(-1.5, 1.5);
            c - c * c * c / 6.75 * 4.0 / 3.0 * 0.75
        }
        Shaper::XShaperAsym => {
            let c = (x + 0.2).clamp(-1.5, 1.5);
            c - c * c * c / 6.75
        }
        Shaper::SineShaper => (x.clamp(-1.0, 1.0) * PI).sin() * 0.5 + x.tanh() * 0.5,
        Shaper::StompBox => (x * 1.5).tanh() * 0.9 + 0.1 * (x * 4.0).tanh(),
        Shaper::TapeSat => x / (1.0 + x * x).sqrt(),
        Shaper::Overdrive => (2.0 / PI) * (x * 1.2).atan() * 1.1,
        Shaper::SoftSat => x / (1.0 + x.abs()),
    }
}

impl Shaper {
    pub fn from_name(name: &str) -> Option<Shaper> {
        Some(match name {
            "kTube" => Shaper::Tube,
            "kSoftClip" => Shaper::SoftClip,
            "kHardClip" => Shaper::HardClip,
            "kDiode1" => Shaper::Diode1,
            "kDiode2" => Shaper::Diode2,
            "kLinFold" => Shaper::LinearFold,
            "kSinFold" => Shaper::SineFold,
            "kZeroSquare" => Shaper::ZeroSquare,
            "kDownsample" => Shaper::Downsample,
            "kAsym" => Shaper::Asym,
            "kRectify" => Shaper::Rectify,
            "kXShaper" => Shaper::XShaper,
            "kXShaperAsym" => Shaper::XShaperAsym,
            "kSineShaper" => Shaper::SineShaper,
            "kStompBox" => Shaper::StompBox,
            "kTapeSat" => Shaper::TapeSat,
            "kOverdrive" => Shaper::Overdrive,
            "kSoftSat" => Shaper::SoftSat,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapers_are_bounded_and_finite() {
        let all = [Shaper::Tube, Shaper::SoftClip, Shaper::HardClip, Shaper::Diode1, Shaper::Diode2, Shaper::LinearFold, Shaper::SineFold, Shaper::ZeroSquare, Shaper::Downsample, Shaper::Asym, Shaper::Rectify, Shaper::XShaper, Shaper::XShaperAsym, Shaper::SineShaper, Shaper::StompBox, Shaper::TapeSat, Shaper::Overdrive, Shaper::SoftSat];
        for s in all {
            for i in -400..=400 {
                let x = i as f32 / 20.0;
                let y = shape(s, x);
                assert!(y.is_finite(), "{s:?} {x}");
                if s != Shaper::Downsample {
                    assert!(y.abs() <= 1.6, "{s:?} {x} -> {y}");
                }
            }
        }
        assert!((shape(Shaper::LinearFold, 1.5) - 0.5).abs() < 1e-6);
    }
}
