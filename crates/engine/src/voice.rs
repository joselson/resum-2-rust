//! One playing note: oscillators → filters → amp envelope, with modulation
//! resolved every `BLOCK` samples. All settings are read from `Patch::values`
//! through the parameter layer, with this voice's modulation applied.

use crate::filters::{self, cutoff_hz};
use crate::modulation::{Offsets, Sources};
use crate::params::{ParamRef as P, Values};
use crate::patch::Patch;
use crate::sampleosc::{SampleSource, SampleVoice};
use lumen_dsp::env::{EnvParams, Envelope};
use lumen_dsp::filter::{Filter, FilterKind};
use lumen_dsp::lfo::{Chaos, ChaosKind, Direction, Lfo, LfoMode, LfoTiming};
use lumen_dsp::osc::{DetuneMode, Unison, UnisonStack, WtOscillator};
use lumen_dsp::sub::{SubOscillator, SubShape};
use lumen_dsp::warp::{WarpMode, WarpSlot, WarpSource};
use lumen_dsp::{note_to_hz, Rng};

/// Control-rate period in samples.
pub const BLOCK: usize = 32;

// Parameters read by the voice (instance 0; use `.on(i)` for others).
const OSC_ENABLE: P = P::new("Oscillator", 0, "kParamEnable");
const OSC_VOLUME: P = P::new("Oscillator", 0, "kParamVolume");
const OSC_PAN: P = P::new("Oscillator", 0, "kParamPan");
const OSC_OCTAVE: P = P::new("Oscillator", 0, "kParamOctave");
const OSC_SEMI: P = P::new("Oscillator", 0, "kParamPitch");
const OSC_FINE: P = P::new("Oscillator", 0, "kParamFine");
const OSC_COARSE: P = P::new("Oscillator", 0, "kParamCoarsePit");
const OSC_UNISON: P = P::new("Oscillator", 0, "kParamUnison");
const OSC_DETUNE: P = P::new("Oscillator", 0, "kParamDetune");
const OSC_BLEND: P = P::new("Oscillator", 0, "kParamDetuneWid");
const OSC_WIDTH: P = P::new("Oscillator", 0, "kParamUnisonStereo");
const OSC_DETUNE_MODE: P = P::new("Oscillator", 0, "kParamDetuneMode");
const OSC_UNISON_RANGE: P = P::new("Oscillator", 0, "kParamUnisonRange");
const OSC_UNISON_STACK: P = P::new("Oscillator", 0, "kParamUnisonStack");
const OSC_TYPE: P = P::new("Oscillator", 0, "kParamType");
const WT_POS: P = P::new("WTOsc", 0, "kParamTablePos");
const WT_UNISON_POS: P = P::new("WTOsc", 0, "kParamUnisonWTPos");
const WT_PHASE: P = P::new("WTOsc", 0, "kParamInitialPhase");
const WT_RANDOM_PHASE: P = P::new("WTOsc", 0, "kParamRandomPhase");
const WT_WARP: [(P, P, P); 2] = [
    (P::new("WTOsc", 0, "kParamWarpMenu"), P::new("WTOsc", 0, "kParamWarp"), P::new("WTOsc", 0, "kParamWarpVar")),
    (P::new("WTOsc", 0, "kParamWarpMenu2"), P::new("WTOsc", 0, "kParamWarp2"), P::new("WTOsc", 0, "kParamWarpVar2")),
];
const OSC_UNISON_WARP: [P; 2] = [P::new("Oscillator", 0, "kParamUnisonWarp"), P::new("Oscillator", 0, "kParamUnisonWarp2")];
const NOISE_PITCH: P = P::new("NoiseOsc", 0, "kParamColor");
const SUB_SHAPE: P = P::new("SubOsc", 0, "kParamShape");
const SUB_PHASE: P = P::new("SubOsc", 0, "kParamInitialPhase");
const FILTER_ENABLE: P = P::new("VoiceFilter", 0, "kParamEnable");
const FILTER_TYPE: P = P::new("VoiceFilter", 0, "kParamType");
const FILTER_FREQ: P = P::new("VoiceFilter", 0, "kParamFreq");
const FILTER_RESO: P = P::new("VoiceFilter", 0, "kParamReso");
const FILTER_DRIVE: P = P::new("VoiceFilter", 0, "kParamDrive");
const FILTER_WET: P = P::new("VoiceFilter", 0, "kParamWet");
const FILTER_LEVEL: P = P::new("VoiceFilter", 0, "kParamLevelOut");
const FILTER_KEYTRACK: P = P::new("VoiceFilter", 0, "kParamKeyTrack");
const OVERSAMPLING: P = P::new("Global", 0, "kParamOversampling");
const FILTER_STEREO: P = P::new("VoiceFilter", 0, "kParamStereo");
const FILTER_VAR: P = P::new("VoiceFilter", 0, "kParamVar");
const ENV_ATTACK: P = P::new("Env", 0, "kParamAttack");
const ENV_HOLD: P = P::new("Env", 0, "kParamHold");
const ENV_DECAY: P = P::new("Env", 0, "kParamDecay");
const ENV_SUSTAIN: P = P::new("Env", 0, "kParamSustain");
const ENV_RELEASE: P = P::new("Env", 0, "kParamRelease");
const ENV_CURVES: [P; 3] = [P::new("Env", 0, "kParamCurve1"), P::new("Env", 0, "kParamCurve2"), P::new("Env", 0, "kParamCurve3")];
const ENV_START: P = P::new("Env", 0, "kParamStart");
const ENV_END: P = P::new("Env", 0, "kParamEnd");
const ENV_SYNC: P = P::new("Env", 0, "kParamBeatSync");
const VOICE_AMP: P = P::new("Global", 0, "kParamVoiceAmp");
const LFO_RATE: P = P::new("LFO", 0, "kParamRate");
const LFO_SMOOTH: P = P::new("LFO", 0, "kParamSmooth");
const LFO_RISE: P = P::new("LFO", 0, "kParamRise");
const LFO_DELAY: P = P::new("LFO", 0, "kParamDelay");
const LFO_PHASE: P = P::new("LFO", 0, "kParamPhase");
const LFO_MODE: P = P::new("LFO", 0, "kParamMode");
const LFO_TYPE: P = P::new("LFO", 0, "kParamType");
const LFO_SYNC: P = P::new("LFO", 0, "kParamBeatSync");
const LFO_DOTTED: P = P::new("LFO", 0, "kParamDotted");
const LFO_DIRECTION: P = P::new("LFO", 0, "kParamDirection");
const LFO_TRIPLETS: P = P::new("LFO", 0, "kParamTriplets");
const LFO_RATE10X: P = P::new("LFO", 0, "kParamRate10x");
const MACRO_VALUE: P = P::new("Macro", 0, "kParamValue");
const GLOBAL_A4: P = P::new("Global", 0, "kParamGlobalTuning");
const MASTER_TUNING: P = P::new("Global", 0, "kParamMasterTuning");
const TRANSPOSE: P = P::new("Global", 0, "kParamTranspose");
const BEND_UP: P = P::new("Global", 0, "kParamBendRangeUp");
const BEND_DOWN: P = P::new("Global", 0, "kParamBendRangeDn");
const PORTAMENTO: P = P::new("Global", 0, "kParamPortamentoTime");
const ROUTE_DEST: P = P::new("RoutingSlot", 0, "kParamRoutingDest");
const ROUTE_BALANCE: P = P::new("RoutingSlot", 0, "kParamFilterBalance");
const ROUTE_BUS1: P = P::new("RoutingSlot", 0, "kParamFXBus1Level");
const ROUTE_BUS2: P = P::new("RoutingSlot", 0, "kParamFXBus2Level");

