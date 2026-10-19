//! Unison wavetable oscillator.
//!
//! Pitch and spread are recomputed by `prepare` at control rate; `next` only
//! advances phases and reads the table.

use crate::warp::{WarpMode, WarpSlot, WarpState};
use crate::wavetable::Wavetable;
use crate::Rng;

pub const MAX_UNISON: usize = 16;

/// Voice spacing across ±detune, as a function of the voice's linear position s ∈ [−1, 1]
/// (measured from resolved voice frequencies at 3–16 voices, within 0.003 of the outer offset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DetuneMode {
    /// s.
    Linear,
    /// s·|s|: dense centre.
    Exponential,
    /// Fixed random offsets per voice, scaled by detune (not measured).
    Random,
    /// 0.654·s + 0.346·s⁴ (odd): nearly linear centre, wider edges.
    Super,
    /// sin(π/2·s): sparse centre, dense edges.
    Inverse,
}

impl DetuneMode {
    fn shape(self, s: f32) -> f32 {
        match self {
            DetuneMode::Linear | DetuneMode::Random => s,
            DetuneMode::Exponential => s * s.abs(),
            DetuneMode::Super => 0.654 * s + 0.346 * s * s.abs().powi(3),
            DetuneMode::Inverse => (std::f32::consts::FRAC_PI_2 * s).sin(),
        }
    }
}

/// Unison stack: extra transposition per voice, handed out from the centre outward.
/// A voice's slot is its distance in pitch order from voice n/2, and the blend-centre
/// voice(s) take slot 0; each mode's list repeats across slots. Measured at 3, 4, 5, 7
/// and 8 voices (osc4 stack probes), every voice matching.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum UnisonStack {
    #[default]
    None,
    /// Octave1..3: [0, 12, …, 12·n].
    Octaves(u8),
    /// [0, 0, 7].
    OctaveFifth1,
    /// [0, 12, 7, 12].
    OctaveFifth2,
    /// [0, 12, 31].
    OctaveFifth3,
    /// Only the centre voice(s), down by this many semitones (12 or 24).
    Centre(u8),
}

impl UnisonStack {
    fn semitones(self, slot: usize) -> f32 {
        let pick = |list: &[f32]| list[slot % list.len()];
        match self {
            UnisonStack::None => 0.0,
            UnisonStack::Octaves(n) => (slot % (n as usize + 1)) as f32 * 12.0,
            UnisonStack::OctaveFifth1 => pick(&[0.0, 0.0, 7.0]),
            UnisonStack::OctaveFifth2 => pick(&[0.0, 12.0, 7.0, 12.0]),
            UnisonStack::OctaveFifth3 => pick(&[0.0, 12.0, 31.0]),
            UnisonStack::Centre(down) => if slot == 0 { -(down as f32) } else { 0.0 },
        }
    }
}

/// Per-voice level against a single voice, by count (measured at 2, 3, 4, 5, 7, 8 and
/// 16 voices; the rest interpolated, which the near-linear trend supports).
const UNISON_SCALE: [f32; MAX_UNISON + 1] = [1.0, 1.0, 1.056, 1.0, 0.973, 0.946, 0.920, 0.895, 0.869, 0.847, 0.824, 0.802, 0.780, 0.757, 0.735, 0.712, 0.690];

/// Weight of the centre voice(s) and of the others for blend b ∈ [0, 1]. Both shapes
/// hold at every count; at the default 0.75 the centre is 1.29× the others.
fn blend_weights(b: f32) -> (f32, f32) {
    let fade = 1.0 - 0.552 * b * b;
    (1.109 * fade, 0.049 + 1.394 * b * b * fade)
}

/// Unison pan law for pan p ∈ [−1, 1]: R − L = p exactly and R + L = 0.7915 + 0.2085·p²,
/// so a centred voice sits 8 dB below a hard-panned one in each channel.
fn unison_pan(p: f32) -> (f32, f32) {
    let sum = 0.7915 + 0.2085 * p * p;
    ((sum - p) * 0.5, (sum + p) * 0.5)
}

