//! The parameter layer: every module's parameters, their stored ("plain") value
//! curves, defaults and option lists, and a flat layout over all module instances.
//!
//! Plain values are what presets store. Normalized values ∈ [0, 1] are what hosts
//! automate and what mod amounts (±100%) are applied in.

pub mod serum2;

pub use serum2::ALL as MODULES;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// plain = min + (max − min)·n, optionally rounded to integers.
    Linear { min: f32, max: f32, integer: bool },
    /// plain = min + (max − min)·nᵏ
    Power { min: f32, max: f32, k: f32 },
    /// plain = min·(max/min)ⁿ
    Exp { min: f32, max: f32 },
    /// 0 or 1.
    Toggle,
    /// Index into `ParamSpec::options`.
    Choice,
}

/// Where a parameter's curve and default come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// Measured through Serum 2's host parameter interface.
    Measured,
    /// Not a host parameter; inferred from factory presets.
    Presets,
    /// Option names from the plugin's name table.
    NameTable,
    Assumed,
    /// Enumerated by Serum but never observed.
    Unknown,
}

#[derive(Debug)]
pub struct ParamSpec {
    /// Preset key, e.g. `kParamAttack`.
    pub name: &'static str,
    /// Serum host parameter name of the first instance.
    pub host: Option<&'static str>,
    pub curve: Curve,
    /// Per-instance defaults (plain); a single entry applies to all instances.
    pub defaults: &'static [f32],
    /// Stored option names for `Curve::Choice`.
    pub options: &'static [&'static str],
    /// Display names matching `options`.
    pub displays: &'static [&'static str],
    pub evidence: Evidence,
}

#[derive(Debug)]
pub struct ModuleSpec {
    /// Module type as mod slots name it (`destModuleTypeString`).
    pub name: &'static str,
    /// Preset section path with `#` for the instance number.
    pub section: &'static str,
    pub instances: usize,
    /// Parameters before this index are host-automatable; the index is `destModuleParamID`.
    pub automatable: usize,
    pub params: &'static [ParamSpec],
}

impl ParamSpec {
    pub fn default_for(&self, instance: usize) -> f32 {
        self.defaults.get(instance).or(self.defaults.last()).copied().unwrap_or(0.0)
    }

    fn bounds(&self) -> (f32, f32) {
        match self.curve {
            Curve::Linear { min, max, .. } | Curve::Power { min, max, .. } | Curve::Exp { min, max } => (min, max),
            Curve::Toggle => (0.0, 1.0),
            Curve::Choice => (0.0, self.options.len().saturating_sub(1) as f32),
        }
    }

    pub fn normalize(&self, plain: f32) -> f32 {
        let (min, max) = self.bounds();
        if max == min {
            return 0.0;
        }
        let n = match self.curve {
            Curve::Power { k, .. } => ((plain - min) / (max - min)).max(0.0).powf(1.0 / k),
            Curve::Exp { .. } if min > 0.0 && plain > 0.0 => (plain / min).ln() / (max / min).ln(),
            _ => (plain - min) / (max - min),
        };
        n.clamp(0.0, 1.0)
    }

    pub fn denormalize(&self, norm: f32) -> f32 {
        let n = norm.clamp(0.0, 1.0);
        let (min, max) = self.bounds();
        match self.curve {
            Curve::Linear { integer: true, .. } | Curve::Choice => (min + (max - min) * n).round(),
            Curve::Linear { .. } => min + (max - min) * n,
            Curve::Power { k, .. } => min + (max - min) * n.powf(k),
            Curve::Exp { .. } if min > 0.0 => min * (max / min).powf(n),
            Curve::Exp { .. } => min + (max - min) * n,
            Curve::Toggle => if n >= 0.5 { 1.0 } else { 0.0 },
        }
    }

    /// Applies a normalized offset (mod amount × source) to a plain value.
    #[inline]
    pub fn offset(&self, plain: f32, norm_delta: f32) -> f32 {
        if norm_delta == 0.0 {
            plain
        } else {
            self.denormalize(self.normalize(plain) + norm_delta)
        }
    }

    /// Option index for a stored name.
    pub fn option_index(&self, stored: &str) -> Option<usize> {
        self.options.iter().position(|o| *o == stored)
    }