/// Parameters this voice renders, by (module, parameter). Coverage reports any
/// other parameter a preset changes from its default.
pub(crate) const RENDERED: &[(&str, &str)] = &[
    ("Oscillator", "kParamEnable"), ("Oscillator", "kParamVolume"), ("Oscillator", "kParamPan"), ("Oscillator", "kParamOctave"),
    ("Oscillator", "kParamPitch"), ("Oscillator", "kParamFine"), ("Oscillator", "kParamCoarsePit"), ("Oscillator", "kParamUnison"),
    ("Oscillator", "kParamDetune"), ("Oscillator", "kParamDetuneWid"), ("Oscillator", "kParamUnisonStereo"), ("Oscillator", "kParamDetuneMode"),
    ("Oscillator", "kParamUnisonRange"), ("Oscillator", "kParamUnisonStack"), ("Oscillator", "kParamType"),
    ("WTOsc", "kParamTablePos"), ("WTOsc", "kParamUnisonWTPos"), ("WTOsc", "kParamInitialPhase"), ("WTOsc", "kParamRandomPhase"),
    ("WTOsc", "kParamWarpMenu"), ("WTOsc", "kParamWarp"), ("WTOsc", "kParamWarpVar"), ("WTOsc", "kParamWarpMenu2"),
    ("WTOsc", "kParamWarp2"), ("WTOsc", "kParamWarpVar2"), ("Oscillator", "kParamUnisonWarp"), ("Oscillator", "kParamUnisonWarp2"),
    ("NoiseOsc", "kParamColor"), ("SubOsc", "kParamShape"), ("SubOsc", "kParamInitialPhase"),
    ("VoiceFilter", "kParamEnable"), ("VoiceFilter", "kParamType"), ("VoiceFilter", "kParamFreq"), ("VoiceFilter", "kParamReso"),
    ("VoiceFilter", "kParamDrive"), ("VoiceFilter", "kParamWet"), ("VoiceFilter", "kParamLevelOut"), ("VoiceFilter", "kParamKeyTrack"),
    ("VoiceFilter", "kParamStereo"), ("VoiceFilter", "kParamVar"), ("Global", "kParamOversampling"),
    ("Env", "kParamAttack"), ("Env", "kParamHold"), ("Env", "kParamDecay"), ("Env", "kParamSustain"), ("Env", "kParamRelease"),
    ("Env", "kParamCurve1"), ("Env", "kParamCurve2"), ("Env", "kParamCurve3"),
    ("LFO", "kParamRate"), ("LFO", "kParamSmooth"), ("LFO", "kParamRise"), ("LFO", "kParamDelay"), ("LFO", "kParamPhase"),
    ("LFO", "kParamMode"), ("LFO", "kParamType"), ("LFO", "kParamBeatSync"), ("LFO", "kParamDefaultMode"),
    ("LFO", "kParamDotted"), ("LFO", "kParamTriplets"), ("LFO", "kParamRate10x"), ("LFO", "kParamDirection"),
    ("Macro", "kParamValue"),
    ("ModSlot", "kParamAmount"), ("ModSlot", "kParamBipolar"), ("ModSlot", "kParamBypass"), ("ModSlot", "kParamAuxInverted"),
    ("ModSlot", "kParamCurveIn"), ("ModSlot", "kParamMainCurveData"), ("ModSlot", "kParamAuxCurveData"), ("ModSlot", "kParamOut"), ("ModSlot", "kParamAuxCurve"),
    ("Global", "kParamMasterVolume"), ("Global", "kParamVoiceAmp"),
    ("Env", "kParamStart"), ("Env", "kParamEnd"), ("Env", "kParamBeatSync"), ("Global", "kParamGlobalTuning"), ("Global", "kParamMasterTuning"), ("Global", "kParamTranspose"),
    ("Global", "kParamBendRangeUp"),
    ("Global", "kParamBendRangeDn"), ("Global", "kParamPolyCount"), ("Global", "kParamMonoToggle"), ("Global", "kParamLegato"),
    ("Global", "kParamPortamentoTime"), ("Global", "kParamPortaAlways"), ("Global", "kParamModWheel"),
    ("RoutingSlot", "kParamRoutingDest"), ("RoutingSlot", "kParamFilterBalance"), ("RoutingSlot", "kParamFXBus1Level"),
    ("RoutingSlot", "kParamFXBus2Level"), ("Global", "kParamDirectVol"), ("Global", "kParamFXBus1Vol"), ("Global", "kParamFXBus2Vol"),
    ("Global", "kParamFXBus1Dest"), ("Global", "kParamFXBus2Dest"),
];

