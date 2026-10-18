//! Mod slot sources, destinations and per-voice evaluation.
//!
//! Source IDs index Serum 2's mod-source name table plus one (0 = none):
//! 1 Mod Wheel, 2–5 Env 1–4, 6–15 LFO 1–10, 16 Velo, 17 Note#, 18 Aftertouch,
//! 19 Poly Aftertouch, 20 Noise OSC, 21–22 NoteOn Rand 1–2, 23–24 NoteOn Alt 1–2,
//! 25–32 Macro 1–8, 33 Pitch Bend, 34–36 MPE X/Y/Z, 37 Release Velo, 38 Fixed,
//! 39–48 LFO 1–10 Y, 49–51 OSC A–C, 52 Sub, 53–54 Filter 1–2, 55 Active Voices,
//! 56–57 Voice Mod 1–2, 58 Voice Index, 59 NoteOn Rand (Discrete).
//!
//! Any module parameter can be a destination; the slot's amount, bipolar, aux
//! and bypass settings are ModSlot module parameters.

use crate::params::{self, ParamRef, Values, TOTAL};
use lumen_preset::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    ModWheel,
    Env(usize),
    Lfo(usize),
    Velocity,
    Note,
    Aftertouch,
    NoteOnRandom(usize),
    NoteOnAlternate(usize),
    Macro(usize),
    PitchBend,
    ReleaseVelocity,
    Fixed,
}

impl Source {
    /// Maps a stored source ID; `Err(id)` for sources Lumen does not model yet.
    pub fn from_id(id: i64) -> Result<Option<Source>, i64> {
        Ok(Some(match id {
            0 => return Ok(None),
            1 => Source::ModWheel,
            2..=5 => Source::Env((id - 2) as usize),
            6..=15 => Source::Lfo((id - 6) as usize),
            16 => Source::Velocity,
            17 => Source::Note,
            // Poly aftertouch falls back to channel aftertouch.
            18 | 19 => Source::Aftertouch,
            21 | 22 => Source::NoteOnRandom((id - 21) as usize),
            23 | 24 => Source::NoteOnAlternate((id - 23) as usize),
            25..=32 => Source::Macro((id - 25) as usize),
            33 => Source::PitchBend,
            37 => Source::ReleaseVelocity,
            38 => Source::Fixed,
            _ => return Err(id),
        }))
    }

    /// Bipolar sources natively span [-1, 1]; others span [0, 1].
    pub fn is_bipolar(self) -> bool {
        matches!(self, Source::PitchBend)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub source: Source,
    pub aux: Option<Source>,
    /// The ModSlot instance holding amount/bipolar/aux-invert.
    pub slot: usize,
    pub target: ParamRef,
}

/// Why a stored mod slot is not routed.
#[derive(Clone, Debug, PartialEq)]
pub enum Skipped {
    Empty,
    UnsupportedSource(i64),
    UnknownTarget(String),
}

const AMOUNT: ParamRef = ParamRef::new("ModSlot", 0, "kParamAmount");
const BIPOLAR: ParamRef = ParamRef::new("ModSlot", 0, "kParamBipolar");
const BYPASS: ParamRef = ParamRef::new("ModSlot", 0, "kParamBypass");
const AUX_INVERTED: ParamRef = ParamRef::new("ModSlot", 0, "kParamAuxInverted");

/// Resolves a slot's source and destination. Amount and flags stay in `Values`.
pub fn parse_slot(slot_index: usize, slot: &Value) -> Result<Route, Skipped> {
    let pair = slot.get("source").and_then(Value::as_array).ok_or(Skipped::Empty)?;
    let id = |i: usize| pair.get(i).and_then(Value::as_f64).map(|v| v as i64).unwrap_or(0);
    let source = Source::from_id(id(0)).map_err(Skipped::UnsupportedSource)?.ok_or(Skipped::Empty)?;
    let aux = Source::from_id(id(1)).map_err(Skipped::UnsupportedSource)?;
    let module = slot.get("destModuleTypeString").and_then(Value::as_str).ok_or(Skipped::Empty)?;
    let instance = slot.get("destModuleID").and_then(Value::as_f64).unwrap_or(0.0) as usize;
    let param = slot.get("destModuleParamName").and_then(Value::as_str).unwrap_or("");
    let unknown = || Skipped::UnknownTarget(format!("{module}.{param}"));
    let kind = params::kind_of(module).ok_or_else(unknown)?;
    let spec = params::MODULES[kind];
    let index = spec.index_of(param).ok_or_else(unknown)?;
    if !spec.is_dynamic() && instance >= spec.instances {
        return Err(unknown());
    }
    Ok(Route { source, aux, slot: slot_index, target: ParamRef { kind, instance, index } })
}

/// Stored destination ID disagreeing with the parameter table (a layout error, not a preset error).
pub fn id_mismatch(slot: &Value, route: &Route) -> Option<(i64, usize)> {
    let stored = slot.get("destModuleParamID").and_then(Value::as_f64)? as i64;
    (stored != route.target.index as i64).then_some((stored, route.target.index))
}

/// Current source values for one voice.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub mod_wheel: f32,
    pub envs: [f32; 4],
    pub lfos: [f32; 10],
    pub velocity: f32,
    pub note: f32,
    pub aftertouch: f32,
    pub random: [f32; 2],
    pub alternate: [f32; 2],
    pub macros: [f32; 8],
    pub pitch_bend: f32,
    pub release_velocity: f32,
}

