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
use lumen_dsp::curve;
use lumen_dsp::lfo::Curve;
use lumen_preset::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    ModWheel,
    Env(usize),
    Lfo(usize),
    /// The second output of LFO n (chaotic types; 0 for curve and S&H LFOs).
    LfoY(usize),
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
            39..=48 => Source::LfoY((id - 39) as usize),
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
    /// Drawn curves for the source and the aux source (`flex`), used when the slot's
    /// main/aux curve flags are on.
    pub curves: [Option<Curve>; 2],
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
const MAIN_CURVE: ParamRef = ParamRef::new("ModSlot", 0, "kParamMainCurveData");
const AUX_CURVE: ParamRef = ParamRef::new("ModSlot", 0, "kParamAuxCurveData");
const CURVE_IN: ParamRef = ParamRef::new("ModSlot", 0, "kParamCurveIn");
const AUX_CURVE_IN: ParamRef = ParamRef::new("ModSlot", 0, "kParamAuxCurve");
const OUT: ParamRef = ParamRef::new("ModSlot", 0, "kParamOut");

/// A drawn curve (LFO shape or slot curve): y is stored top-down (0 is the top of
/// the editor) and `numPoints` counts segments, so the arrays hold one more point.
pub fn slot_curve(data: &Value) -> Option<Curve> {
    let n = data.get("numPoints")?.as_f64()? as usize;
    let xs = data.get("xVals")?.as_array()?;
    let ys = data.get("yVals")?.as_array()?;
    let tensions: Vec<f32> = data.get("curveVals").and_then(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_f64).map(|v| v as f32).collect();
    // numPoints counts segments: the arrays hold one more point.
    let n = (n + 1).min(xs.len()).min(ys.len());
    let points: Vec<(f32, f32)> = xs[..n].iter().zip(&ys[..n]).filter_map(|(x, y)| Some((x.as_f64()? as f32, 1.0 - y.as_f64()? as f32))).collect();
    Curve::new(&points, &tensions)
}

/// Maps a source value through a drawn curve that spans the source's full range: a
/// curve is not periodic, so the end point is reached at 1.
fn through(curve: &Curve, v: f32) -> f32 {
    curve.eval(v.clamp(0.0, 1.0) * 0.999_999)
}

/// Resolves a slot's source and destination. Amount and flags stay in `Values`.
pub fn parse_slot(slot_index: usize, slot: &Value) -> Result<Route, Skipped> {
    let pair = slot.get("source").and_then(Value::as_array).ok_or(Skipped::Empty)?;
    let id = |i: usize| pair.get(i).and_then(Value::as_f64).map(|v| v as i64).unwrap_or(0);
    let source = Source::from_id(id(0)).map_err(Skipped::UnsupportedSource)?.ok_or(Skipped::Empty)?;
    let aux = Source::from_id(id(1)).map_err(Skipped::UnsupportedSource)?;
    let module = slot.get("destModuleTypeString").and_then(Value::as_str).ok_or(Skipped::Empty)?;
    let stored = slot.get("destModuleID").and_then(Value::as_f64).unwrap_or(0.0) as usize;
    let param = slot.get("destModuleParamName").and_then(Value::as_str).unwrap_or("");
    let unknown = || Skipped::UnknownTarget(format!("{module}.{param}"));
    let kind = params::kind_of(module).ok_or_else(unknown)?;
    let spec = params::MODULES[kind];
    let index = spec.index_of(param).ok_or_else(unknown)?;
    let instance = fixed_instance(spec, stored);
    if !spec.is_dynamic() && instance >= spec.instances {
        return Err(unknown());
    }
    let flex = slot.get("flex").and_then(Value::as_array);
    let curves = [0, 1].map(|i| flex.and_then(|f| f.get(i)).and_then(slot_curve));
    Ok(Route { source, aux, slot: slot_index, target: ParamRef { kind, instance, index }, curves })
}

/// A single-instance module living in a numbered section ("Oscillator3.NoiseOsc3") is
/// addressed by that number (factory slots store NoiseOsc as module 3); others by index.
fn fixed_instance(spec: &params::ModuleSpec, stored: usize) -> usize {
    if spec.instances != 1 || spec.section.contains('#') {
        return stored;
    }
    let digits: String = spec.section.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    match digits.parse::<usize>() {
        Ok(n) if n == stored => 0,
        _ => stored,
    }
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
    pub lfo_y: [f32; 10],
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
            Source::LfoY(i) => self.lfo_y[i],
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
            // Out scales the slot's output (50% halves it, slot probes).
            let amount = values.get(AMOUNT.on(r.slot)) / 100.0 * values.get(OUT.on(r.slot)) / 100.0;
            if amount == 0.0 {
                continue;
            }
            let mut v = sources.value(r.source);
            // The drawn curve, else the input curve (−100…100), shapes the source.
            let native = r.source.is_bipolar();
            let unit = if native { (v + 1.0) * 0.5 } else { v };
            let shaped = match &r.curves[0] {
                Some(c) if values.flag(MAIN_CURVE.on(r.slot)) => through(c, unit),
                _ => match values.get(CURVE_IN.on(r.slot)) {
                    c if c != 0.0 => curve::shape(unit.clamp(0.0, 1.0), c / 100.0),
                    _ => unit,
                },
            };
            if shaped != unit {
                v = if native { shaped * 2.0 - 1.0 } else { shaped };
            }
            // Bipolar spans the amount peak to peak: amount 50 swings ±0.25 (lfo probes).
            if values.flag(BIPOLAR.on(r.slot)) && !r.source.is_bipolar() {
                v -= 0.5;
            }
            if let Some(aux) = r.aux {
                let mut a = sources.value(aux);
                // The aux source takes its own drawn curve, else its input curve (like curve-in).
                if let Some(c) = r.curves[1].as_ref().filter(|_| values.flag(AUX_CURVE.on(r.slot))) {
                    a = through(c, a);
                } else {
                    let c = values.get(AUX_CURVE_IN.on(r.slot));
                    if c != 0.0 {
                        a = curve::shape(a.clamp(0.0, 1.0), c / 100.0);
                    }
                }
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
    fn numbered_single_modules_resolve_by_their_section_number() {
        // Factory "ModSlot → NoiseOsc #3 kParamColor".
        let r = parse_slot(0, &slot([6, 0], "NoiseOsc", 3, "kParamColor", 0)).unwrap();
        assert_eq!(r.target, ParamRef::new("NoiseOsc", 0, "kParamColor"));
    }

    #[test]
    fn amount_aux_and_bipolar_come_from_slot_params() {
        let mut values = Values::default();
        values.set(AMOUNT.on(2), 50.0);
        values.set(BIPOLAR.on(2), 1.0);
        let target = ParamRef::new("VoiceFilter", 0, "kParamFreq");
        let route = Route { source: Source::Lfo(0), aux: Some(Source::Velocity), slot: 2, target, curves: [None, None] };
        let mut s = Sources { velocity: 0.5, ..Sources::default() };
        s.lfos[0] = 1.0;
        let mut offsets = Offsets::default();
        offsets.evaluate(std::slice::from_ref(&route), &s, &values);
        assert!((offsets.get(target) - 0.125).abs() < 1e-6);
        values.set(BYPASS.on(2), 1.0);
        offsets.evaluate(&[route], &s, &values);
        assert_eq!(offsets.get(target), 0.0);
    }
}
