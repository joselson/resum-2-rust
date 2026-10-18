//! Lumen as a CLAP (and optionally VST3) instrument.
//!
//! The preset comes from the plugin's saved state, the editor's browser (with the
//! `gui` feature) or, for a fresh instance, the `LUMEN_PRESET` environment
//! variable. Wavetables are found from `LUMEN_TABLES` or by walking up from the
//! preset to a `Tables` directory.
//!
//! Every host-automatable module parameter is exposed, generated from the
//! parameter tables, with Serum's normalized range, default and display. The editor
//! publishes the loaded preset's values to the host when it opens and after each
//! load from its browser; until then the host shows defaults, and a parameter takes
//! effect once the host moves it (or restores a non-default value).

// vst3-sys macros inside `nih_export_vst3!` trip this future-incompatibility lint (upstream).
#![cfg_attr(feature = "vst3", allow(semicolon_in_expressions_from_non_local_macros))]

#[cfg(feature = "gui")]
mod editor;
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
mod library;
pub mod params;
#[cfg(feature = "standalone")]
pub mod standalone;
// Used by the editor; built (and tested) without it too.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
mod piano;

use lumen_engine::notes::NoteEvent as Note;
use lumen_engine::params::ParamRef;
use lumen_engine::{tables, Coverage, Patch, PreparedPatch, Synth, TableStore};
use lumen_preset::Preset;
use nih_plug::prelude::*;
use params::LumenParams;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const MACRO: ParamRef = ParamRef::new("Macro", 0, "kParamValue");

pub struct Lumen {
    params: Arc<LumenParams>,
    shared: Arc<Shared>,
    synth: Option<Synth>,
    /// Last normalized value seen per module parameter, to apply only host changes.
    seen: Vec<f32>,
}

/// What the editor shows about the loaded preset.
#[derive(Clone, Default)]
pub struct Status {
    pub name: String,
    pub message: String,
    pub loading: bool,
}

/// State shared by the audio thread, the editor and the background loader.
#[derive(Default)]
pub struct Shared {
    /// A patch the loader built, waiting for the audio thread to swap it in.
    incoming: Mutex<Option<PreparedPatch>>,
    /// The patch the audio thread swapped out, freed by the loader rather than on the audio thread.
    retired: Mutex<Option<PreparedPatch>>,
    /// Sample rate as f32 bits, for building effect racks off the audio thread.
    sample_rate: AtomicU32,
    pub status: Mutex<Status>,
    /// Normalized module parameter values of a preset loaded from the editor, for the
    /// editor to publish to the host (in `LumenParams::modules` order).
    pub sync: Mutex<Option<Vec<f32>>>,
    /// `.SerumPreset` files found by the last scan, sorted.
    pub presets: Mutex<Vec<PathBuf>>,
    /// Notes played on the editor's keyboard, drained by the audio thread.
    pub keys: Mutex<Vec<Note>>,
    /// The last preset loaded, unedited, keyed by preset path and tables directory:
    /// hosts re-initialize often (sample rate, block size, reset), and reloading
    /// tables and samples each time is slow.
    loaded: Mutex<Option<(String, String, Arc<Patch>)>>,
}

pub enum Task {
    Load(PathBuf),
    Scan(PathBuf),
}

impl Default for Lumen {
    fn default() -> Self {
        Lumen { params: Arc::new(LumenParams::default()), shared: Arc::default(), synth: None, seen: Vec::new() }
    }
}

/// Loads a preset with wavetables from `tables_dir`, or the directory found above the preset.
fn load_preset(path: &Path, tables_dir: &str) -> Result<(Patch, Coverage), String> {
    let root = (!tables_dir.is_empty()).then(|| PathBuf::from(tables_dir)).or_else(|| tables::infer_root(path));
    let mut store = TableStore::new(root);
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let preset = Preset::read(&bytes).map_err(|e| e.to_string())?;
    Ok(Patch::from_payload(&preset.payload, &mut store))
}

fn summary(coverage: &Coverage) -> String {
    let mut parts = vec![format!("{} mod routes", coverage.routes)];
    if !coverage.unsupported.is_empty() {
        parts.push(format!("{} unsupported features", coverage.unsupported.len()));
    }
    if !coverage.table_errors.is_empty() {
        parts.push(format!("{} asset load errors", coverage.table_errors.len()));
    }
    parts.join(", ")
}

