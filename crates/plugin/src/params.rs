//! Host parameters, generated from the module parameter tables.

use lumen_engine::params::{ParamRef, MODULES};
use nih_plug::params::internals::ParamPtr;
use nih_plug::params::persist;
use nih_plug::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// One generated host parameter bound to a module parameter.
pub struct ModuleParam {
    pub id: String,
    pub group: String,
    /// Parameter name without the module, for the editor ("Attack").
    pub name: String,
    pub target: ParamRef,
    pub param: FloatParam,
}

pub struct LumenParams {
    /// Absolute path of the loaded `.SerumPreset`, saved with the host project.
    pub preset_path: RwLock<String>,
    /// Optional override for the wavetable directory.
    pub tables_dir: RwLock<String>,
    #[cfg(feature = "gui")]
    pub editor_state: Arc<nih_plug_egui::EguiState>,
    pub gain: FloatParam,
    /// Offsets from the preset's stored macro values, so a fresh instance sounds as saved.
    pub macros: [FloatParam; 8],
    pub modules: Vec<ModuleParam>,
}

fn macro_param(n: usize) -> FloatParam {
    FloatParam::new(format!("Macro {n}"), 0.0, FloatRange::Linear { min: -1.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(Arc::new(|v| format!("{:+.1}", v * 100.0)))
        .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|v| v / 100.0)))
        .with_smoother(SmoothingStyle::Linear(20.0))
}

/// "kParamAttack" → "Attack", "kParamMasterVolume" → "Master Volume".
pub fn pretty(name: &str) -> String {
    let base = name.strip_prefix("kParam").unwrap_or(name);
    let mut out = String::new();
    for (i, c) in base.char_indices() {
        if i > 0 && c.is_ascii_uppercase() && !base[..i].ends_with(|p: char| p.is_ascii_uppercase()) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// One host parameter per host-automatable parameter of every static module instance.
fn module_params() -> Vec<ModuleParam> {
    let mut out = Vec::new();
    for (kind, spec) in MODULES.iter().enumerate() {
        if spec.is_dynamic() || spec.name == "ModSlot" {
            continue;
        }
        for instance in 0..spec.instances {
            for index in 0..spec.automatable.min(spec.params.len()) {
                let target = ParamRef { kind, instance, index };
                let p = target.spec();
                let default = p.normalize(p.default_for(instance));
                let display = move |n: f32| target.spec().display(target.spec().denormalize(n));
                let parse = move |s: &str| target.spec().parse(s).map(|v| target.spec().normalize(v));
                let name = pretty(p.name);
                let label = if spec.instances > 1 { format!("{} {} {name}", spec.name, instance + 1) } else { format!("{} {name}", spec.name) };
                let param = FloatParam::new(label, default, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(Arc::new(display)).with_string_to_value(Arc::new(parse));
                out.push(ModuleParam { id: format!("{}{instance}.{}", spec.name, p.name), group: spec.name.to_owned(), name, target, param });
            }
        }
    }
    out
}

impl Default for LumenParams {
    fn default() -> Self {
        LumenParams {
            preset_path: RwLock::new(std::env::var("LUMEN_PRESET").unwrap_or_default()),
            tables_dir: RwLock::new(std::env::var("LUMEN_TABLES").unwrap_or_default()),
            #[cfg(feature = "gui")]
            editor_state: nih_plug_egui::EguiState::from_size(980, 760),
            gain: FloatParam::new("Output", util::db_to_gain(0.0), FloatRange::Skewed { min: util::db_to_gain(-48.0), max: util::db_to_gain(12.0), factor: FloatRange::gain_skew_factor(-48.0, 12.0) })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
                .with_string_to_value(formatters::s2v_f32_gain_to_db())
                .with_smoother(SmoothingStyle::Logarithmic(20.0)),
            macros: std::array::from_fn(|i| macro_param(i + 1)),
            modules: module_params(),
        }
    }
}

// SAFETY: every pointer refers to a field of `self`, which outlives the map (the
// host holds `Arc<LumenParams>`), and IDs are unique per parameter.
unsafe impl Params for LumenParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        let mut map = vec![("gain".to_owned(), self.gain.as_ptr(), String::new())];
        for (i, m) in self.macros.iter().enumerate() {
            map.push((format!("macro{}", i + 1), m.as_ptr(), "Macros".to_owned()));
        }
        for m in &self.modules {
            map.push((m.id.clone(), m.param.as_ptr(), m.group.clone()));
        }
        map
    }

    fn serialize_fields(&self) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::new();
        for (key, field) in [("preset-path", &self.preset_path), ("tables-dir", &self.tables_dir)] {
            if let Ok(json) = persist::serialize_field(field) {
                fields.insert(key.to_owned(), json);
            }
        }
        #[cfg(feature = "gui")]
        if let Ok(json) = persist::serialize_field(&*self.editor_state) {
            fields.insert("editor-state".to_owned(), json);
        }
        fields
    }

    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        for (key, field) in [("preset-path", &self.preset_path), ("tables-dir", &self.tables_dir)] {
            if let Some(json) = serialized.get(key) {
                if let Ok(value) = persist::deserialize_field::<String>(json) {
                    *field.write().unwrap() = value;
                }
            }
        }
        #[cfg(feature = "gui")]
        if let Some(json) = serialized.get("editor-state") {
            if let Ok(state) = persist::deserialize_field::<nih_plug_egui::EguiState>(json) {
                persist::PersistentField::set(&self.editor_state, state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_parameters_have_unique_ids_and_valid_defaults() {
        let params = LumenParams::default();
        let mut ids = std::collections::HashSet::new();
        for m in &params.modules {
            assert!(ids.insert(m.id.clone()), "duplicate id {}", m.id);
            let d = m.param.default_normalized_value();
            assert!((0.0..=1.0).contains(&d), "{} default {d}", m.id);
        }
        assert!(params.modules.len() > 300, "{} parameters", params.modules.len());
        assert_eq!(pretty("kParamMasterVolume"), "Master Volume");
        assert_eq!(pretty("kParamFXBus1Vol"), "FXBus1 Vol");
    }
}
