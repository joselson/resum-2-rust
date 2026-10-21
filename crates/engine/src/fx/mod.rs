//! FX racks: Serum 2's three chains (main, bus 1, bus 2) of effect modules.
//!
//! A rack is an ordered module list. Split modules own the modules that follow
//! them (`kParamModuleCount1..3` per band), so the list parses into a tree.
//! Every module is addressed as `rack × 100 + position in the list`, the
//! `destModuleID` mod slots use.

mod dist_curves;
mod effects;
mod reverb_model;
mod reverb_table;

use crate::params::{self, ModuleSpec, MODULES};
use crate::tables::TableStore;
use lumen_dsp::sampler::SampleData;
use lumen_dsp::biquad::Crossover;
use lumen_preset::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub use effects::Effect;

/// Host state shared by all effects.
#[derive(Clone, Copy, Debug)]
pub struct FxContext {
    pub sample_rate: f32,
    pub bpm: f32,
}

/// One module's parameters with modulation applied.
pub struct FxParams<'a> {
    spec: &'static ModuleSpec,
    values: &'a [f32],
    offsets: &'a [f32],
}

impl FxParams<'_> {
    #[inline]
    pub fn get(&self, index: usize) -> f32 {
        self.spec.params[index].offset(self.values[index], self.offsets[index])
    }

    pub fn flag(&self, index: usize) -> bool {
        self.get(index) >= 0.5
    }

    pub fn choice(&self, index: usize) -> &'static str {
        let p = &self.spec.params[index];
        p.options.get(self.get(index).round().max(0.0) as usize).copied().unwrap_or("")
    }
}

/// Static description of one module, owned by the patch.
#[derive(Clone)]
pub struct ModuleSetup {
    pub kind: params::ModuleKind,
    /// rack × 100 + position.
    pub id: usize,
    pub values: Vec<f32>,
    pub impulse: Option<Arc<SampleData>>,
}

#[derive(Clone)]
pub enum NodeSetup {
    Effect(ModuleSetup),
    Split { module: ModuleSetup, bands: Vec<Vec<NodeSetup>> },
}

/// The three racks of a patch.
#[derive(Clone, Default)]
pub struct FxSetup {
    pub racks: [Vec<NodeSetup>; 3],
}

impl FxSetup {
    pub fn is_empty(&self) -> bool {
        self.racks.iter().all(Vec::is_empty)
    }

    /// Parses `FXRack0..2`. Unknown module types and parameters are reported in `notes`.
    pub fn load(payload: &Value, tables: &mut TableStore, notes: &mut BTreeMap<String, usize>) -> FxSetup {
        let mut setup = FxSetup::default();
        for (r, rack) in setup.racks.iter_mut().enumerate() {
            let Some(list) = payload.get(&format!("FXRack{r}")).and_then(|v| v.get("FX")).and_then(Value::as_array) else { continue };
            let mut pos = 0;
            while pos < list.len() {
                if let Some(node) = parse_node(list, &mut pos, r, tables, notes) {
                    rack.push(node);
                }
            }
        }
        setup
    }
}

fn note(notes: &mut BTreeMap<String, usize>, what: String) {
    *notes.entry(what).or_default() += 1;
}

