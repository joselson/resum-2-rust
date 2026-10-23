//! Loads a Serum 2 preset payload: every module section through the parameter
//! tables, plus the non-parameter data (wavetables, LFO shapes, mod routing).

use crate::fx::{FxSetup, NodeSetup};
use crate::modulation::{self, Route, Skipped};
use crate::params::{self, Curve, ParamRef, Values, MODULES};
use crate::sampleosc::{self, SampleSource};
use crate::tables::TableStore;
use crate::voice::RENDERED;
use lumen_dsp::lfo::{Curve as LfoCurve, Path as LfoPath};
use lumen_dsp::wavetable::Wavetable;
use lumen_preset::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a preset uses that Lumen does not render, with occurrence counts.
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub unsupported: BTreeMap<String, usize>,
    pub table_errors: Vec<String>,
    pub routes: usize,
    /// Mod slots whose stored parameter ID disagrees with the parameter table.
    pub id_mismatches: usize,
}

impl Coverage {
    fn note(&mut self, what: impl Into<String>) {
        *self.unsupported.entry(what.into()).or_default() += 1;
    }
}

#[derive(Clone)]
pub struct Patch {
    pub name: String,
    pub values: Values,
    /// Wavetables for oscillators A–C (the built-in table when none is set).
    pub tables: [Arc<Wavetable>; 3],
    /// Samples or multisample regions for oscillators A–C of those types.
    pub samples: [SampleSource; 3],
    pub lfo_curves: Vec<LfoCurve>,
    /// Drawn XY paths of Path-type LFOs (None: no path, which Serum plays as 0.5, 0.5).
    pub lfo_paths: Vec<Option<LfoPath>>,
    pub routes: Vec<Route>,
    pub fx: FxSetup,
}

/// Follows a dotted section path ("Oscillator0.WTOsc0").
fn section<'a>(payload: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(payload, |v, key| v.get(key))
}

fn load_params(payload: &Value, values: &mut Values, cov: &mut Coverage) {
    for (kind, spec) in MODULES.iter().enumerate() {
        for instance in 0..spec.instances {
            let Some(Value::Map(entries)) = section(payload, &spec.section_path(instance)).and_then(|s| s.get("plainParams")) else { continue };
            for (key, value) in entries {
                let Some(key) = key.as_str() else { continue };
                let Some(index) = spec.index_of(key) else {
                    cov.note(format!("{}.{key} (unknown parameter)", spec.name));
                    continue;
                };
                let p = ParamRef { kind, instance, index };
                let plain = match (value, p.spec().curve) {
                    (Value::Text(name), Curve::Choice) => match p.spec().option_index(name) {
                        Some(i) => i as f32,
                        None => {
                            cov.note(format!("{}.{key} option {name}", spec.name));
                            continue;
                        }
                    },
                    (v, _) => match v.as_f64() {
                        Some(x) => x as f32,
                        None => continue,
                    },
                };
                values.set(p, plain);
            }
        }
    }
}

/// A Path LFO's points; unlike curves, `numPoints` is the point count and y is not flipped.
fn lfo_path(section: Option<&Value>) -> Option<LfoPath> {
    let data = section?.get("pathData")?;
    let n = data.get("numPoints")?.as_f64()? as usize;
    let num = |key: &str| -> Vec<f32> { data.get(key).and_then(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_f64).map(|v| v as f32).collect() };
    let (xs, ys) = (num("xVals"), num("yVals"));
    let n = n.min(xs.len()).min(ys.len());
    let points: Vec<(f32, f32)> = xs[..n].iter().zip(&ys[..n]).map(|(&x, &y)| (x, y)).collect();
    let open = matches!(data.get("isOpen"), Some(Value::Bool(true)));
    LfoPath::new(&points, &num("curveVals"), !open)
}

fn lfo_curve(section: Option<&Value>) -> Option<LfoCurve> {
    crate::modulation::slot_curve(section?.get("curveData")?)
}

const OSC_ENABLE: ParamRef = ParamRef::new("Oscillator", 0, "kParamEnable");
const OSC_TYPE: ParamRef = ParamRef::new("Oscillator", 0, "kParamType");
const FILTER_ENABLE: ParamRef = ParamRef::new("VoiceFilter", 0, "kParamEnable");
const FILTER_TYPE: ParamRef = ParamRef::new("VoiceFilter", 0, "kParamType");
const LFO_TYPE: ParamRef = ParamRef::new("LFO", 0, "kParamType");
const WARP_MENUS: [ParamRef; 2] = [ParamRef::new("WTOsc", 0, "kParamWarpMenu"), ParamRef::new("WTOsc", 0, "kParamWarpMenu2")];

/// Whether a module instance can affect the sound (disabled oscillators/filters can't).
fn active(values: &Values, kind: usize, instance: usize) -> bool {
    match MODULES[kind].name {
        "Oscillator" => values.flag(OSC_ENABLE.on(instance)),
        "WTOsc" => values.flag(OSC_ENABLE.on(instance)) && values.choice(OSC_TYPE.on(instance)) == "kOsc_WT",
        "NoiseOsc" => values.flag(OSC_ENABLE.on(3)),
        "SubOsc" => values.flag(OSC_ENABLE.on(4)),
        "VoiceFilter" => values.flag(FILTER_ENABLE.on(instance)),
        // Editor/sequencer modules are reported separately; unused mod slots keep defaults.
        "ClipPlayer" | "PitchQuantizer" => false,
        _ => true,
    }
}

