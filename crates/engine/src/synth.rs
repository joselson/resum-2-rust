//! Polyphonic voice management.

use crate::notes::{NoteEvent, NoteProcessor, Passthrough};
use crate::params::ParamRef;
use crate::patch::Patch;
use crate::fx::FxRacks;
use crate::voice::{Bus, Controls, NoteStart, Voice, BLOCK, BUS_CHANNELS};
use std::sync::Arc;

pub const MAX_VOICES: usize = 32;

const POLY: ParamRef = ParamRef::new("Global", 0, "kParamPolyCount");
const MONO: ParamRef = ParamRef::new("Global", 0, "kParamMonoToggle");
const LEGATO: ParamRef = ParamRef::new("Global", 0, "kParamLegato");
const PORTAMENTO: ParamRef = ParamRef::new("Global", 0, "kParamPortamentoTime");
const PORTA_ALWAYS: ParamRef = ParamRef::new("Global", 0, "kParamPortaAlways");
const MACRO: ParamRef = ParamRef::new("Macro", 0, "kParamValue");
const MASTER_VOLUME: ParamRef = ParamRef::new("Global", 0, "kParamMasterVolume");
const DIRECT_VOL: ParamRef = ParamRef::new("Global", 0, "kParamDirectVol");
const BUS_VOL: [ParamRef; 2] = [ParamRef::new("Global", 0, "kParamFXBus1Vol"), ParamRef::new("Global", 0, "kParamFXBus2Vol")];
const BUS_DEST: [ParamRef; 2] = [ParamRef::new("Global", 0, "kParamFXBus1Dest"), ParamRef::new("Global", 0, "kParamFXBus2Dest")];

/// Stored macro values (0–100) as [0, 1].
fn patch_macros(patch: &Patch) -> [f32; 8] {
    std::array::from_fn(|i| patch.param(MACRO.on(i)) / 100.0)
}

/// A patch with its effect racks built for one sample rate, so swapping it into a
/// running synth does not allocate.
pub struct PreparedPatch {
    patch: Arc<Patch>,
    fx: FxRacks,
    sample_rate: f32,
}

impl PreparedPatch {
    pub fn new(patch: Arc<Patch>, sample_rate: f32) -> PreparedPatch {
        PreparedPatch { fx: FxRacks::new(&patch.fx, sample_rate, BLOCK), patch, sample_rate }
    }

    pub fn patch(&self) -> &Arc<Patch> {
        &self.patch
    }
}

pub struct Synth {
    patch: Arc<Patch>,
    voices: Vec<Voice>,
    controls: Controls,
    age: u64,
    alternate: bool,
    /// Held notes in press order, for mono note priority (last note wins).
    held: Vec<u8>,
    last_pitch: Option<f32>,
    /// Arp/clip stage between incoming notes and the voices.
    notes: Box<dyn NoteProcessor>,
    note_out: Vec<NoteEvent>,
    fx: FxRacks,
    buses: Box<[[f32; BLOCK]; BUS_CHANNELS]>,
}

impl Synth {
    pub fn new(sample_rate: f32, patch: Arc<Patch>) -> Synth {
        Synth {
            voices: (0..MAX_VOICES as u32).map(|i| Voice::new(0x9e37_79b9u32.wrapping_mul(i + 1))).collect(),
            controls: Controls { sample_rate, bpm: 120.0, mod_wheel: 0.0, pitch_bend: 0.0, aftertouch: 0.0, macros: patch_macros(&patch) },
            age: 0,
            alternate: false,
            held: Vec::with_capacity(128),
            last_pitch: None,
            notes: Box::new(Passthrough),
            note_out: Vec::with_capacity(64),
            fx: FxRacks::new(&patch.fx, sample_rate, BLOCK),
            buses: Box::new([[0.0; BLOCK]; BUS_CHANNELS]),
            patch,
        }
    }

    pub fn patch(&self) -> &Arc<Patch> {
        &self.patch
    }

    /// Swaps in a prepared patch and returns the previous one, so the caller can free
    /// it off the audio thread. Sounding voices are cut so they never mix settings;
    /// macros reset to the new patch's stored values.
    pub fn set_patch(&mut self, mut next: PreparedPatch) -> PreparedPatch {
        if next.sample_rate != self.controls.sample_rate {
            next.fx = FxRacks::new(&next.patch.fx, self.controls.sample_rate, BLOCK);
            next.sample_rate = self.controls.sample_rate;
        }
        self.controls.macros = patch_macros(&next.patch);
        std::mem::swap(&mut self.patch, &mut next.patch);
        std::mem::swap(&mut self.fx, &mut next.fx);
        self.all_notes_off(true);
        next
    }

    /// Live-edits one parameter (host automation). The synth owns its patch, so this
    /// does not clone it on the audio thread; voices pick the value up next block.
    pub fn set_param(&mut self, p: ParamRef, plain: f32) {
        if p.is_dynamic() {
            return;
        }
        Arc::make_mut(&mut self.patch).values.set(p, plain);
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.controls.sample_rate = sample_rate;
        self.fx = FxRacks::new(&self.patch.fx, sample_rate, BLOCK);
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.controls.bpm = bpm.clamp(20.0, 999.0);
        self.fx.set_bpm(self.controls.bpm);
    }