fn parse_module(entry: &Value, id: usize, tables: &mut TableStore, notes: &mut BTreeMap<String, usize>) -> Option<ModuleSetup> {
    let (name, body) = entry.entries().iter().find_map(|(k, v)| k.as_str().filter(|k| k.starts_with("FX")).map(|k| (k, v)))?;
    let Some(kind) = params::kind_of(name).filter(|&k| MODULES[k].is_dynamic()) else {
        note(notes, format!("FX module {name} (unknown)"));
        return None;
    };
    let spec = MODULES[kind];
    let mut values: Vec<f32> = spec.params.iter().map(|p| p.default_for(0)).collect();
    if let Some(Value::Map(entries)) = body.get("plainParams") {
        for (k, v) in entries {
            let Some(k) = k.as_str() else { continue };
            let Some(index) = spec.index_of(k) else {
                note(notes, format!("{name}.{k} (unknown parameter)"));
                continue;
            };
            let p = &spec.params[index];
            values[index] = match (v, p.curve) {
                (Value::Text(t), params::Curve::Choice) => match p.option_index(t) {
                    Some(i) => i as f32,
                    None => {
                        note(notes, format!("{name}.{k} option {t}"));
                        continue;
                    }
                },
                (v, _) => match v.as_f64() {
                    Some(x) => x as f32,
                    None => continue,
                },
            };
        }
    }
    let impulse = body.get("relativePathToIR").and_then(Value::as_str).and_then(|path| match tables.impulse(path) {
        Ok(ir) => Some(ir),
        Err(e) => {
            note(notes, format!("impulse not loaded: {e}"));
            None
        }
    });
    if name == "FXConv" && impulse.is_none() && body.get("embeddedIR").is_some() {
        note(notes, "FXConv embedded impulse".into());
    }
    Some(ModuleSetup { kind, id, values, impulse })
}

/// Consumes the module at `pos` (and, for splits, the modules its bands own).
fn parse_node(list: &[Value], pos: &mut usize, rack: usize, tables: &mut TableStore, notes: &mut BTreeMap<String, usize>) -> Option<NodeSetup> {
    let id = rack * 100 + *pos;
    let entry = &list[*pos];
    *pos += 1;
    let module = parse_module(entry, id, tables, notes)?;
    let spec = MODULES[module.kind];
    let band_counts: Vec<usize> = (1..=3)
        .filter_map(|b| spec.index_of(&format!("kParamModuleCount{b}")))
        .map(|i| module.values[i].max(0.0) as usize)
        .collect();
    if band_counts.is_empty() {
        return Some(NodeSetup::Effect(module));
    }
    let bands = band_counts
        .iter()
        .map(|&count| {
            let mut band = Vec::new();
            for _ in 0..count {
                if *pos >= list.len() {
                    break;
                }
                if let Some(n) = parse_node(list, pos, rack, tables, notes) {
                    band.push(n);
                }
            }
            band
        })
        .collect();
    Some(NodeSetup::Split { module, bands })
}

/// A live module: setup values, per-block modulation offsets and DSP state.
struct Slot {
    spec: &'static ModuleSpec,
    id: usize,
    values: Vec<f32>,
    offsets: Vec<f32>,
    effect: Box<dyn Effect>,
    enable: Option<usize>,
    wet: Option<usize>,
    level: Option<usize>,
    dry: [Vec<f32>; 2],
}

impl Slot {
    fn new(m: &ModuleSetup, ctx: &FxContext, max_block: usize) -> Slot {
        let spec = MODULES[m.kind];
        // Wet/dry is applied here for every module except splits and effects that mix their own.
        let is_split = spec.name.starts_with("FXSplit");
        let effect = effects::build(spec.name, ctx, m.impulse.as_deref());
        let wet = spec.index_of("kParamWet").filter(|_| !is_split && !effect.owns_wet());
        Slot {
            spec,
            id: m.id,
            offsets: vec![0.0; m.values.len()],
            values: m.values.clone(),
            effect,
            enable: spec.index_of("kParamEnable"),
            wet,
            level: spec.index_of("kParamLevelOut"),
            dry: [vec![0.0; max_block], vec![0.0; max_block]],
        }
    }

    fn params(&self) -> FxParams<'_> {
        FxParams { spec: self.spec, values: &self.values, offsets: &self.offsets }
    }

    fn enabled(&self) -> bool {
        self.enable.is_none_or(|i| self.params().flag(i))
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
        if !self.enabled() {
            return;
        }
        let n = l.len();
        let wet = self.wet.map(|i| self.params().get(i) / 100.0);
        if wet.is_some() {
            self.dry[0][..n].copy_from_slice(l);
            self.dry[1][..n].copy_from_slice(r);
        }
        let p = FxParams { spec: self.spec, values: &self.values, offsets: &self.offsets };
        self.effect.process(&p, l, r, ctx);
        // Output level: 0.5 is unity and gain is squared (measured: 0.25 → −12 dB).
        let gain = self.level.map(|i| (p.get(i) / 0.5).powi(2)).unwrap_or(1.0);
        match wet {
            Some(w) => {
                // Wet follows a raised cosine and dry is 1 − wet² (fxodm- probes: delay
                // echoes and distortion at drive 0, exact to 10⁻⁴); 50 % is 0.75 dry, 0.5 wet.
                let wg = 0.5 - 0.5 * (w * std::f32::consts::PI).cos();
                let dg = 1.0 - wg * wg;
                for i in 0..n {
                    l[i] = (self.dry[0][i] * dg + l[i] * wg) * gain;
                    r[i] = (self.dry[1][i] * dg + r[i] * wg) * gain;
                }
            }
            None if gain != 1.0 => {
                l.iter_mut().chain(r.iter_mut()).for_each(|s| *s *= gain);
            }
            None => {}
        }
    }
}