/// Host/controller state shared by all voices.
#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub sample_rate: f32,
    pub bpm: f32,
    pub mod_wheel: f32,
    pub pitch_bend: f32,
    pub aftertouch: f32,
    /// Macro values in [0, 1]; initialised from the patch, then host-controllable.
    pub macros: [f32; 8],
}

/// How a note starts: `legato` retargets a sounding voice without retriggering,
/// `glide_from` is the pitch portamento starts at, `alternate` flips per note-on synth-wide.
#[derive(Clone, Copy, Debug)]
pub struct NoteStart {
    pub note: u8,
    pub velocity: f32,
    pub age: u64,
    pub legato: bool,
    pub glide_from: Option<f32>,
    pub alternate: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dest {
    Filter,
    Master,
    Direct,
    None,
}

fn dest(values: &Values, slot: usize) -> Dest {
    match values.choice(ROUTE_DEST.on(slot)) {
        "kRoutingDestFilter" => Dest::Filter,
        "kRoutingDestDirect" => Dest::Direct,
        "kRoutingDestNone" => Dest::None,
        _ => Dest::Master,
    }
}

fn detune_mode(name: &str) -> DetuneMode {
    match name {
        "kDetuneExp" => DetuneMode::Exponential,
        "kDetuneRandom" => DetuneMode::Random,
        "kDetuneSuper" => DetuneMode::Super,
        "kDetuneInv" => DetuneMode::Inverse,
        _ => DetuneMode::Linear,
    }
}

fn chaos_kind(name: &str) -> Option<ChaosKind> {
    match name {
        "Lorenz" => Some(ChaosKind::Lorenz),
        "Rossler" => Some(ChaosKind::Rossler),
        _ => None,
    }
}

fn unison_stack(name: &str) -> UnisonStack {
    match name {
        "kOctave1" => UnisonStack::Octaves(1),
        "kOctave2" => UnisonStack::Octaves(2),
        "kOctave3" => UnisonStack::Octaves(3),
        "kOctaveFifth1" => UnisonStack::OctaveFifth1,
        "kOctaveFifth2" => UnisonStack::OctaveFifth2,
        "kOctaveFifth3" => UnisonStack::OctaveFifth3,
        "kCenter12" => UnisonStack::Centre(12),
        "kCenter24" => UnisonStack::Centre(24),
        _ => UnisonStack::None,
    }
}

fn sub_shape(name: &str) -> SubShape {
    match name {
        "kRoundRect" => SubShape::RoundRect,
        "kTriangle" => SubShape::Triangle,
        "kSaw" => SubShape::Saw,
        "kSquare" => SubShape::Square,
        "kPulse" => SubShape::Pulse,
        _ => SubShape::Sine,
    }
}

/// Beat-synced envelope time. The time control (plain = 32·x⁵) steps through 47 equal
/// cells, round(46x): "fast", then from 1/1024 to 32 bars in threes — a division, the
/// next one's triplet, then the division dotted (display sweep; renders at 120 BPM
/// confirm 1/16 t, 1/4 t and 1/2 t).
fn synced_env_seconds(stored: f32, bpm: f32) -> f32 {
    let x = (stored.max(0.0) / 32.0).powf(0.2);
    let j = (x * 46.0).round().clamp(0.0, 46.0) as i32;
    if j == 0 {
        return 0.0;
    }
    let (group, step) = ((j - 1) / 3, (j - 1) % 3);
    let bars = (group as f32).exp2() / 1024.0 * [1.0, 4.0 / 3.0, 1.5][step as usize];
    bars * 4.0 * 60.0 / bpm.max(1.0)
}

/// Curve amount for a stored envelope curve (0–100, 50 linear): below 50 a segment
/// starts slowly. Each segment is the shared curve law in amplitude (envelope probes).
fn env_curve(stored: f32) -> f32 {
    ((stored - 50.0) / 50.0).clamp(-1.0, 1.0)
}

/// Tempo-synced LFO rate. The rate control (plain = 100·x⁴) steps through 15 equal
/// cells, ⌊14x + 0.16⌋: 32 bars, 16 bars … 1/256, then "fast" (taken as 1/512).
/// Dotted gives the last two thirds of each cell to the next division dotted;
/// triplets give the last third to the cell's own division as a triplet (rate
/// display sweeps; renders confirm the snapping at 120 BPM).
fn synced_lfo_hz(rate: f32, bpm: f32, dotted: bool, triplets: bool) -> f32 {
    let x = (rate.max(0.0) / 100.0).powf(0.25);
    let pos = (x * 14.0 + 0.16).clamp(0.0, 14.999);
    let (cell, frac) = (pos.floor(), pos.fract());
    // Division length in beats: 32 bars is 128 beats, halving per cell.
    let beats = |k: f32| 128.0 * (-k).exp2();
    let length = if dotted && frac >= 1.0 / 3.0 && (!triplets || frac < 2.0 / 3.0) {
        beats(cell + 1.0) * 1.5
    } else if triplets && frac >= 2.0 / 3.0 {
        beats(cell) * 2.0 / 3.0
    } else {
        beats(cell)
    };
    bpm / 60.0 / length
}

#[derive(Clone)]
pub struct Voice {
    pub note: u8,
    pub age: u64,
    pub held: bool,
    velocity: f32,
    release_velocity: f32,
    pitch: f32,
    target_pitch: f32,
    oscs: [WtOscillator; 3],
    sub: SubOscillator,
    rng: Rng,
    noise_lp: [f32; 2],
    filters: [Filter; 2],
    envs: [Envelope; 4],
    /// Env 1 as applied to amplitude: the same segments, run toward sustain².
    amp_env: Envelope,
    lfos: [Lfo; 10],
    lfo_hold: [f32; 10],
    chaos: [Chaos; 10],
    lfo_smoothed: [f32; 10],
    lfo_last_phase: [f32; 10],
    random: [f32; 2],
    alternate: [f32; 2],
    amp: f32,
    offsets: Offsets,
    /// Previous-sample outputs of A, B, C, Noise, Sub, Filter 1, Filter 2 (warp modulators).
    prev: [f32; 7],
    sample_voices: [SampleVoice; 3],
}

impl Voice {
    pub fn new(seed: u32) -> Voice {
        Voice {
            note: 0,
            age: 0,
            held: false,
            velocity: 0.0,
            release_velocity: 0.0,
            pitch: 60.0,
            target_pitch: 60.0,
            oscs: Default::default(),
            sub: SubOscillator::default(),
            rng: Rng::new(seed),
            noise_lp: [0.0; 2],
            filters: [Filter::new(FilterKind::Ladder { poles: 2 }), Filter::new(FilterKind::Ladder { poles: 2 })],
            envs: Default::default(),
            amp_env: Envelope::default(),
            lfos: Default::default(),
            lfo_hold: [0.0; 10],
            chaos: [Chaos::default(); 10],
            lfo_smoothed: [0.0; 10],
            lfo_last_phase: [0.0; 10],
            random: [0.0; 2],
            alternate: [0.0; 2],
            amp: 0.0,
            offsets: Offsets::default(),
            prev: [0.0; 7],
            sample_voices: Default::default(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.amp_env.is_active()
    }

    fn lfo_timing(&self, v: &Values, i: usize, bpm: f32) -> LfoTiming {
        let m = |p: P| self.offsets.apply(v, p.on(i));
        let rate = m(LFO_RATE);
        LfoTiming {
            // ×10 applies to the free rate only (synced probes are unchanged by it).
            hz: if v.flag(LFO_SYNC.on(i)) {
                synced_lfo_hz(rate, bpm, v.flag(LFO_DOTTED.on(i)), v.flag(LFO_TRIPLETS.on(i)))
            } else if v.flag(LFO_RATE10X.on(i)) {
                rate * 10.0
            } else {
                rate
            },
            mode: match v.choice(LFO_MODE.on(i)) {
                "Free" => LfoMode::Free,
                "Envelope" => LfoMode::Envelope,
                _ => LfoMode::Trigger,
            },
            phase: m(LFO_PHASE) / 360.0,
            delay: m(LFO_DELAY),
            rise: m(LFO_RISE),
            direction: match v.get(LFO_DIRECTION.on(i)).round() as i32 {
                1 => Direction::Reverse,
                2 => Direction::PingPong,
                _ => Direction::Forward,
            },
        }
    }

    /// Starts (or, with `legato`, retargets) a note.
    pub fn start(&mut self, patch: &Patch, start: NoteStart) {
        self.note = start.note;
        self.age = start.age;
        self.held = true;
        self.velocity = start.velocity;
        self.target_pitch = start.note as f32;
        self.pitch = start.glide_from.unwrap_or(start.note as f32);
        if start.legato && self.is_active() {
            return;
        }
        let v = &patch.values;
        self.offsets.clear();
        self.random = [self.rng.unit(), self.rng.unit()];
        self.alternate = if start.alternate { [1.0, 0.0] } else { [0.0, 1.0] };
        // One random draw per note, shared by A–C: identically set oscillators start in
        // phase and sum coherently, as measured against Serum.
        let phase_seed = self.rng.next_u32();
        for (i, osc) in self.oscs.iter_mut().enumerate() {
            osc.trigger(v.get(WT_PHASE.on(i)) / 360.0, v.get(WT_RANDOM_PHASE.on(i)) / 100.0, &mut Rng::new(phase_seed));
        }
        // At phase 0 Serum's sub starts half a cycle into its shape (every shape, measured).
        self.sub.trigger(v.get(SUB_PHASE) / 360.0 + 0.5);
        for (i, sv) in self.sample_voices.iter_mut().enumerate() {
            let transposed = start.note as f32 + v.get(OSC_OCTAVE.on(i)).round() * 12.0 + v.get(OSC_SEMI.on(i)).round() + v.get(OSC_COARSE.on(i));
            sv.trigger(&patch.samples[i], v, i, transposed.round().clamp(0.0, 127.0) as u8, start.velocity, &mut self.rng);
        }
        for f in &mut self.filters {
            f.reset();
        }
        for e in &mut self.envs {
            e.trigger(false);
        }
        self.amp_env.trigger(false);
        for i in 0..10 {
            let timing = self.lfo_timing(v, i, 120.0);
            self.lfos[i].trigger(&timing);
            self.lfo_hold[i] = self.rng.unit();
            if let Some(kind) = chaos_kind(v.choice(LFO_TYPE.on(i))) {
                self.chaos[i].trigger(kind);
            }
            self.lfo_last_phase[i] = self.lfos[i].phase();
            self.lfo_smoothed[i] = patch.lfo_curves[i].eval(self.lfos[i].position());
        }
        self.amp = 0.0;
        self.prev = [0.0; 7];
    }

    pub fn release(&mut self, velocity: f32) {
        self.held = false;
        for sv in &mut self.sample_voices {
            sv.release();
        }
        self.release_velocity = velocity;
        for e in &mut self.envs {
            e.release();
        }
        self.amp_env.release();
    }

    /// Silences immediately (voice stealing, patch change).
    pub fn kill(&mut self) {
        self.held = false;
        self.envs = Default::default();
        self.amp_env = Envelope::default();
        self.amp = 0.0;
    }

    /// Adds `n` (≤ `BLOCK`) samples of this voice into the output buses.
    // The sample loop indexes several parallel per-sample buffers by position.
    #[allow(clippy::needless_range_loop)]
    pub fn render(&mut self, patch: &Patch, controls: &Controls, out: &mut [[f32; BLOCK]; BUS_CHANNELS], n: usize) {
        let sr = controls.sample_rate;
        let dt = n as f32 / sr;
        let v = &patch.values;

        let porta = v.get(PORTAMENTO);
        self.pitch = if porta > 1e-4 {
            self.target_pitch + (self.pitch - self.target_pitch) * (-dt / (porta * 0.25)).exp()
        } else {
            self.target_pitch
        };

        // Sources; envelope and LFO settings use the previous block's modulation.
        let mut src = Sources {
            mod_wheel: controls.mod_wheel,
            velocity: self.velocity,
            note: self.note as f32 / 127.0,
            aftertouch: controls.aftertouch,
            random: self.random,
            alternate: self.alternate,
            pitch_bend: controls.pitch_bend,
            release_velocity: self.release_velocity,
            ..Sources::default()
        };
        let mut amp_level = 0.0;
        for i in 0..4 {
            let m = |p: P| self.offsets.apply(v, p.on(i));
            let synced = v.flag(ENV_SYNC.on(i));
            let time = |p: P| if synced { synced_env_seconds(m(p), controls.bpm) } else { m(p) };
            let params = EnvParams {
                attack: time(ENV_ATTACK),
                hold: time(ENV_HOLD),
                decay: time(ENV_DECAY),
                sustain: m(ENV_SUSTAIN),
                release: time(ENV_RELEASE),
                curves: ENV_CURVES.map(|c| env_curve(m(c))),
                start: m(ENV_START),
                end: m(ENV_END),
            };
            src.envs[i] = self.envs[i].advance(&params, sr, n);
            if i == 0 {
                // Serum's amp envelope runs in amplitude toward sustain² (0.5 sustains at
                // −12 dB) with the same curves; it is not the square of the curved level.
                amp_level = self.amp_env.advance(&EnvParams { sustain: params.sustain * params.sustain, ..params }, sr, n);
            }
        }
        for i in 0..10 {
            let timing = self.lfo_timing(v, i, controls.bpm);
            let shaped = self.lfos[i].advance(&patch.lfo_curves[i], &timing, dt);
            let phase = self.lfos[i].phase();
            if phase < self.lfo_last_phase[i] {
                self.lfo_hold[i] = self.rng.unit();
            }
            self.lfo_last_phase[i] = phase;
            let kind = v.choice(LFO_TYPE.on(i));
            let raw = match chaos_kind(kind) {
                Some(chaos) => {
                    let [x, y] = self.chaos[i].advance(chaos, timing.hz, dt);
                    src.lfo_y[i] = y;
                    x
                }
                None if kind == "RandomSH" => self.lfo_hold[i],
                None if kind == "Path" => {
                    let (x, y) = patch.lfo_paths[i].as_ref().map_or((0.5, 0.5), |p| p.eval(self.lfos[i].position()));
                    src.lfo_y[i] = y;
                    x
                }
                None => shaped,
            };
            let smooth = self.offsets.apply(v, LFO_SMOOTH.on(i)) / 100.0;
            let coef = if smooth > 0.0 { 1.0 - (-dt / (smooth * 0.25)).exp() } else { 1.0 };
            self.lfo_smoothed[i] += (raw - self.lfo_smoothed[i]) * coef;
            src.lfos[i] = self.lfo_smoothed[i];
        }
        src.macros = controls.macros;
        self.offsets.evaluate(&patch.routes, &src, v);
        // Macros can be modulated too; resolve them, then re-evaluate once.
        let mut moved = false;
        for i in 0..8 {
            let delta = self.offsets.get(MACRO_VALUE.on(i));
            if delta != 0.0 {
                src.macros[i] = (controls.macros[i] + delta).clamp(0.0, 1.0);
                moved = true;
            }
        }
        if moved {
            self.offsets.evaluate(&patch.routes, &src, v);
        }
        let offsets = &self.offsets;
        let m = |p: P| offsets.apply(v, p);

        let bend = if controls.pitch_bend >= 0.0 { controls.pitch_bend * v.get(BEND_UP) } else { -controls.pitch_bend * v.get(BEND_DOWN) };
        // Master tuning spans ±64 semitones around 0.5 (0.1 of its range measured 12.80 st).
        let base_note = self.pitch + v.get(TRANSPOSE) + bend + (m(MASTER_TUNING) - 0.5) * 128.0;
        let a4 = v.get(GLOBAL_A4);
        let osc_note = |i: usize| base_note + m(OSC_OCTAVE.on(i)).round() * 12.0 + m(OSC_SEMI.on(i)).round() + m(OSC_COARSE.on(i)) + m(OSC_FINE.on(i)) / 100.0;

        let mut gains = [[0f32; 2]; 5];
        let mut dests = [Dest::None; 5];
        let enabled: [bool; 5] = std::array::from_fn(|i| v.flag(OSC_ENABLE.on(i)));
        for i in 0..5 {
            if v.flag(OSC_ENABLE.on(i)) {
                dests[i] = dest(v, i);
                gains[i] = pan_gains(m(OSC_PAN.on(i)) / 50.0, m(OSC_VOLUME.on(i)));
            }
        }
        let mut wavetable = [false; 3];
        let mut sampled = [false; 3];
        let mut sample_pitch = [0f32; 3];
        let mut warp_sources: [[Option<WarpSource>; 2]; 3] = [[None; 2]; 3];
        // Enabled oscillators always run, so ones routed to None can still modulate warps.
        for (i, osc) in self.oscs.iter_mut().enumerate() {
            if !v.flag(OSC_ENABLE.on(i)) {
                continue;
            }
            if !matches!(patch.samples[i], SampleSource::None) {
                sampled[i] = true;
                sample_pitch[i] = osc_note(i);
                continue;
            }
            if v.choice(OSC_TYPE.on(i)) != "kOsc_WT" {
                dests[i] = Dest::None;
                continue;
            }
            wavetable[i] = true;
            let table = &patch.tables[i];
            let frames = table.frames().saturating_sub(1) as f32;
            let unison = Unison {
                voices: m(OSC_UNISON.on(i)).round().clamp(1.0, 16.0) as usize,
                // Outer voices sit at ±range·detune^1.25 semitones (measured at detune
                // 0.05–1 and range 2, 12 and 48).
                detune_semis: v.get(OSC_UNISON_RANGE.on(i)) * m(OSC_DETUNE.on(i)).max(0.0).powf(1.25),
                mode: detune_mode(v.choice(OSC_DETUNE_MODE.on(i))),
                width: m(OSC_WIDTH.on(i)) / 100.0,
                position_spread: m(WT_UNISON_POS.on(i)) / 100.0 * frames * 0.5,
                blend: m(OSC_BLEND.on(i)) / 100.0,
                warp_spread: OSC_UNISON_WARP.map(|p| m(p.on(i)) / 100.0),
                stack: unison_stack(v.choice(OSC_UNISON_STACK.on(i))),
            };
            let warps = WT_WARP.map(|(menu, amount, var)| WarpSlot {
                mode: WarpMode::from_name(v.choice(menu.on(i))).unwrap_or(WarpMode::Off),
                amount: m(amount.on(i)),
                var: m(var.on(i)),
            });
            warp_sources[i] = warps.map(|w| w.mode.source());
            osc.set_warps(warps);
            let pos = (m(WT_POS.on(i)) - 1.0) / 255.0 * frames;
            osc.prepare(note_to_hz(osc_note(i), a4), sr, pos, table.frames(), &unison);
        }
        self.sub.set_frequency(note_to_hz(osc_note(4), a4), sr);
        let shape = sub_shape(v.choice(SUB_SHAPE));
        // Noise pitch as brightness: open at the default (0.5) and above, darker below.
        let noise_cut = 200.0 * 100f32.powf((m(NOISE_PITCH) / 0.5).min(1.0));
        let noise_coef = 1.0 - (-std::f32::consts::TAU * noise_cut / sr).exp();

        // Serum runs its filters at 4× by default and at 2× on the lowest setting (measured).
        let oversampling = if m(OVERSAMPLING) < 0.5 { 2 } else { 4 };
        let mut filter_on = [false; 2];
        let mut filter_mix = [(0f32, 1f32); 2];
        for i in 0..2 {
            if !v.flag(FILTER_ENABLE.on(i)) {
                continue;
            }
            let Some(kind) = filters::model(v.choice(FILTER_TYPE.on(i))).0 else { continue };
            filter_on[i] = true;
            if self.filters[i].kind() != kind {
                self.filters[i].set_kind(kind);
            }
            self.filters[i].set_var(m(FILTER_VAR.on(i)) / 100.0);
            self.filters[i].set_oversampling(oversampling);
            // Stereo 50 is neutral; 0 moves the left cutoff 2.65 octaves up and the right down.
            self.filters[i].set_spread((50.0 - m(FILTER_STEREO.on(i))) / 50.0 * 2.65);
            let mut hz = cutoff_hz(m(FILTER_FREQ.on(i)));
            if v.flag(FILTER_KEYTRACK.on(i)) {
                hz *= ((self.note as f32 - 60.0) / 12.0).exp2();
            }
            self.filters[i].set(hz, m(FILTER_RESO.on(i)) / 100.0, m(FILTER_DRIVE.on(i)) / 100.0, sr);
            // Level is squared: Serum shows 0.1 as −28 dB and 1.0 as +12 dB around 0.5.
            let level = (m(FILTER_LEVEL.on(i)) / FILTER_LEVEL.spec().default_for(i).max(1e-6)).powi(2);
            filter_mix[i] = (m(FILTER_WET.on(i)) / 100.0, level);
        }
        // −100 = all to filter 1, +100 = all to filter 2.
        let balance: [f32; 5] = std::array::from_fn(|i| ((m(ROUTE_BALANCE.on(i)) + 100.0) / 200.0).clamp(0.0, 1.0));
        // Routing slots 5 and 6 are the filter outputs; filter 1 → "Filter" feeds filter 2.
        let filter_dest = [dest(v, 5), dest(v, 6)];
        let sends: [[f32; 2]; 7] = std::array::from_fn(|i| [m(ROUTE_BUS1.on(i)) / 100.0, m(ROUTE_BUS2.on(i)) / 100.0]);

        // Voice amp is linear gain with 0.5 as unity (0.25 measured −6 dB, 1.0 +6 dB).
        let amp_target = amp_level * m(VOICE_AMP) * 2.0;
        let amp_start = self.amp;

        for s in 0..n {
            let mut samples = [(0f32, 0f32); 5];
            let prev = self.prev;
            for (i, osc) in self.oscs.iter_mut().enumerate() {
                if wavetable[i] {
                    // "OSC"/"OSC2" are the other two oscillators in A, B, C order.
                    let others = [(i + 1) % 3, (i + 2) % 3];
                    let (first, second) = (others[0].min(others[1]), others[0].max(others[1]));
                    let warp_mod = warp_sources[i].map(|src| match src {
                        Some(WarpSource::Osc) => prev[first],
                        Some(WarpSource::Osc2) => prev[second],
                        Some(WarpSource::Noise) => prev[3],
                        Some(WarpSource::Sub) => prev[4],
                        Some(WarpSource::Filter1) => prev[5],
                        Some(WarpSource::Filter2) => prev[6],
                        None => 0.0,
                    });
                    samples[i] = osc.next(&patch.tables[i], warp_mod);
                }
            }
            for i in 0..3 {
                if sampled[i] {
                    samples[i] = self.sample_voices[i].next(&patch.samples[i], sample_pitch[i], self.held, sr);
                }
            }
            if enabled[3] {
                // Mono, like Serum's noise oscillator.
                let w = self.rng.bipolar() * NOISE_GAIN;
                self.noise_lp[0] += (w - self.noise_lp[0]) * noise_coef;
                samples[3] = (self.noise_lp[0], self.noise_lp[0]);
            }
            if enabled[4] {
                let x = self.sub.next(shape) * SUB_GAIN;
                samples[4] = (x, x);
            }
            for (p, s) in self.prev.iter_mut().zip(&samples) {
                *p = 0.5 * (s.0 + s.1);
            }

            let amp = amp_start + (amp_target - amp_start) * ((s + 1) as f32 / n as f32);
            let mut filter_in = [[0f32; 2]; 2];
            let mut add = |bus: Bus, l: f32, r: f32| {
                let k = bus as usize * 2;
                out[k][s] += l * amp;
                out[k + 1][s] += r * amp;
            };
            for i in 0..5 {
                if dests[i] == Dest::None {
                    continue;
                }
                let (l, r) = (samples[i].0 * gains[i][0], samples[i].1 * gains[i][1]);
                for (b, bus) in [Bus::Fx1, Bus::Fx2].into_iter().enumerate() {
                    if sends[i][b] > 0.0 {
                        add(bus, l * sends[i][b], r * sends[i][b]);
                    }
                }
                let (to1, to2) = match (dests[i], filter_on) {
                    (Dest::Filter, [true, true]) => (1.0 - balance[i], balance[i]),
                    (Dest::Filter, [true, false]) => (1.0, 0.0),
                    (Dest::Filter, [false, true]) => (0.0, 1.0),
                    _ => (0.0, 0.0),
                };
                if to1 + to2 == 0.0 {
                    add(if dests[i] == Dest::Direct { Bus::Direct } else { Bus::Main }, l, r);
                } else {
                    filter_in[0][0] += l * to1;
                    filter_in[0][1] += r * to1;
                    filter_in[1][0] += l * to2;
                    filter_in[1][1] += r * to2;
                }
            }
            for i in 0..2 {
                if !filter_on[i] {
                    continue;
                }
                let [dry_l, dry_r] = filter_in[i];
                let (fl, fr) = self.filters[i].process(dry_l, dry_r);
                let (wet, level) = filter_mix[i];
                let (yl, yr) = ((dry_l + (fl - dry_l) * wet) * level, (dry_r + (fr - dry_r) * wet) * level);
                self.prev[5 + i] = 0.5 * (fl + fr);
                for (b, bus) in [Bus::Fx1, Bus::Fx2].into_iter().enumerate() {
                    if sends[5 + i][b] > 0.0 {
                        add(bus, yl * sends[5 + i][b], yr * sends[5 + i][b]);
                    }
                }
                match filter_dest[i] {
                    Dest::None => {}
                    Dest::Direct => add(Bus::Direct, yl, yr),
                    Dest::Filter if i == 0 && filter_on[1] => {
                        filter_in[1][0] += yl;
                        filter_in[1][1] += yr;
                    }
                    _ => add(Bus::Main, yl, yr),
                }
            }
        }
        self.amp = amp_target;
    }

    /// Modulation offsets this voice computed for FX modules in the last block.
    pub fn fx_offsets(&self) -> &[(usize, usize, f32)] {
        &self.offsets.fx
    }
}

/// Voice output buses, each a stereo pair in `out[bus·2]`, `out[bus·2 + 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    /// Into the main FX rack.
    Main = 0,
    /// Bypasses the FX racks.
    Direct = 1,
    Fx1 = 2,
    Fx2 = 3,
}

pub const BUS_CHANNELS: usize = 8;

/// Sub level relative to a wavetable oscillator at the same volume (measured −1.2 dB).
const SUB_GAIN: f32 = 0.871;
/// Noise level for full-scale uniform white noise (measured −7.4 dB against Serum).
const NOISE_GAIN: f32 = 0.427;

/// −4.5 dB pan law with centre at unity (measured: hard pan is +4.4 dB on the
/// remaining channel); `pan` ∈ [-1, 1].
fn pan_gains(pan: f32, gain: f32) -> [f32; 2] {
    const LAW: f32 = 1.5;
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    let centre = std::f32::consts::FRAC_1_SQRT_2.powf(LAW);
    // cos(π/2) is slightly negative in floating point; a negative base would make powf NaN.
    [angle.cos().max(0.0).powf(LAW) / centre * gain, angle.sin().max(0.0).powf(LAW) / centre * gain]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::MODULES;

    #[test]
    fn synced_envelope_times_follow_the_division_list() {
        // Rendered at 120 BPM: stored 0.25, 1 and 2 → 1/16 t, 1/4 t, 1/2 t.
        for (stored, secs) in [(0.25, 1.0 / 12.0), (1.0, 1.0 / 3.0), (2.0, 2.0 / 3.0)] {
            assert!((synced_env_seconds(stored, 120.0) - secs).abs() < 1e-4, "{stored}: {}", synced_env_seconds(stored, 120.0));
        }
        assert_eq!(synced_env_seconds(0.0, 120.0), 0.0);
        assert!((synced_env_seconds(32.0, 120.0) - 64.0).abs() < 1e-3, "32 bars");
    }

    #[test]
    fn synced_lfo_rates_snap_to_measured_divisions() {
        let at = |x: f32, d: bool, t: bool| synced_lfo_hz(100.0 * x.powi(4), 120.0, d, t);
        // Rendered at 120 BPM: x → Hz.
        for (x, hz) in [(0.3, 0.25), (0.4, 0.5), (0.425, 1.0), (0.5, 2.0), (0.55, 2.0), (0.575, 4.0), (0.65, 8.0), (0.725, 16.0), (0.8, 32.0)] {
            assert!((at(x, false, false) - hz).abs() < 1e-3, "{x}: {}", at(x, false, false));
        }
        assert!((at(0.6, true, false) - 16.0 / 3.0).abs() < 1e-3, "1/16 dotted");
        assert!((at(0.6, false, true) - 4.0).abs() < 1e-3);
        // Triplet label starts two thirds into the 1/4 cell (display sweep: 0.536).
        assert!((at(0.54, false, true) - 3.0).abs() < 1e-3);
        assert!((at(0.0, false, false) - 1.0 / 64.0).abs() < 1e-6, "32 bars at 120 BPM is 64 s");
    }


    #[test]
    fn rendered_entries_exist() {
        for (module, param) in RENDERED {
            let kind = crate::params::kind_of(module).unwrap_or_else(|| panic!("{module}"));
            assert!(MODULES[kind].index_of(param).is_some(), "{module}.{param}");
        }
    }

    #[test]
    fn pan_law_is_finite_at_extremes_and_unity_at_centre() {
        for pan in [-1.0, -0.5, 0.0, 0.5, 1.0, -7.0, 7.0] {
            let [l, r] = pan_gains(pan, 1.0);
            assert!(l.is_finite() && r.is_finite() && l >= 0.0 && r >= 0.0, "{pan}: {l} {r}");
        }
        let [l, r] = pan_gains(0.0, 1.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        // Measured: hard pan is +4.4 dB on the remaining channel.
        let [l, _] = pan_gains(-1.0, 1.0);
        assert!((20.0 * l.log10() - 4.5).abs() < 0.2, "{l}");
    }

    #[test]
    fn cutoff_matches_serum_display() {
        // Serum displays whole hertz.
        for (n, hz) in [(0.0, 8.0), (0.25, 59.0), (0.5, 425.0), (0.75, 3064.0), (1.0, 22050.0)] {
            let got = cutoff_hz(n);
            assert!((got - hz).abs() <= 0.6f32.max(hz * 0.002), "{n}: {got} vs {hz}");
        }
    }
}