fn preset_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Loads the configured preset, falling back to the init patch on any failure.
fn initial_patch(preset_path: &str, tables_dir: &str, shared: &Shared) -> Patch {
    let mut status = Status { name: "Init".into(), ..Status::default() };
    let patch = if preset_path.is_empty() {
        Patch::init(&mut TableStore::new((!tables_dir.is_empty()).then(|| PathBuf::from(tables_dir))))
    } else {
        let path = Path::new(preset_path);
        match load_preset(path, tables_dir) {
            Ok((patch, coverage)) => {
                nih_log!("loaded {} ({})", path.display(), summary(&coverage));
                for e in &coverage.table_errors {
                    nih_log!("load error: {e}");
                }
                status = Status { name: preset_name(path), message: summary(&coverage), loading: false };
                patch
            }
            Err(e) => {
                nih_error!("could not load preset {}: {e}", path.display());
                status.message = format!("could not load {}: {e}", path.display());
                Patch::init(&mut TableStore::new(None))
            }
        }
    };
    *shared.status.lock().unwrap() = status;
    patch
}

/// Normalized host values for a patch, in `LumenParams::modules` order.
fn host_values(params: &LumenParams, patch: &Patch) -> Vec<f32> {
    params.modules.iter().map(|m| m.target.spec().normalize(patch.param(m.target))).collect()
}

/// All `.SerumPreset` files under `dir`, sorted by path.
fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(_) if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("SerumPreset")) => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

fn run_task(task: Task, params: &LumenParams, shared: &Shared) {
    match task {
        Task::Load(path) => {
            shared.status.lock().unwrap().loading = true;
            // Free the patch the audio thread last swapped out.
            drop(shared.retired.lock().unwrap().take());
            let tables_dir = params.tables_dir.read().unwrap().clone();
            let mut status = shared.status.lock().unwrap().clone();
            match load_preset(&path, &tables_dir) {
                Ok((patch, coverage)) => {
                    let values = host_values(params, &patch);
                    let rate = f32::from_bits(shared.sample_rate.load(Ordering::Acquire));
                    let path_text = path.to_string_lossy().into_owned();
                    let prepared = PreparedPatch::new(Arc::new(patch.clone()), if rate > 0.0 { rate } else { 48000.0 });
                    *shared.loaded.lock().unwrap() = Some((path_text.clone(), tables_dir, Arc::new(patch)));
                    // Replaces (and frees, here) any load the audio thread has not picked up yet.
                    *shared.incoming.lock().unwrap() = Some(prepared);
                    *params.preset_path.write().unwrap() = path_text;
                    *shared.sync.lock().unwrap() = Some(values);
                    status.name = preset_name(&path);
                    status.message = summary(&coverage);
                }
                Err(e) => status.message = format!("could not load {}: {e}", path.display()),
            }
            status.loading = false;
            *shared.status.lock().unwrap() = status;
        }
        Task::Scan(dir) => *shared.presets.lock().unwrap() = scan(&dir),
    }
}

impl Plugin for Lumen {
    const NAME: &'static str = "Lumen";
    const VENDOR: &'static str = "Lumen";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout { main_input_channels: None, main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let (params, shared) = (self.params.clone(), self.shared.clone());
        Box::new(move |task| run_task(task, &params, &shared))
    }

    #[cfg(feature = "gui")]
    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.shared.clone(), async_executor)
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        // Runs off the audio thread, so file I/O and table building are safe here.
        self.shared.sample_rate.store(config.sample_rate.to_bits(), Ordering::Release);
        drop(self.shared.incoming.lock().unwrap().take());
        let (preset_path, tables_dir) = (self.params.preset_path.read().unwrap().clone(), self.params.tables_dir.read().unwrap().clone());
        let cached = self.shared.loaded.lock().unwrap().as_ref().filter(|(p, t, _)| *p == preset_path && *t == tables_dir).map(|(_, _, patch)| patch.clone());
        let patch = match cached {
            Some(patch) => patch,
            None => {
                let patch = Arc::new(initial_patch(&preset_path, &tables_dir, &self.shared));
                *self.shared.loaded.lock().unwrap() = Some((preset_path, tables_dir, patch.clone()));
                patch
            }
        };
        // The synth gets its own copy, so live edits never clone a shared patch on the audio thread.
        let mut synth = Synth::new(config.sample_rate, Arc::new((*patch).clone()));
        // Values restored from a project that differ from the default are user edits:
        // apply them over the preset. Defaults leave the preset's own value in place.
        self.seen = self.params.modules.iter().map(|m| m.param.modulated_normalized_value()).collect();
        for m in &self.params.modules {
            let n = m.param.modulated_normalized_value();
            if (n - m.param.default_normalized_value()).abs() > 1e-6 {
                synth.set_param(m.target, m.target.spec().denormalize(n));
            }
        }
        // The editor publishes these when it opens, so the host shows the preset's values.
        *self.shared.sync.lock().unwrap() = Some(host_values(&self.params, synth.patch()));
        self.synth = Some(synth);
        true
    }

    fn reset(&mut self) {
        if let Some(s) = &mut self.synth {
            s.all_notes_off(true);
        }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let Some(synth) = &mut self.synth else { return ProcessStatus::Normal };
        // try_lock: the loader holds these locks only to move a patch in or out.
        if let Some(next) = self.shared.incoming.try_lock().ok().and_then(|mut slot| slot.take()) {
            let old = synth.set_patch(next);
            if let Ok(mut retired) = self.shared.retired.try_lock() {
                if retired.is_none() {
                    *retired = Some(old);
                }
            }
        }
        if let Ok(mut keys) = self.shared.keys.try_lock() {
            for key in keys.drain(..) {
                synth.event(key);
            }
        }
        if let Some(tempo) = context.transport().tempo {
            synth.set_bpm(tempo as f32);
        }
        for (i, p) in self.params.macros.iter().enumerate() {
            let stored = synth.patch().param(MACRO.on(i)) / 100.0;
            synth.set_macro(i, stored + p.smoothed.next_step(buffer.samples() as u32));
        }
        for (m, seen) in self.params.modules.iter().zip(&mut self.seen) {
            let n = m.param.modulated_normalized_value();
            if n != *seen {
                *seen = n;
                synth.set_param(m.target, m.target.spec().denormalize(n));
            }
        }

        let samples = buffer.samples();
        let [left, right] = buffer.as_slice() else { return ProcessStatus::Normal };
        let mut pos = 0;
        let mut next = context.next_event();
        while pos < samples {
            while let Some(event) = next.filter(|e| e.timing() as usize <= pos) {
                handle_event(synth, event);
                next = context.next_event();
            }
            let end = next.map(|e| (e.timing() as usize).clamp(pos + 1, samples)).unwrap_or(samples);
            synth.process(&mut left[pos..end], &mut right[pos..end]);
            pos = end;
        }
        // Events stamped past the buffer end are applied now rather than dropped.
        while let Some(event) = next {
            handle_event(synth, event);
            next = context.next_event();
        }

        for i in 0..samples {
            let g = self.params.gain.smoothed.next();
            left[i] *= g;
            right[i] *= g;
        }
        ProcessStatus::KeepAlive
    }
}

