//! Filter types shared by the voice filters and the FX filter.
//!
//! Serum 2 offers the types listed in `VoiceFilter.kParamType`'s options; each maps
//! to a Lumen filter model and a fidelity, so coverage reports stay honest.

use lumen_dsp::filter::FilterKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// Same response family and slope.
    Matched,
    /// Closest available response; character differs.
    Approximate,
    /// No implementation; the filter passes audio through.
    Missing,
}

/// Filter cutoff for the stored normalized frequency, exponential from MIDI note 0
/// (8.1758 Hz) to 22.05 kHz; matches Serum's displayed values (0.25 → 59 Hz,
/// 0.5 → 425 Hz, 0.75 → 3064 Hz). Shared by voice and FX filters.
pub fn cutoff_hz(n: f32) -> f32 {
    const LOW: f32 = 8.175_799;
    LOW * (22050.0 / LOW).powf(n.clamp(0.0, 1.0))
}

/// Lumen model for a stored filter type name.
pub fn model(name: &str) -> (Option<FilterKind>, Fidelity) {
    use Fidelity::*;
    use FilterKind::*;
    let poles = |n: &str| -> u8 { if n.ends_with("24") || n.ends_with("18") { 4 } else { 2 } };
    match name {
        // Moog-style ladders, tapped after 1–4 stages.
        "MgL6" => (Some(Ladder { poles: 1 }), Matched),
        "MgL12" => (Some(Ladder { poles: 2 }), Matched),
        "MgL18" => (Some(Ladder { poles: 3 }), Matched),
        "MgL24" => (Some(Ladder { poles: 4 }), Matched),
        // Plain multimode responses.
        "L12" | "L24" => (Some(Lowpass { poles: poles(name) }), Matched),
        "H12" | "H24" => (Some(Highpass { poles: poles(name) }), Matched),
        "B12" | "B24" => (Some(Bandpass { poles: poles(name) }), Matched),
        "N12" | "N24" => (Some(Notch), if name == "N12" { Matched } else { Approximate }),
        "P12" | "P24" => (Some(Peak), if name == "P12" { Matched } else { Approximate }),
        "L6" | "L18" => (Some(Lowpass { poles: poles(name) }), Approximate),
        "H6" | "H18" => (Some(Highpass { poles: poles(name) }), Approximate),
        // Morphing dual/triple types start at their first response.
        n if n.starts_with('L') && n[1..].chars().next().is_some_and(|c| "HBPN".contains(c)) => (Some(Lowpass { poles: poles(n) }), Approximate),
        n if n.starts_with('H') && n.len() == 4 && "BPN".contains(&n[1..2]) => (Some(Highpass { poles: 2 }), Approximate),
        n if n.starts_with("BP") || n.starts_with("BN") => (Some(Bandpass { poles: poles(n) }), Approximate),
        "PP12" | "PN12" => (Some(Peak), Approximate),
        "NN12" | "BandReject" => (Some(Notch), Approximate),
        n if n.starts_with("Ladder") || n.starts_with("DirtyMg") || n == "ZDF_A" => (Some(Ladder { poles: 4 }), Approximate),
        _ => (None, Missing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::ParamRef;

    #[test]
    fn every_serum_type_has_a_decision() {
        let options = ParamRef::new("VoiceFilter", 0, "kParamType").spec().options;
        assert!(options.len() >= 100, "{} types", options.len());
        let mut counts = [0; 3];
        for name in options {
            let (kind, fidelity) = model(name);
            assert_eq!(kind.is_none(), fidelity == Fidelity::Missing, "{name}");
            counts[fidelity as usize] += 1;
        }
        // Snapshot of current coverage; update as models are added.
        assert!(counts[0] >= 9, "{counts:?}");
    }
}