impl Sources {
    pub fn value(&self, s: Source) -> f32 {
        match s {
            Source::ModWheel => self.mod_wheel,
            Source::Env(i) => self.envs[i],
            Source::Lfo(i) => self.lfos[i],
            Source::Velocity => self.velocity,
            Source::Note => self.note,
            Source::Aftertouch => self.aftertouch,
            Source::NoteOnRandom(i) => self.random[i],
            Source::NoteOnAlternate(i) => self.alternate[i],
            Source::Macro(i) => self.macros[i],
            Source::PitchBend => self.pitch_bend,
            Source::ReleaseVelocity => self.release_velocity,
            Source::Fixed => 1.0,
        }
    }
}

/// Normalized offsets for every parameter, cleared only where routes write.
/// FX targets (dynamic modules) are collected separately in `fx`.
#[derive(Clone)]
pub struct Offsets {
    values: Box<[f32; TOTAL]>,
    touched: Vec<usize>,
    /// (module instance id, parameter index, offset) for FX modules.
    pub fx: Vec<(usize, usize, f32)>,
}

impl Default for Offsets {
    fn default() -> Self {
        Offsets { values: Box::new([0.0; TOTAL]), touched: Vec::with_capacity(64), fx: Vec::with_capacity(32) }
    }
}

impl Offsets {
    #[inline]
    pub fn get(&self, p: ParamRef) -> f32 {
        self.values[p.flat()]
    }

    /// Plain value of `p` with this voice's modulation applied.
    #[inline]
    pub fn apply(&self, values: &Values, p: ParamRef) -> f32 {
        p.spec().offset(values.get(p), self.get(p))
    }

    pub fn clear(&mut self) {
        for &i in &self.touched {
            self.values[i] = 0.0;
        }
        self.touched.clear();
        self.fx.clear();
    }

    /// Sums every route into offsets. Amount, bipolar and bypass are read through
    /// `slot_value` so mod slot settings can themselves be live or modulated.
    pub fn evaluate(&mut self, routes: &[Route], sources: &Sources, values: &Values) {
        self.clear();
        for r in routes {
            if values.flag(BYPASS.on(r.slot)) {
                continue;
            }
            let amount = values.get(AMOUNT.on(r.slot)) / 100.0;
            if amount == 0.0 {
                continue;
            }
            let mut v = sources.value(r.source);
            if values.flag(BIPOLAR.on(r.slot)) && !r.source.is_bipolar() {
                v = v * 2.0 - 1.0;
            }
            if let Some(aux) = r.aux {
                let a = sources.value(aux);
                v *= if values.flag(AUX_INVERTED.on(r.slot)) { 1.0 - a } else { a };
            }
            if r.target.is_dynamic() {
                self.fx.push((r.target.instance, r.target.index, v * amount));
                continue;
            }
            let i = r.target.flat();
            if self.values[i] == 0.0 {
                self.touched.push(i);
            }
            self.values[i] += v * amount;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn slot(source: [i128; 2], module: &str, id: i128, param: &str, param_id: i128) -> Value {
        Value::Map(vec![
            (text("destModuleID"), Value::Int(id)),
            (text("destModuleParamID"), Value::Int(param_id)),
            (text("destModuleParamName"), text(param)),
            (text("destModuleTypeString"), text(module)),
            (text("source"), Value::Array(vec![Value::Int(source[0]), Value::Int(source[1])])),
        ])
    }

    #[test]
    fn parses_factory_style_slot() {
        // ModSlot0 of "ARP - Aardvark": LFO 1 → Filter 1 cutoff.
        let s = slot([6, 0], "VoiceFilter", 0, "kParamFreq", 3);
        let r = parse_slot(0, &s).unwrap();
        assert_eq!(r.source, Source::Lfo(0));
        assert_eq!(r.target, ParamRef::new("VoiceFilter", 0, "kParamFreq"));
        assert_eq!(id_mismatch(&s, &r), None);
    }

    #[test]
    fn any_module_parameter_is_a_target() {
        let r = parse_slot(0, &slot([6, 0], "Oscillator", 3, "kParamVolume", 1)).unwrap();
        assert_eq!(r.target, ParamRef::new("Oscillator", 3, "kParamVolume"));
        assert_eq!(parse_slot(0, &slot([55, 0], "VoiceFilter", 0, "kParamFreq", 3)), Err(Skipped::UnsupportedSource(55)));
        assert!(matches!(parse_slot(0, &slot([6, 0], "FXNope", 0, "kParamWet", 1)), Err(Skipped::UnknownTarget(_))));
    }

    #[test]
    fn amount_aux_and_bipolar_come_from_slot_params() {
        let mut values = Values::default();
        values.set(AMOUNT.on(2), 50.0);
        values.set(BIPOLAR.on(2), 1.0);
        let target = ParamRef::new("VoiceFilter", 0, "kParamFreq");
        let route = Route { source: Source::Lfo(0), aux: Some(Source::Velocity), slot: 2, target };
        let mut s = Sources { velocity: 0.5, ..Sources::default() };
        s.lfos[0] = 1.0;
        let mut offsets = Offsets::default();
        offsets.evaluate(std::slice::from_ref(&route), &s, &values);
        assert!((offsets.get(target) - 0.25).abs() < 1e-6);
        values.set(BYPASS.on(2), 1.0);
        offsets.evaluate(&[route], &s, &values);
        assert_eq!(offsets.get(target), 0.0);
    }
}