enum Node {
    Effect(Slot),
    Split { slot: Slot, bands: Vec<Vec<Node>>, crossovers: Box<[[Crossover; 2]; 2]>, scratch: Vec<[Vec<f32>; 2]> },
}

const SPLIT_FREQ: usize = params::index("FXSplit", "kParamFreq");
const SPLIT3_FREQ: usize = params::index("FXSplit3", "kParamFreq");
const SPLIT3_FREQ2: usize = params::index("FXSplit3", "kParamFreq2");
const SPLIT_SOLO: usize = params::index("FXSplit", "kParamSolo");
const SPLIT3_SOLO: usize = params::index("FXSplit3", "kParamSolo");

fn build_nodes(setups: &[NodeSetup], ctx: &FxContext, max_block: usize) -> Vec<Node> {
    setups
        .iter()
        .map(|n| match n {
            NodeSetup::Effect(m) => Node::Effect(Slot::new(m, ctx, max_block)),
            NodeSetup::Split { module, bands } => Node::Split {
                slot: Slot::new(module, ctx, max_block),
                scratch: (0..bands.len().max(2)).map(|_| [vec![0.0; max_block], vec![0.0; max_block]]).collect(),
                bands: bands.iter().map(|b| build_nodes(b, ctx, max_block)).collect(),
                crossovers: Default::default(),
            },
        })
        .collect()
}

fn process_nodes(nodes: &mut [Node], l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
    for node in nodes {
        match node {
            Node::Effect(slot) => slot.process(l, r, ctx),
            Node::Split { slot, bands, crossovers, scratch } => process_split(slot, bands, crossovers, scratch, l, r, ctx),
        }
    }
}

/// Splits into bands (frequency or mid/side), runs each band's chain, and sums.
fn process_split(slot: &mut Slot, bands: &mut [Vec<Node>], crossovers: &mut [[Crossover; 2]; 2], scratch: &mut [[Vec<f32>; 2]], l: &mut [f32], r: &mut [f32], ctx: &FxContext) {
    let n = l.len();
    let name = slot.spec.name;
    let p = slot.params();
    let (count, solo) = match name {
        "FXSplit" => (2, p.get(SPLIT_SOLO)),
        "FXSplit3" => (3, p.get(SPLIT3_SOLO)),
        _ => (2, -1.0),
    };
    if !slot.enabled() {
        return;
    }
    match name {
        "FXSplitMS" => {
            for i in 0..n {
                scratch[0][0][i] = 0.5 * (l[i] + r[i]);
                scratch[0][1][i] = scratch[0][0][i];
                scratch[1][0][i] = 0.5 * (l[i] - r[i]);
                scratch[1][1][i] = -scratch[1][0][i];
            }
        }
        _ => {
            let (f1, f2) = if count == 3 { (p.get(SPLIT3_FREQ), p.get(SPLIT3_FREQ2)) } else { (p.get(SPLIT_FREQ), 0.0) };
            let [first, second] = crossovers;
            for (a, b) in first.iter_mut().zip(second.iter_mut()) {
                a.set(f1, ctx.sample_rate);
                b.set(f2.max(f1), ctx.sample_rate);
            }
            for i in 0..n {
                for (ch, x) in [l[i], r[i]].into_iter().enumerate() {
                    let (low, high) = crossovers[0][ch].split(x);
                    scratch[0][ch][i] = low;
                    if count == 3 {
                        let (mid, top) = crossovers[1][ch].split(high);
                        scratch[1][ch][i] = mid;
                        scratch[2][ch][i] = top;
                    } else {
                        scratch[1][ch][i] = high;
                    }
                }
            }
        }
    }
    let bypass: Vec<bool> = (1..=count).map(|b| slot.spec.index_of(&format!("kParamBand{b}Bypass")).is_some_and(|i| slot.params().flag(i))).collect();
    l.fill(0.0);
    r.fill(0.0);
    for b in 0..count {
        if solo >= 0.0 && (solo.round() as usize) != b {
            continue;
        }
        let [bl, br] = &mut scratch[b];
        if !bypass[b] {
            if let Some(chain) = bands.get_mut(b) {
                process_nodes(chain, &mut bl[..n], &mut br[..n], ctx);
            }
        }
        for i in 0..n {
            l[i] += bl[i];
            r[i] += br[i];
        }
    }
}