fn note_unrendered(values: &Values, cov: &mut Coverage) {
    for (kind, spec) in MODULES.iter().enumerate() {
        for instance in 0..spec.instances {
            if !active(values, kind, instance) {
                continue;
            }
            for (index, p) in spec.params.iter().enumerate() {
                let r = ParamRef { kind, instance, index };
                let key = (spec.name, p.name);
                if !values.is_default(r) && !RENDERED.contains(&key) && !sampleosc::RENDERED.contains(&key) {
                    cov.note(format!("{}.{}", spec.name, p.name));
                }
            }
        }
    }
}

impl Patch {
    /// A playable patch with every parameter at its default.
    pub fn init(tables: &mut TableStore) -> Patch {
        Patch::from_payload(&Value::Map(Vec::new()), tables).0
    }

    pub fn from_payload(payload: &Value, tables: &mut TableStore) -> (Patch, Coverage) {
        let mut cov = Coverage::default();
        let mut values = Values::default();
        load_params(payload, &mut values, &mut cov);

        let table_for = |i: usize, tables: &mut TableStore, cov: &mut Coverage| {
            let path = section(payload, &format!("Oscillator{i}.WTOsc{i}")).and_then(|s| s.get("relativePathToWT")).and_then(Value::as_str);
            match path {
                Some(path) if values.flag(OSC_ENABLE.on(i)) => tables.load(path).unwrap_or_else(|e| {
                    cov.table_errors.push(e);
                    tables.fallback()
                }),
                _ => tables.fallback(),
            }
        };
        let osc_tables = [table_for(0, tables, &mut cov), table_for(1, tables, &mut cov), table_for(2, tables, &mut cov)];

        let mut samples: [SampleSource; 3] = Default::default();
        for (i, slot) in samples.iter_mut().enumerate() {
            let kind = values.choice(OSC_TYPE.on(i));
            if !values.flag(OSC_ENABLE.on(i)) {
                continue;
            }
            match kind {
                "kOsc_WT" => {}
                "kOsc_Sample" | "kOsc_MultiSample" => {
                    *slot = sampleosc::load(payload, i, kind, tables, &mut cov.table_errors);
                    cov.note(match slot {
                        SampleSource::None => format!("oscillator type {kind} (not loaded)"),
                        _ => format!("oscillator type {kind} (approximate)"),
                    });
                }
                _ => cov.note(format!("oscillator type {kind}")),
            }
        }
        for i in 0..3 {
            let kind = values.choice(OSC_TYPE.on(i));
            if values.flag(OSC_ENABLE.on(i)) && kind == "kOsc_WT" {
                for menu in WARP_MENUS {
                    let mode = values.choice(menu.on(i));
                    if !matches!(mode, "" | "kNoWarp") {
                        cov.note(format!("warp {mode} (approximate)"));
                    }
                }
            }
        }
        for i in 0..2 {
            if values.flag(FILTER_ENABLE.on(i)) {
                let name = values.choice(FILTER_TYPE.on(i));
                match crate::filters::model(name).1 {
                    crate::filters::Fidelity::Matched => {}
                    crate::filters::Fidelity::Approximate => cov.note(format!("filter type {name} (approximate)")),
                    crate::filters::Fidelity::Missing => cov.note(format!("filter type {name}")),
                }
            }
        }
        let lfo_curves = (0..10)
            .map(|i| {
                let kind = values.choice(LFO_TYPE.on(i));
                if !matches!(kind, "Curve" | "Path" | "RandomSH" | "Lorenz" | "Rossler") {
                    cov.note(format!("LFO type {kind}"));
                }
                lfo_curve(payload.get(&format!("LFO{i}"))).unwrap_or_else(LfoCurve::triangle)
            })
            .collect();

        let mut routes = Vec::new();
        for i in 0..64 {
            let Some(slot) = payload.get(&format!("ModSlot{i}")) else { continue };
            match modulation::parse_slot(i, slot) {
                Ok(route) => {
                    if modulation::id_mismatch(slot, &route).is_some() {
                        cov.id_mismatches += 1;
                    }
                    let module = MODULES[route.target.kind].name;
                    // FX parameters are rendered by their effect (reported per module type).
                    if !route.target.is_dynamic() && !RENDERED.contains(&(module, route.target.spec().name)) {
                        cov.note(format!("mod target {module}.{}", route.target.spec().name));
                    }
                    routes.push(route);
                }
                Err(Skipped::Empty) => {}
                Err(Skipped::UnsupportedSource(id)) => cov.note(format!("mod source #{id}")),
                Err(Skipped::UnknownTarget(t)) => cov.note(format!("mod target {t}")),
            }
        }
        cov.routes = routes.len();

        for (key, _) in payload.entries() {
            let Some(key) = key.as_str() else { continue };
            let base = key.trim_end_matches(|c: char| c.is_ascii_digit());
            if (base == "Arp" && key != base || key == "ClipPlayer0") && !is_default_section(payload.get(key)) {
                cov.note(format!("{base} (not implemented)"));
            }
        }
        note_unrendered(&values, &mut cov);
        let fx = FxSetup::load(payload, tables, &mut cov.unsupported);
        note_fx(&fx, &mut cov);

        let name = payload.get("presetName").and_then(Value::as_str).unwrap_or("Init").to_owned();
        let lfo_paths = (0..10).map(|i| lfo_path(payload.get(&format!("LFO{i}")))).collect();
        (Patch { name, values, tables: osc_tables, samples, lfo_curves, lfo_paths, routes, fx }, cov)
    }