    /// Plain value from text written by `display` (or typed by a user).
    pub fn parse(&self, text: &str) -> Option<f32> {
        let t = text.trim();
        match self.curve {
            Curve::Choice => self.displays.iter().position(|d| d.eq_ignore_ascii_case(t)).or_else(|| self.option_index(t)).map(|i| i as f32),
            Curve::Toggle => match t.to_ascii_lowercase().as_str() {
                "on" | "1" | "true" => Some(1.0),
                "off" | "0" | "false" => Some(0.0),
                _ => None,
            },
            _ => t.parse::<f32>().ok().filter(|v| v.is_finite()),
        }
    }

    pub fn display(&self, plain: f32) -> String {
        match self.curve {
            Curve::Choice => {
                let i = plain.round().max(0.0) as usize;
                self.displays.get(i).or(self.options.get(i)).map(|s| s.to_string()).unwrap_or_else(|| format!("#{i}"))
            }
            Curve::Toggle => if plain >= 0.5 { "On".into() } else { "Off".into() },
            Curve::Linear { integer: true, .. } => format!("{}", plain.round()),
            _ => {
                // Five significant figures: round-trips through f32 normalization.
                let magnitude = if plain == 0.0 { 0 } else { plain.abs().log10().floor() as i32 };
                format!("{plain:.*}", (4 - magnitude).clamp(0, 6) as usize)
            }
        }
    }
}