fn for_each_slot(nodes: &mut [Node], f: &mut impl FnMut(&mut Slot)) {
    for node in nodes {
        match node {
            Node::Effect(s) => f(s),
            Node::Split { slot, bands, .. } => {
                f(slot);
                for b in bands {
                    for_each_slot(b, f);
                }
            }
        }
    }
}

/// Live racks for one synth instance.
pub struct FxRacks {
    racks: [Vec<Node>; 3],
    ctx: FxContext,
}

impl FxRacks {
    pub fn new(setup: &FxSetup, sample_rate: f32, max_block: usize) -> FxRacks {
        let ctx = FxContext { sample_rate, bpm: 120.0 };
        FxRacks { racks: std::array::from_fn(|i| build_nodes(&setup.racks[i], &ctx, max_block)), ctx }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.ctx.bpm = bpm;
    }

    pub fn reset(&mut self) {
        for rack in &mut self.racks {
            for_each_slot(rack, &mut |s| s.effect.reset());
        }
    }

    /// Replaces modulation offsets: `(module id, parameter index, normalized offset)`.
    pub fn set_modulation(&mut self, offsets: &[(usize, usize, f32)]) {
        for rack in &mut self.racks {
            for_each_slot(rack, &mut |s| {
                s.offsets.fill(0.0);
                for &(id, index, delta) in offsets {
                    if id == s.id && index < s.offsets.len() {
                        s.offsets[index] += delta;
                    }
                }
            });
        }
    }

    pub fn is_empty(&self, rack: usize) -> bool {
        self.racks[rack].is_empty()
    }