    pub fn param(&self, p: ParamRef) -> f32 {
        self.values.get(p)
    }

    /// Sets one parameter by module/parameter name (live edit or host automation).
    pub fn set(&mut self, module: &str, instance: usize, param: &str, plain: f32) -> bool {
        let Some(kind) = params::kind_of(module) else { return false };
        let Some(index) = MODULES[kind].index_of(param) else { return false };
        if instance >= MODULES[kind].instances {
            return false;
        }
        self.values.set(ParamRef { kind, instance, index }, plain);
        true
    }
}

/// FX modules whose model is fitted to measurements (docs/CALIBRATION.md; distortion
/// without X-shaper drawn curves, chorus without its exact high-pass and beat sync). The
/// reverb's metrics are fitted too, but on a different network and with controls
/// combined as independent.
const MEASURED_FX: &[&str] = &["FXEQ", "FXDelay", "FXFilter", "FXComp", "FXDistortion", "FXChorus"];

/// Reports each FX module type used whose model is not yet calibrated.
fn note_fx(fx: &FxSetup, cov: &mut Coverage) {
    fn walk(nodes: &[NodeSetup], cov: &mut Coverage) {
        for n in nodes {
            match n {
                NodeSetup::Effect(m) if MEASURED_FX.contains(&MODULES[m.kind].name) => {}
                NodeSetup::Effect(m) => cov.note(format!("{} (approximate)", MODULES[m.kind].name)),
                NodeSetup::Split { module, bands } => {
                    cov.note(format!("{} (approximate)", MODULES[module.kind].name));
                    bands.iter().for_each(|b| walk(b, cov));
                }
            }
        }
    }
    fx.racks.iter().for_each(|r| walk(r, cov));
}

/// True when a section stores nothing but defaults.
fn is_default_section(section: Option<&Value>) -> bool {
    match section {
        None => true,
        Some(Value::Map(entries)) => entries.iter().all(|(k, v)| {
            matches!(v, Value::Text(t) if t == "default")
                || matches!(v, Value::Map(m) if m.is_empty())
                || matches!(v, Value::Array(a) if a.is_empty())
                || k.as_str() == Some("displayName")
        }),
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn init_patch_uses_measured_defaults() {
        let mut tables = TableStore::new(None);
        let p = Patch::init(&mut tables);
        assert!(p.values.flag(OSC_ENABLE) && !p.values.flag(OSC_ENABLE.on(1)));
        assert_eq!(p.values.choice(FILTER_TYPE), "MgL12");
        assert!((p.param(ParamRef::new("Oscillator", 0, "kParamVolume")) - 0.5625).abs() < 1e-6);
        assert!(p.routes.is_empty());
    }

    #[test]
    fn loads_sections_and_reports_unknowns() {
        let payload = Value::Map(vec![
            (t("Env1"), Value::Map(vec![(t("plainParams"), Value::Map(vec![(t("kParamDecay"), Value::Float(0.25)), (t("kParamBogus"), Value::Float(1.0))]))])),
            (t("Oscillator1"), Value::Map(vec![
                (t("plainParams"), Value::Map(vec![(t("kParamEnable"), Value::Float(1.0)), (t("kParamUnisonStack"), t("kOctave1")), (t("kParamUnisonSpan"), Value::Float(30.0))])),
                (t("WTOsc1"), Value::Map(vec![(t("plainParams"), Value::Map(vec![(t("kParamWarpMenu"), t("kNotAMode"))]))])),
            ])),
        ]);
        let mut tables = TableStore::new(None);
        let (p, cov) = Patch::from_payload(&payload, &mut tables);
        assert_eq!(p.param(ParamRef::new("Env", 1, "kParamDecay")), 0.25);
        assert!(p.values.flag(OSC_ENABLE.on(1)));
        assert_eq!(p.values.choice(ParamRef::new("Oscillator", 1, "kParamUnisonStack")), "kOctave1");
        assert!(cov.unsupported.contains_key("Env.kParamBogus (unknown parameter)"));
        assert!(cov.unsupported.contains_key("WTOsc.kParamWarpMenu option kNotAMode"));
        assert!(cov.unsupported.contains_key("Oscillator.kParamUnisonSpan"), "{:?}", cov.unsupported);
    }
}
