//! Filter types shared by the voice filters and the FX filter.
//!
//! Serum 2 offers the types listed in `VoiceFilter.kParamType`'s options; each maps
//! to a Lumen filter model and a fidelity, so coverage reports stay honest.

pub use lumen_dsp::filter::cutoff_hz;
use lumen_dsp::filter::{Damping, FilterKind, NamedLadder as Named, Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// Same response family and slope.
    Matched,
    /// Closest available response; character differs.
    Approximate,
    /// No implementation; the filter passes audio through.
    Missing,
}

/// Lumen model for a stored filter type name.
pub fn model(name: &str) -> (Option<FilterKind>, Fidelity) {
    use Fidelity::*;
    use FilterKind::*;
    let poles = |n: &str| -> u8 {
        match &n[n.len() - 2..] {
            "24" => 4,
            "18" => 3,
            _ if n.ends_with('6') => 1,
            _ => 2,
        }
    };
    match name {
        // Moog-style ladders, tapped after 1–4 stages.
        "MgL6" => (Some(Ladder { poles: 1 }), Matched),
        "MgL12" => (Some(Ladder { poles: 2 }), Matched),
        "MgL18" => (Some(Ladder { poles: 3 }), Matched),
        "MgL24" => (Some(Ladder { poles: 4 }), Matched),
        // Plain multimode responses.
        // Plain multimode responses, fitted to measurements.
        "L6" | "L12" | "L18" | "L24" => (Some(Lowpass { poles: poles(name) }), Matched),
        "H6" | "H12" | "H18" | "H24" => (Some(Highpass { poles: poles(name) }), Matched),
        "B12" | "B24" => (Some(Bandpass { poles: poles(name) }), Matched),
        "N12" | "N24" => (Some(Notch { poles: poles(name) }), Matched),
        "P12" | "P24" => (Some(Peak { poles: poles(name) }), Matched),
        // Dual (two letters) and triple (three letters) morph types, fitted on LH12, HP12,
        // L/B/H 24, L/N/H 24 and B/P/N 12; the others follow the same structure.
        n if morph_letters(n).is_some() => {
            let letters = morph_letters(n).unwrap();
            let fitted = matches!(n, "LH12" | "HP12" | "LBH24" | "LNH24" | "BPN12" | "LBH12");
            let fidelity = if fitted { Matched } else { Approximate };
            match letters.as_slice() {
                [a, b] => (Some(Dual { first: *a, second: *b, poles: if n.ends_with('6') { 1 } else { 2 } }), fidelity),
                [a, b, c] => (Some(Morph { responses: [*a, *b, *c], poles: if n.ends_with("24") { 4 } else { 2 } }), fidelity),
                _ => unreachable!(),
            }
        }
        "BandReject" => (Some(Notch { poles: 2 }), Approximate),
        "LadderMg" => (Some(CompensatedLadder), Matched),
        "Exp" => (Some(Exp { band: false }), Matched),
        "ExpBPF" => (Some(Exp { band: true }), Matched),
        "Wsp" => (Some(Whisper), Matched),
        "Scream" => (Some(Scream { band: false, gain_db: -12 }), Matched),
        "Scream3LP" => (Some(Scream { band: false, gain_db: -13 }), Approximate),
        "Scream3BP" => (Some(Scream { band: true, gain_db: -14 }), Approximate),
        "DistComb1LP" => (Some(Scream { band: false, gain_db: -9 }), Approximate),
        "DistComb1BP" => (Some(Scream { band: true, gain_db: -10 }), Approximate),
        // Delay-based families (fitted; damped variants and negative flange/phase approximate).
        n if n.starts_with("Comb") && (n.ends_with('P') || n.ends_with('N')) => {
            let damping = if n.contains("HL6") { Damping::Both } else if n.contains("L6") { Damping::Low } else if n.contains("H6") { Damping::High } else { Damping::None };
            let kind = Comb { flange: false, negative: n.ends_with('N'), damping };
            (Some(kind), if damping == Damping::None { Matched } else { Approximate })
        }
        n if n.starts_with("Flange") && !n.starts_with("FlangePhase") => {
            let damping = if n.contains("HL6") { Damping::Both } else if n.contains("L6") { Damping::Low } else if n.contains("H6") { Damping::High } else { Damping::None };
            (Some(Comb { flange: true, negative: n.ends_with('N'), damping }), if n == "FlangeP" { Matched } else { Approximate })
        }
        n if n.starts_with("Phase") => {
            let stages = n[5..7].parse::<u8>().map_or(8, |p| p / 6);
            let plain = n.len() == 8;
            (Some(Phaser { stages, negative: n.ends_with('N') }), if plain && n.ends_with('P') { Matched } else { Approximate })
        }
        n if n.starts_with("FlangePhase") => (Some(Phaser { stages: 2, negative: n.ends_with('N') }), Approximate),
        // EMS, Acid and Dirty ladders are nonlinear designs of their own.
        // Nonlinear designs of their own; only their drive stages are measured.
        "LadderEMS" => (Some(NamedLadder(Named::Ems)), Approximate),
        "LadderAcid" => (Some(NamedLadder(Named::Acid)), Approximate),
        "DirtyMg" => (Some(NamedLadder(Named::Dirty)), Approximate),
        // German LP measures as a 4-pole lowpass about 3 dB down; the ladder is the nearest model.
        "ZDF_A" => (Some(Ladder { poles: 4 }), Approximate),
        _ => (None, Missing),
    }
}

/// The responses named by a morph type ("LH12" → low, high), if it is one.
fn morph_letters(name: &str) -> Option<Vec<Response>> {
    let letters: String = name.chars().take_while(|c| c.is_ascii_uppercase()).collect();
    let digits = &name[letters.len()..];
    if !(2..=3).contains(&letters.len()) || !matches!(digits, "6" | "12" | "24") {
        return None;
    }
    letters
        .chars()
        .map(|c| match c {
            'L' => Some(Response::Low),
            'H' => Some(Response::High),
            'B' => Some(Response::Band),
            'P' => Some(Response::Peak),
            'N' => Some(Response::Notch),
            _ => None,
        })
        .collect()
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
        assert!(counts[0] >= 26, "{counts:?}");
        assert_eq!(model("Phase24P").0, Some(FilterKind::Phaser { stages: 4, negative: false }));
        assert_eq!(model("Phase48HL6N").0, Some(FilterKind::Phaser { stages: 8, negative: true }));
        assert_eq!(model("CombHL6N").0, Some(FilterKind::Comb { flange: false, negative: true, damping: Damping::Both }));
        assert_eq!(model("FlangeL6P").0, Some(FilterKind::Comb { flange: true, negative: false, damping: Damping::Low }));
        assert!(model("Comb2").0.is_none() && model("Combs").0.is_none());
        assert_eq!(model("LH6").0, Some(FilterKind::Dual { first: Response::Low, second: Response::High, poles: 1 }));
        assert_eq!(model("BPN24").0, Some(FilterKind::Morph { responses: [Response::Band, Response::Peak, Response::Notch], poles: 4 }));
        assert_eq!(model("L12").0, Some(FilterKind::Lowpass { poles: 2 }), "plain types are not morphs");
    }
}