#[derive(Clone, Copy, Debug)]
pub struct Unison {
    pub voices: usize,
    /// Outer-voice offset in semitones.
    pub detune_semis: f32,
    pub mode: DetuneMode,
    /// 0 = all voices centred, ±1 = side voices hard left/right (negative mirrors).
    pub width: f32,
    /// Shift of the table position across voices, ±frames at the outer voices.
    pub position_spread: f32,
    /// Crossfade from the centre voice(s) (0) toward the others (1); see `blend_weights`.
    pub blend: f32,
    /// Warp amount offset at the outer voices, per slot (unison warp, ±1).
    pub warp_spread: [f32; 2],
    pub stack: UnisonStack,
}

impl Default for Unison {
    fn default() -> Self {
        Unison { voices: 1, detune_semis: 0.0, mode: DetuneMode::Linear, width: 1.0, position_spread: 0.0, blend: 0.75, warp_spread: [0.0; 2], stack: UnisonStack::None }
    }
}

#[derive(Clone)]
pub struct WtOscillator {
    phases: [f32; MAX_UNISON],
    increments: [f32; MAX_UNISON],
    gains_l: [f32; MAX_UNISON],
    gains_r: [f32; MAX_UNISON],
    positions: [f32; MAX_UNISON],
    random_offsets: [f32; MAX_UNISON],
    levels: [usize; MAX_UNISON],
    voices: usize,
    warps: [WarpSlot; 2],
    /// Per-voice warp amounts after unison spread.
    warp_amounts: [[f32; 2]; MAX_UNISON],
    warp_state: [[WarpState; 2]; MAX_UNISON],
}

impl Default for WtOscillator {
    fn default() -> Self {
        WtOscillator {
            phases: [0.0; MAX_UNISON],
            increments: [0.0; MAX_UNISON],
            gains_l: [0.0; MAX_UNISON],
            gains_r: [0.0; MAX_UNISON],
            positions: [0.0; MAX_UNISON],
            random_offsets: [0.0; MAX_UNISON],
            levels: [0; MAX_UNISON],
            voices: 1,
            warps: [WarpSlot::default(); 2],
            warp_amounts: [[0.0; 2]; MAX_UNISON],
            warp_state: [[WarpState::default(); 2]; MAX_UNISON],
        }
    }
}

impl WtOscillator {
    /// Starts a note. `random_phase` ∈ [0, 1] blends each voice from `initial_phase` toward a random phase.
    pub fn trigger(&mut self, initial_phase: f32, random_phase: f32, rng: &mut Rng) {
        for i in 0..MAX_UNISON {
            let random = rng.unit();
            self.phases[i] = (initial_phase + random_phase * random).rem_euclid(1.0);
            self.random_offsets[i] = rng.bipolar();
        }
        self.warp_state = [[WarpState::default(); 2]; MAX_UNISON];
    }

    /// Sets the two warp slots (call before `prepare`).
    pub fn set_warps(&mut self, warps: [WarpSlot; 2]) {
        self.warps = warps;
    }

