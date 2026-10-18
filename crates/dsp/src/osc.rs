//! Unison wavetable oscillator.
//!
//! Pitch and spread are recomputed by `prepare` at control rate; `next` only
//! advances phases and reads the table.

use crate::warp::{WarpMode, WarpSlot, WarpState};
use crate::wavetable::Wavetable;
use crate::Rng;

pub const MAX_UNISON: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DetuneMode {
    /// Voices evenly spaced across ±detune.
    Linear,
    /// Outer voices pushed further out (spacing grows with distance from centre).
    Exponential,
    /// Fixed random offsets per voice, scaled by detune.
    Random,
    /// Dense centre, sparse edges; resembles classic supersaw spreads.
    Super,
    /// Linear spacing with alternate voices mirrored.
    Inverse,
}

#[derive(Clone, Copy, Debug)]
pub struct Unison {
    pub voices: usize,
    /// Outer-voice offset in semitones.
    pub detune_semis: f32,
    pub mode: DetuneMode,
    /// 0 = all voices centred, 1 = outer voices hard left/right.
    pub width: f32,
    /// Shift of the table position across voices, ±frames at the outer voices.
    pub position_spread: f32,
    /// Level of the centre voice(s) relative to the detuned ones, [0, 1]; 0.75 weighs
    /// all equally, 0 leaves only the detuned voices (measured: Serum stays wide at 0).
    pub blend: f32,
    /// Warp amount offset at the outer voices, per slot (unison warp, ±1).
    pub warp_spread: [f32; 2],
}

impl Default for Unison {
    fn default() -> Self {
        Unison { voices: 1, detune_semis: 0.0, mode: DetuneMode::Linear, width: 1.0, position_spread: 0.0, blend: 0.75, warp_spread: [0.0; 2] }
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
        // Detuned voices stay at full level; the centre voice(s) (one for odd counts,
        // two for even) follow blend. Equal-power normalisation keeps loudness steady.
        let centre = (unison.blend / 0.75).clamp(0.0, 4.0 / 3.0);
        let weight = |i: usize| if n > 2 && (i == n / 2 || (n.is_multiple_of(2) && i == n / 2 - 1)) { centre } else { 1.0 };
        let norm = 1.0 / (0..n).map(|i| weight(i) * weight(i)).sum::<f32>().sqrt().max(1e-6);
        let max_pos = frames.saturating_sub(1) as f32;
        for i in 0..n {
            let norm = norm * weight(i);
            let spread = if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 * 2.0 - 1.0 };
            let offset = match unison.mode {
                DetuneMode::Linear => spread,
                DetuneMode::Exponential => spread * spread.abs(),
                DetuneMode::Random => self.random_offsets[i],
                DetuneMode::Super => spread.signum() * spread.abs().powf(1.6),
                DetuneMode::Inverse => if i % 2 == 0 { spread } else { -spread },
            };
            let hz = base_hz * (offset * unison.detune_semis / 12.0).exp2();
            self.increments[i] = (hz / sample_rate).min(0.5);
            self.levels[i] = Wavetable::level_for(self.increments[i]);
            // Equal-power spread with the centre at unity, so a single voice is not attenuated.
            let pan = (spread * unison.width).clamp(-1.0, 1.0);
            let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
            self.gains_l[i] = angle.cos() * std::f32::consts::SQRT_2 * norm;
            self.gains_r[i] = angle.sin() * std::f32::consts::SQRT_2 * norm;
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