fn handle_event(synth: &mut Synth, event: NoteEvent<()>) {
    match event {
        NoteEvent::NoteOn { note, velocity, .. } => synth.event(Note::On { note, velocity }),
        NoteEvent::NoteOff { note, velocity, .. } => synth.event(Note::Off { note, velocity }),
        NoteEvent::MidiPitchBend { value, .. } => synth.set_pitch_bend(value * 2.0 - 1.0),
        NoteEvent::MidiCC { cc: 1, value, .. } => synth.set_mod_wheel(value),
        NoteEvent::MidiCC { cc: 120 | 123, .. } => synth.all_notes_off(cc_is_hard(event)),
        NoteEvent::MidiChannelPressure { pressure, .. } => synth.set_aftertouch(pressure),
        _ => {}
    }
}

/// CC 120 (all sound off) cuts immediately; CC 123 (all notes off) releases.
fn cc_is_hard(event: NoteEvent<()>) -> bool {
    matches!(event, NoteEvent::MidiCC { cc: 120, .. })
}

impl ClapPlugin for Lumen {
    const CLAP_ID: &'static str = "dev.lumen.synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Wavetable synthesizer that reads Serum 2 presets");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo];
}

nih_export_clap!(Lumen);

#[cfg(feature = "vst3")]
impl Vst3Plugin for Lumen {
    const VST3_CLASS_ID: [u8; 16] = *b"LumenSynthVst3!!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

#[cfg(feature = "vst3")]
nih_export_vst3!(Lumen);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_scans_and_rejects_bad_presets_without_side_effects() {
        let dir = std::env::temp_dir().join(format!("lumen-loader-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let path = dir.join("sub").join("Probe.SerumPreset");
        std::fs::write(&path, b"not a preset").unwrap();
        let (params, shared) = (LumenParams::default(), Shared::default());

        run_task(Task::Scan(dir.clone()), &params, &shared);
        assert_eq!(*shared.presets.lock().unwrap(), vec![path.clone()]);

        run_task(Task::Load(path.clone()), &params, &shared);
        let status = shared.status.lock().unwrap().clone();
        assert!(status.message.starts_with("could not load") && !status.loading, "{}", status.message);
        assert!(shared.incoming.lock().unwrap().is_none() && shared.sync.lock().unwrap().is_none(), "a failed load changes nothing");
        assert!(params.preset_path.read().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn host_values_round_trip_the_patch() {
        let params = LumenParams::default();
        let patch = Patch::init(&mut TableStore::new(None));
        let values = host_values(&params, &patch);
        assert_eq!(values.len(), params.modules.len());
        for (m, n) in params.modules.iter().zip(&values) {
            let back = m.target.spec().denormalize(*n);
            assert!((back - patch.param(m.target)).abs() <= 1e-3 * (1.0 + back.abs()), "{}: {back} vs {}", m.id, patch.param(m.target));
        }
    }
}