    pub fn process(&mut self, rack: usize, l: &mut [f32], r: &mut [f32]) {
        let ctx = self.ctx;
        process_nodes(&mut self.racks[rack], l, r, &ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn module(name: &str, ty: i128, params: Vec<(&str, Value)>) -> Value {
        let pp = if params.is_empty() { t("default") } else { Value::Map(params.into_iter().map(|(k, v)| (t(k), v)).collect()) };
        Value::Map(vec![(t(name), Value::Map(vec![(t("plainParams"), pp)])), (t("type"), Value::Int(ty))])
    }

    #[test]
    fn splits_own_following_modules_and_ids_follow_positions() {
        let list = Value::Array(vec![
            module("FXSplit", 13, vec![("kParamModuleCount1", Value::Float(1.0)), ("kParamModuleCount2", Value::Float(2.0))]),
            module("FXDistortion", 0, vec![]),
            module("FXDelay", 4, vec![]),
            module("FXReverb", 6, vec![]),
            module("FXEQ", 7, vec![]),
        ]);
        let payload = Value::Map(vec![(t("FXRack1"), Value::Map(vec![(t("FX"), list)]))]);
        let mut tables = TableStore::new(None);
        let mut notes = BTreeMap::new();
        let setup = FxSetup::load(&payload, &mut tables, &mut notes);
        assert!(setup.racks[0].is_empty());
        let rack = &setup.racks[1];
        assert_eq!(rack.len(), 2, "split + EQ at top level");
        let NodeSetup::Split { module, bands } = &rack[0] else { panic!("split first") };
        assert_eq!(module.id, 100);
        assert_eq!(bands.iter().map(Vec::len).collect::<Vec<_>>(), vec![1, 2]);
        let NodeSetup::Effect(eq) = &rack[1] else { panic!() };
        assert_eq!(eq.id, 104);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn every_module_type_processes_finite_audio() {
        let mut tables = TableStore::new(None);
        for (kind, spec) in MODULES.iter().enumerate().filter(|(_, m)| m.is_dynamic()) {
            let values = spec.params.iter().map(|p| p.default_for(0)).collect();
            let setup = FxSetup { racks: [vec![NodeSetup::Effect(ModuleSetup { kind, id: 0, values, impulse: None })], vec![], vec![]] };
            let mut racks = FxRacks::new(&setup, 48000.0, 64);
            let _ = &mut tables;
            let mut peak = 0f32;
            for block in 0..1500 {
                let mut l: Vec<f32> = (0..64).map(|i| ((block * 64 + i) as f32 * 0.05).sin() * 0.5).collect();
                let mut r = l.clone();
                racks.process(0, &mut l, &mut r);
                for x in l.iter().chain(&r) {
                    assert!(x.is_finite(), "{} produced {x}", spec.name);
                    peak = peak.max(x.abs());
                }
            }
            assert!(peak < 8.0, "{} peaked at {peak}", spec.name);
        }
    }
    #[test]
    fn multiband_compressor_matches_probe_points() {
        // 523 Hz sits in the mid band; Serum's steady gain (fxmb probes): unity at
        // −31.6 dB peak, −14.4 dB at −0.5 dB, and +2.4 dB at −31.6 dB with RatioBelow 1
        // (the leakage into the other bands is lifted). At 130.8 Hz, on the low
        // crossover, wet 50 % still keeps a quiet sine's level (the dry is the band sum).
        let kind = params::kind_of("FXComp").unwrap();
        let spec = &MODULES[kind];
        let gain = |hz: f32, amp: f32, below: f32, wet: f32| {
            let mut values: Vec<f32> = spec.params.iter().map(|p| p.default_for(0)).collect();
            values[spec.index_of("kParamMultiband").unwrap()] = 1.0;
            values[spec.index_of("kParamRatioBelow").unwrap()] = below;
            values[spec.index_of("kParamWet").unwrap()] = wet;
            let setup = FxSetup { racks: [vec![NodeSetup::Effect(ModuleSetup { kind, id: 0, values, impulse: None })], vec![], vec![]] };
            let mut racks = FxRacks::new(&setup, 48000.0, 64);
            let (mut sum_in, mut sum_out) = (0f64, 0f64);
            for block in 0..412 {
                let x: Vec<f32> = (0..64).map(|i| (std::f32::consts::TAU * hz * (block * 64 + i) as f32 / 48000.0).sin() * amp).collect();
                let (mut l, mut r) = (x.clone(), x.clone());
                racks.process(0, &mut l, &mut r);
                if block >= 262 {
                    sum_in += x.iter().map(|&v| (v * v) as f64).sum::<f64>();
                    sum_out += l.iter().map(|&v| (v * v) as f64).sum::<f64>();
                }
            }
            10.0 * (sum_out / sum_in).log10()
        };
        let quiet = 10f32.powf(-31.6 / 20.0);
        let loud = 10f32.powf(-0.5 / 20.0);
        assert!(gain(523.25, quiet, 0.0, 100.0).abs() < 0.1, "{}", gain(523.25, quiet, 0.0, 100.0));
        assert!((gain(523.25, loud, 0.0, 100.0) + 14.36).abs() < 0.5, "{}", gain(523.25, loud, 0.0, 100.0));
        assert!((gain(523.25, quiet, 1.0, 100.0) - 2.42).abs() < 0.5, "{}", gain(523.25, quiet, 1.0, 100.0));
        assert!(gain(130.81, quiet, 0.0, 50.0).abs() < 0.2, "{}", gain(130.81, quiet, 0.0, 50.0));
    }
}