    /// Recomputes per-voice increments, pans, mip levels and positions.
    pub fn prepare(&mut self, base_hz: f32, sample_rate: f32, position: f32, frames: usize, unison: &Unison) {
        let n = unison.voices.clamp(1, MAX_UNISON);
        self.voices = n;
        let (centre, side) = blend_weights(unison.blend.clamp(0.0, 1.0));
        let max_pos = frames.saturating_sub(1) as f32;
        for i in 0..n {
            let spread = if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 * 2.0 - 1.0 };
            let offset = if unison.mode == DetuneMode::Random { self.random_offsets[i] } else { unison.mode.shape(spread) };
            // The middle one or two voices take the centre blend weight and stack slot 0.
            let edge = i.min(n - 1 - i);
            let middle = edge == (n - 1) / 2;
            let stack = if n == 1 { 0.0 } else { unison.stack.semitones(if middle { 0 } else { i.abs_diff(n / 2) }) };
            let hz = base_hz * ((offset * unison.detune_semis + stack) / 12.0).exp2();
            self.increments[i] = (hz / sample_rate).min(0.5);
            self.levels[i] = Wavetable::level_for(self.increments[i]);
            if n == 1 {
                (self.gains_l[i], self.gains_r[i]) = (1.0, 1.0);
            } else {
                // Voices alternate right/left from the lowest pitch inward and left/right
                // from the highest, so neighbours in pitch sit on opposite sides; an odd
                // count's middle voice stays centred.
                let side_sign = if (edge % 2 == 0) == (i < n / 2) { 1.0 } else { -1.0 };
                let pan = if n % 2 == 1 && middle { 0.0 } else { side_sign * unison.width.clamp(-1.0, 1.0) };
                let level = UNISON_SCALE[n] * if middle { centre } else { side };
                let (l, r) = unison_pan(pan);
                (self.gains_l[i], self.gains_r[i]) = (l * level, r * level);
            }
            self.positions[i] = (position + spread * unison.position_spread).clamp(0.0, max_pos);
            for s in 0..2 {
                self.warp_amounts[i][s] = (self.warps[s].amount + spread * unison.warp_spread[s]).clamp(0.0, 1.0);
            }
        }
    }

    /// Next stereo sample. `warp_mod` holds each slot's modulator sample in [-1, 1].
    #[inline]
    pub fn next(&mut self, table: &Wavetable, warp_mod: [f32; 2]) -> (f32, f32) {
        let warped = self.warps.iter().any(|w| w.mode != WarpMode::Off);
        let (mut l, mut r) = (0.0, 0.0);
        for i in 0..self.voices {
            let mut increment = self.increments[i];
            let s = if warped {
                let slots = self.warps;
                let mut phase = self.phases[i];
                let mut sign = 1.0;
                for (k, w) in slots.iter().enumerate() {
                    if w.mode != WarpMode::Off {
                        let slot = WarpSlot { amount: self.warp_amounts[i][k], ..*w };
                        let (p, s) = slot.phase(phase, warp_mod[k], &self.warp_state[i][k]);
                        phase = p;
                        sign *= s;
                        increment *= slot.increment_scale(warp_mod[k]);
                    }
                }
                let mut x = table.read(self.levels[i], self.positions[i], phase) * sign;
                for (k, w) in slots.iter().enumerate() {
                    if w.mode != WarpMode::Off {
                        let slot = WarpSlot { amount: self.warp_amounts[i][k], ..*w };
                        let half = if w.mode.needs_half_read() { table.read(self.levels[i], self.positions[i], (phase + 0.5).fract()) } else { 0.0 };
                        x = slot.sample(x, half, warp_mod[k], self.increments[i], &mut self.warp_state[i][k]);
                    }
                }
                x
            } else {
                table.read(self.levels[i], self.positions[i], self.phases[i])
            };
            l += s * self.gains_l[i];
            r += s * self.gains_r[i];
            let p = self.phases[i] + increment.clamp(-0.5, 0.5);
            self.phases[i] = p - p.floor();
        }
        (l, r)
    }

    /// Phase of the centre voice, for sync-style consumers.
    pub fn phase(&self) -> f32 {
        self.phases[self.voices / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wavetable::Wavetable;

    #[test]
    fn single_voice_produces_expected_frequency() {
        let table = Wavetable::basic_shapes();
        let mut osc = WtOscillator::default();
        let mut rng = Rng::new(1);
        osc.trigger(0.0, 0.0, &mut rng);
        osc.prepare(441.0, 44100.0, 0.0, table.frames(), &Unison { width: 0.0, ..Unison::default() });
        // Count rising zero crossings over one second of a sine.
        let mut prev = 0.0;
        let mut crossings = 0;
        for _ in 0..44100 {
            let (l, _) = osc.next(&table, [0.0; 2]);
            if prev <= 0.0 && l > 0.0 {
                crossings += 1;
            }
            prev = l;
        }
        assert!((440..=442).contains(&crossings), "{crossings}");
    }

    #[test]
    fn unison_matches_measured_pan_level_and_stack_laws() {
        let table = Wavetable::basic_shapes();
        let mut osc = WtOscillator::default();
        osc.trigger(0.0, 0.0, &mut Rng::new(1));
        // Seven voices at blend 0.75: hard-panned sides at 0.895·0.590 (pitch order R, L, R,
        // centre, L, R, L) and a centre voice at 1.29× a side, −8 dB per channel.
        osc.prepare(100.0, 48000.0, 0.0, table.frames(), &Unison { voices: 7, detune_semis: 2.0, ..Unison::default() });
        let side = 0.895 * (0.049 + 1.394 * 0.5625 * (1.0 - 0.552 * 0.5625));
        let pans: Vec<char> = (0..7).map(|i| if osc.gains_l[i] == osc.gains_r[i] { 'C' } else if osc.gains_r[i] > osc.gains_l[i] { 'R' } else { 'L' }).collect();
        assert_eq!(pans.iter().collect::<String>(), "RLRCLRL");
        assert!((osc.gains_r[0] - side).abs() < 1e-3 && osc.gains_l[0].abs() < 1e-6, "{} {}", osc.gains_l[0], osc.gains_r[0]);
        assert!((osc.gains_l[3] / side - 1.285 * 0.3958).abs() < 0.01, "{}", osc.gains_l[3] / side);
        // Linear spacing to ±detune semitones.
        assert!((osc.increments[6] / osc.increments[3] - 2f32.powf(2.0 / 12.0)).abs() < 1e-4);
        assert!((osc.increments[4] / osc.increments[3] - 2f32.powf(2.0 / 36.0)).abs() < 1e-4);

        // Half width: R − L = width, R + L = 0.7915 + 0.2085·width².
        osc.prepare(100.0, 48000.0, 0.0, table.frames(), &Unison { voices: 7, width: 0.5, ..Unison::default() });
        let (l, r) = (osc.gains_l[0] / side, osc.gains_r[0] / side);
        assert!((r - l - 0.5).abs() < 1e-3 && (r + l - 0.8436).abs() < 1e-3, "{l} {r}");

        // Octave 2 on four voices: the inner pair stays, the top voice is up 12, the bottom up 24.
        osc.prepare(100.0, 48000.0, 0.0, table.frames(), &Unison { voices: 4, stack: UnisonStack::Octaves(2), ..Unison::default() });
        let ratios: Vec<f32> = (0..4).map(|i| osc.increments[i] / osc.increments[1]).collect();
        assert!((ratios[0] - 4.0).abs() < 1e-4 && (ratios[2] - 1.0).abs() < 1e-4 && (ratios[3] - 2.0).abs() < 1e-4, "{ratios:?}");
    }

    #[test]
    fn detune_modes_follow_measured_spacing() {
        for (mode, s, want) in [(DetuneMode::Exponential, 0.5, 0.25), (DetuneMode::Inverse, 1.0 / 3.0, 0.5), (DetuneMode::Super, 2.0 / 3.0, 0.505), (DetuneMode::Super, -1.0, -1.0)] {
            assert!((mode.shape(s) - want).abs() < 0.003, "{mode:?} {s}: {}", mode.shape(s));
        }
    }

    #[test]
    fn unison_spreads_stereo_and_preserves_level() {
        let table = Wavetable::basic_shapes();
        let mut osc = WtOscillator::default();
        let mut rng = Rng::new(7);
        osc.trigger(0.0, 1.0, &mut rng);
        let unison = Unison { voices: 7, detune_semis: 0.3, width: 1.0, ..Unison::default() };
        osc.prepare(220.0, 48000.0, 10.0, table.frames(), &unison);
        let (mut el, mut er, mut diff) = (0.0, 0.0, 0.0);
        for _ in 0..48000 {
            let (l, r) = osc.next(&table, [0.0; 2]);
            el += l * l;
            er += r * r;
            diff += (l - r).abs();
        }
        let rms = ((el + er) / 96000.0f32).sqrt();
        assert!(rms > 0.2 && rms < 1.0, "rms {rms}");
        assert!(diff > 1000.0, "unison should decorrelate channels");
    }
}