    /// `value` ∈ [-1, 1].
    pub fn set_pitch_bend(&mut self, value: f32) {
        self.controls.pitch_bend = value.clamp(-1.0, 1.0);
    }

    pub fn set_mod_wheel(&mut self, value: f32) {
        self.controls.mod_wheel = value.clamp(0.0, 1.0);
    }

    /// `value` ∈ [0, 1].
    pub fn set_macro(&mut self, index: usize, value: f32) {
        if let Some(m) = self.controls.macros.get_mut(index) {
            *m = value.clamp(0.0, 1.0);
        }
    }

    pub fn macro_value(&self, index: usize) -> Option<f32> {
        self.controls.macros.get(index).copied()
    }

    pub fn set_aftertouch(&mut self, value: f32) {
        self.controls.aftertouch = value.clamp(0.0, 1.0);
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.is_active()).count()
    }

    /// Delivers a note event through the note-processing stage.
    pub fn event(&mut self, event: NoteEvent) {
        let mut out = std::mem::take(&mut self.note_out);
        self.notes.process(&[event], 0, &mut out);
        for e in out.drain(..) {
            match e {
                NoteEvent::On { note, velocity } => self.note_on(note, velocity),
                NoteEvent::Off { note, velocity } => self.note_off(note, velocity),
            }
        }
        self.note_out = out;
    }

    /// Replaces the note-processing stage (arpeggiator, clip player).
    pub fn set_note_processor(&mut self, processor: Box<dyn NoteProcessor>) {
        self.notes = processor;
    }

    /// `velocity` ∈ [0, 1].
    fn note_on(&mut self, note: u8, velocity: f32) {
        let note = note.min(127);
        self.age += 1;
        self.alternate = !self.alternate;
        self.held.retain(|&n| n != note);
        self.held.push(note);
        let v = &self.patch.values;
        let glide_from = if v.get(PORTAMENTO) > 0.0 && (v.flag(PORTA_ALWAYS) || self.held.len() > 1) { self.last_pitch } else { None };
        let (mono, legato, poly) = (v.flag(MONO), v.flag(LEGATO), v.get(POLY).round().clamp(1.0, MAX_VOICES as f32) as usize);

        let start = NoteStart { note, velocity, age: self.age, legato: false, glide_from, alternate: self.alternate };
        if mono {
            let legato = legato && self.voices[0].is_active() && self.held.len() > 1;
            self.voices[0].start(&self.patch, NoteStart { legato, ..start });
        } else {
            let index = self.allocate(note, poly);
            self.voices[index].start(&self.patch, start);
        }
        self.last_pitch = Some(note as f32);
    }

    fn note_off(&mut self, note: u8, velocity: f32) {
        self.held.retain(|&n| n != note);
        let v = &self.patch.values;
        if v.flag(MONO) {
            // Fall back to the most recent still-held note.
            if let Some(&prev) = self.held.last() {
                if self.voices[0].note == note {
                    self.age += 1;
                    let start = NoteStart {
                        note: prev,
                        velocity: 0.8,
                        age: self.age,
                        legato: v.flag(LEGATO),
                        glide_from: (v.get(PORTAMENTO) > 0.0).then_some(note as f32),
                        alternate: self.alternate,
                    };
                    self.voices[0].start(&self.patch, start);
                }
                return;
            }
        }
        for v in &mut self.voices {
            if v.held && v.note == note {
                v.release(velocity);
            }
        }
    }

    pub fn all_notes_off(&mut self, hard: bool) {
        self.held.clear();
        self.notes.reset();
        for v in &mut self.voices {
            if hard { v.kill() } else { v.release(0.0) }
        }
    }

    /// Reuse a voice already on this note, then a free one, then steal the
    /// oldest released voice, then the oldest overall.
    fn allocate(&mut self, note: u8, poly: usize) -> usize {
        let pool = &self.voices[..poly];
        if let Some(i) = pool.iter().position(|v| v.is_active() && v.note == note) {
            return i;
        }
        if let Some(i) = pool.iter().position(|v| !v.is_active()) {
            return i;
        }
        let oldest = |held: bool| pool.iter().enumerate().filter(|(_, v)| v.held == held).min_by_key(|(_, v)| v.age).map(|(i, _)| i);
        let i = oldest(false).or_else(|| oldest(true)).unwrap_or(0);
        self.voices[i].kill();
        i
    }

    /// Renders into `left`/`right` (overwriting them). Lengths must match.
    ///
    /// Per block: voices fill the main, direct and FX-bus buses; bus racks run and
    /// feed their destinations; the main rack runs; direct joins after it; master
    /// volume applies last.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len());
        let v = &self.patch.values;
        // Gains stored as amplitude with 0.5 = 0 dB; master has +3 dB at full scale.
        let master = v.get(MASTER_VOLUME) * std::f32::consts::SQRT_2;
        let direct_gain = v.get(DIRECT_VOL) * 2.0;
        let bus_gain = BUS_VOL.map(|p| v.get(p) * 2.0);
        // Bus destination: 1 = direct output, otherwise the main rack (assumed).
        let bus_to_direct = BUS_DEST.map(|p| v.get(p).round() as i32 == 1);
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            let n = l.len();
            let buses = &mut *self.buses;
            for ch in buses.iter_mut() {
                ch[..n].fill(0.0);
            }
            let mut newest: Option<(usize, u64)> = None;
            for (i, voice) in self.voices.iter_mut().enumerate() {
                if voice.is_active() {
                    voice.render(&self.patch, &self.controls, buses, n);
                    if newest.is_none_or(|(_, age)| voice.age > age) {
                        newest = Some((i, voice.age));
                    }
                }
            }
            // FX follow the newest voice's modulation (they are shared, not per voice).
            match newest {
                Some((i, _)) => self.fx.set_modulation(self.voices[i].fx_offsets()),
                None => self.fx.set_modulation(&[]),
            }
            for (b, bus) in [Bus::Fx1, Bus::Fx2].into_iter().enumerate() {
                let k = bus as usize * 2;
                if self.fx.is_empty(b + 1) && buses[k][..n].iter().all(|&x| x == 0.0) {
                    continue;
                }
                let [bl, br] = buses.get_disjoint_mut([k, k + 1]).expect("distinct bus channels");
                self.fx.process(b + 1, &mut bl[..n], &mut br[..n]);
                let target = if bus_to_direct[b] { Bus::Direct } else { Bus::Main } as usize * 2;
                let [bl, br, tl, tr] = buses.get_disjoint_mut([k, k + 1, target, target + 1]).expect("distinct bus channels");
                for i in 0..n {
                    tl[i] += bl[i] * bus_gain[b];
                    tr[i] += br[i] * bus_gain[b];
                }
            }
            let [ml, mr] = buses.get_disjoint_mut([0, 1]).expect("distinct bus channels");
            self.fx.process(0, &mut ml[..n], &mut mr[..n]);
            for i in 0..n {
                l[i] = (buses[0][i] + buses[2][i] * direct_gain) * master;
                r[i] = (buses[1][i] + buses[3][i] * direct_gain) * master;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TableStore;

    fn synth() -> Synth {
        let mut tables = TableStore::new(None);
        Synth::new(48000.0, Arc::new(Patch::init(&mut tables)))
    }

    fn render(s: &mut Synth, samples: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; samples], vec![0.0; samples]);
        s.process(&mut l, &mut r);
        (l, r)
    }

    #[test]
    fn init_patch_plays_and_releases() {
        let mut s = synth();
        s.event(NoteEvent::On { note: 60, velocity: 1.0 });
        let (l, _) = render(&mut s, 4800);
        let peak = l.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.1 && peak < 2.0, "peak {peak}");
        s.event(NoteEvent::Off { note: 60, velocity: 0.0 });
        render(&mut s, 4800);
        assert_eq!(s.active_voices(), 0);
        let (l, _) = render(&mut s, 480);
        assert!(l.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn polyphony_is_capped_by_stealing() {
        let mut s = synth();
        let poly = s.patch().param(POLY) as usize;
        for n in 0..(poly as u8 + 4) {
            s.event(NoteEvent::On { note: 40 + n, velocity: 1.0 });
            render(&mut s, 64);
        }
        assert_eq!(s.active_voices(), poly);
    }

    #[test]
    fn live_parameter_edits_take_effect() {
        let mut s = synth();
        let cutoff = ParamRef::new("Oscillator", 0, "kParamVolume");
        s.event(NoteEvent::On { note: 60, velocity: 1.0 });
        let (loud, _) = render(&mut s, 4800);
        s.set_param(cutoff, 0.0);
        let (quiet, _) = render(&mut s, 4800);
        let peak = |x: &[f32]| x[2400..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak(&loud) > 0.05 && peak(&quiet) < 1e-4, "{} {}", peak(&loud), peak(&quiet));
    }

    #[test]
    fn swapping_a_prepared_patch_returns_the_old_one_and_cuts_voices() {
        let mut s = synth();
        let old = s.patch().clone();
        s.event(NoteEvent::On { note: 60, velocity: 1.0 });
        render(&mut s, 480);
        let mut next = (*old).clone();
        next.name = "next".into();
        let retired = s.set_patch(PreparedPatch::new(Arc::new(next), 44100.0));
        assert!(Arc::ptr_eq(retired.patch(), &old));
        assert_eq!(s.patch().name, "next");
        assert_eq!(s.active_voices(), 0);
        let (l, _) = render(&mut s, 480);
        assert!(l.iter().all(|x| x.is_finite()), "racks rebuilt for the synth's own rate");
    }

    #[test]
    fn output_is_finite_under_extreme_controls() {
        let mut s = synth();
        s.set_pitch_bend(1.0);
        s.set_mod_wheel(1.0);
        s.event(NoteEvent::On { note: 127, velocity: 1.0 });
        s.event(NoteEvent::On { note: 0, velocity: 1.0 });
        let (l, r) = render(&mut s, 48000);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    }
}