impl ModuleSpec {
    /// Index of a parameter by preset key.
    pub const fn index_of(&self, name: &str) -> Option<usize> {
        let mut i = 0;
        while i < self.params.len() {
            if str_eq(self.params[i].name, name) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    pub fn section_path(&self, instance: usize) -> String {
        self.section.replace('#', &instance.to_string())
    }
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// A module type by position in `MODULES`.
pub type ModuleKind = usize;

pub const fn kind_of(name: &str) -> Option<ModuleKind> {
    let mut i = 0;
    while i < MODULES.len() {
        if str_eq(MODULES[i].name, name) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Index of `param` within `module`; panics at compile time when used in a const.
pub const fn index(module: &str, param: &str) -> usize {
    let Some(kind) = kind_of(module) else { panic!("unknown module") };
    let Some(index) = MODULES[kind].index_of(param) else { panic!("unknown parameter") };
    index
}

impl ModuleSpec {
    /// FX modules have no fixed instances: they live in rack lists and are
    /// addressed as rack × 100 + position.
    pub const fn is_dynamic(&self) -> bool {
        self.instances == 0
    }
}

/// One parameter of one module instance, resolvable at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamRef {
    pub kind: ModuleKind,
    pub instance: usize,
    pub index: usize,
}

impl ParamRef {
    /// Panics (at compile time when used in a const) if the module or parameter is unknown.
    pub const fn new(module: &str, instance: usize, param: &str) -> ParamRef {
        let Some(kind) = kind_of(module) else { panic!("unknown module") };
        let Some(index) = MODULES[kind].index_of(param) else { panic!("unknown parameter") };
        assert!(instance < MODULES[kind].instances, "instance out of range");
        ParamRef { kind, instance, index }
    }

    pub fn spec(self) -> &'static ParamSpec {
        &MODULES[self.kind].params[self.index]
    }

    pub fn module(self) -> &'static ModuleSpec {
        MODULES[self.kind]
    }

    /// Same parameter on another instance of the module.
    pub const fn on(self, instance: usize) -> ParamRef {
        ParamRef { instance, ..self }
    }

    /// Whether this targets a dynamic (FX) module, which has no slot in `Values`.
    pub const fn is_dynamic(self) -> bool {
        MODULES[self.kind].is_dynamic()
    }

    /// Position in the flat layout (static modules only).
    #[inline]
    pub const fn flat(self) -> usize {
        BASES[self.kind] + self.instance * MODULES[self.kind].params.len() + self.index
    }
}

const fn bases() -> [usize; MODULES.len() + 1] {
    let mut out = [0; MODULES.len() + 1];
    let mut i = 0;
    while i < MODULES.len() {
        out[i + 1] = out[i] + MODULES[i].instances * MODULES[i].params.len();
        i += 1;
    }
    out
}

const BASES: [usize; MODULES.len() + 1] = bases();

/// Total parameters across all module instances.
pub const TOTAL: usize = BASES[MODULES.len()];

/// Plain values for every parameter of every module instance.
#[derive(Clone)]
pub struct Values {
    values: Box<[f32; TOTAL]>,
}

impl Default for Values {
    fn default() -> Self {
        let mut values = Box::new([0.0; TOTAL]);
        for (kind, m) in MODULES.iter().enumerate() {
            for instance in 0..m.instances {
                for (index, p) in m.params.iter().enumerate() {
                    values[ParamRef { kind, instance, index }.flat()] = p.default_for(instance);
                }
            }
        }
        Values { values }
    }
}

impl Values {
    #[inline]
    pub fn get(&self, p: ParamRef) -> f32 {
        self.values[p.flat()]
    }

    pub fn set(&mut self, p: ParamRef, plain: f32) {
        self.values[p.flat()] = plain;
    }

    pub fn is_default(&self, p: ParamRef) -> bool {
        (self.get(p) - p.spec().default_for(p.instance)).abs() <= 1e-6 * (1.0 + self.get(p).abs())
    }

    /// Stored option name of a choice parameter.
    pub fn choice(&self, p: ParamRef) -> &'static str {
        p.spec().options.get(self.get(p).round().max(0.0) as usize).copied().unwrap_or("")
    }

    pub fn flag(&self, p: ParamRef) -> bool {
        self.get(p) >= 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_follow_serum_ids() {
        // destModuleParamID values seen in factory mod slots.
        assert_eq!(ParamRef::new("Oscillator", 0, "kParamVolume").index, 1);
        assert_eq!(ParamRef::new("VoiceFilter", 0, "kParamFreq").index, 3);
        assert_eq!(ParamRef::new("WTOsc", 0, "kParamTablePos").index, 6);
        assert_eq!(ParamRef::new("Env", 0, "kParamDecay").index, 2);
        assert_eq!(ParamRef::new("Global", 0, "kParamMasterTuning").index, 1);
        assert_eq!(ParamRef::new("RoutingSlot", 0, "kParamFXBus1Level").index, 1);
    }

    #[test]
    fn measured_defaults_and_curves() {
        let v = Values::default();
        let attack = ParamRef::new("Env", 0, "kParamAttack");
        assert!((v.get(attack) - 0.001).abs() < 1e-5, "Env 1 attack is 1 ms");
        assert!((v.get(attack.on(1)) - 0.000515).abs() < 1e-5);
        // 32·n⁵: n = 0.5 → 1 s.
        assert!((attack.spec().denormalize(0.5) - 1.0).abs() < 1e-4);
        assert_eq!(v.choice(ParamRef::new("VoiceFilter", 0, "kParamType")), "MgL12");
        assert!(v.flag(ParamRef::new("Oscillator", 0, "kParamEnable")));
        assert!(!v.flag(ParamRef::new("Oscillator", 1, "kParamEnable")));
        assert_eq!(v.choice(ParamRef::new("RoutingSlot", 0, "kParamRoutingDest")), "kRoutingDestFilter");
        assert_eq!(v.choice(ParamRef::new("RoutingSlot", 1, "kParamRoutingDest")), "kRoutingDestMaster");
    }

    #[test]
    fn normalize_round_trips() {
        for m in MODULES {
            for p in m.params {
                if matches!(p.curve, Curve::Choice | Curve::Toggle | Curve::Linear { integer: true, .. }) {
                    continue;
                }
                for n in [0.0, 0.3, 1.0] {
                    let back = p.normalize(p.denormalize(n));
                    assert!((back - n).abs() < 1e-3, "{}.{}: {n} -> {back}", m.name, p.name);
                }
            }
        }
    }

    #[test]
    fn display_and_parse_round_trip() {
        for m in MODULES {
            for p in m.params {
                for n in [0.0, 0.37, 1.0] {
                    let plain = p.denormalize(n);
                    let text = p.display(plain);
                    let back = p.parse(&text).unwrap_or_else(|| panic!("{}.{}: cannot parse {text:?}", m.name, p.name));
                    assert_eq!(p.display(back), text, "{}.{}", m.name, p.name);
                }
            }
        }
    }

    #[test]
    fn layout_is_dense() {
        let last = MODULES.iter().rposition(|m| !m.is_dynamic()).unwrap();
        assert!(MODULES[last + 1..].iter().all(|m| m.is_dynamic()), "dynamic modules come last");
        let m = MODULES[last];
        let p = ParamRef { kind: last, instance: m.instances - 1, index: m.params.len() - 1 };
        assert_eq!(p.flat(), TOTAL - 1);
    }
}
