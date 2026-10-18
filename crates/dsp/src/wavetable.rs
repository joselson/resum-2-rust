//! Band-limited wavetables.
//!
//! Every frame is resampled to `FRAME_LEN` and stored at one mip level per
//! octave: level k keeps the lowest `FRAME_LEN / 2 >> k` harmonics, so a voice
//! picks the richest level whose top harmonic stays under Nyquist.

use realfft::RealFftPlanner;
use std::f32::consts::TAU;

pub const FRAME_LEN: usize = 2048;
const MAX_HARMONICS: usize = FRAME_LEN / 2;
/// Levels down to a single harmonic (1024, 512, ..., 1).
pub const MIP_LEVELS: usize = 11;
/// Upper bound on frames; keeps a hostile file from exhausting memory (256 × 11 × 8 KiB ≈ 23 MiB).
pub const MAX_FRAMES: usize = 256;

pub struct Wavetable {
    frames: usize,
    /// `levels[level][frame * STRIDE + i]`, each frame padded with one wrap sample.
    levels: Vec<Vec<f32>>,
}

const STRIDE: usize = FRAME_LEN + 1;

impl Wavetable {
    /// Builds from raw single-cycle frames of `frame_len` samples each.
    pub fn from_samples(samples: &[f32], frame_len: usize) -> Option<Wavetable> {
        if frame_len < 2 || samples.len() < frame_len {
            return None;
        }
        let frames = (samples.len() / frame_len).min(MAX_FRAMES);
        let cycles: Vec<Vec<f32>> = (0..frames).map(|f| resample_cycle(&samples[f * frame_len..(f + 1) * frame_len], FRAME_LEN)).collect();
        Some(Self::from_cycles(&cycles))
    }

    /// Builds from cycles already `FRAME_LEN` long.
    pub fn from_cycles(cycles: &[Vec<f32>]) -> Wavetable {
        let frames = cycles.len().clamp(1, MAX_FRAMES);
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(FRAME_LEN);
        let inverse = planner.plan_fft_inverse(FRAME_LEN);
        let mut levels = vec![vec![0.0; frames * STRIDE]; MIP_LEVELS];
        let mut spectrum = forward.make_output_vec();
        let mut scratch_in = forward.make_input_vec();
        let mut filtered = spectrum.clone();
        let mut out = inverse.make_output_vec();
        for (f, cycle) in cycles.iter().take(frames).enumerate() {
            scratch_in.copy_from_slice(&cycle[..FRAME_LEN]);
            forward.process(&mut scratch_in, &mut spectrum).expect("FFT sizes match");
            // DC is kept: frames play as stored (pulse frames carry offsets). Nyquist is
            // dropped at every level.
            for (level, table) in levels.iter_mut().enumerate() {
                let keep = MAX_HARMONICS >> level;
                for (bin, (dst, src)) in filtered.iter_mut().zip(&spectrum).enumerate() {
                    *dst = if bin <= keep && bin < MAX_HARMONICS { *src } else { Default::default() };
                }
                inverse.process(&mut filtered, &mut out).expect("FFT sizes match");
                let dst = &mut table[f * STRIDE..(f + 1) * STRIDE];
                for (d, s) in dst.iter_mut().zip(&out) {
                    *d = s / FRAME_LEN as f32;
                }
                dst[FRAME_LEN] = dst[0];
            }
        }
        // No normalization: frames play at their stored level.
        Wavetable { frames, levels }
    }

    /// A built-in table morphing sine → triangle → saw → square across 64 frames.
    pub fn basic_shapes() -> Wavetable {
        const FRAMES: usize = 64;
        let cycles: Vec<Vec<f32>> = (0..FRAMES)
            .map(|f| {
                let t = f as f32 / (FRAMES - 1) as f32 * 3.0;
                let (a, b, mix) = (t.floor() as usize, (t.floor() as usize + 1).min(3), t.fract());
                (0..FRAME_LEN).map(|i| {
                    let x = i as f32 / FRAME_LEN as f32;
                    shape(a, x) * (1.0 - mix) + shape(b, x) * mix
                }).collect()
            })
            .collect();
        Self::from_cycles(&cycles)
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Mip level for a phase increment in cycles per sample.
    #[inline]
    pub fn level_for(increment: f32) -> usize {
        let limit = 0.5 / increment.abs().max(1e-9);
        let mut level = 0;
        while level + 1 < MIP_LEVELS && (MAX_HARMONICS >> level) as f32 > limit {
            level += 1;
        }
        level
    }

    /// Reads at `phase` ∈ [0, 1) and fractional frame `position` ∈ [0, frames - 1].
    #[inline]
    pub fn read(&self, level: usize, position: f32, phase: f32) -> f32 {
        let table = &self.levels[level];
        let pos = position.clamp(0.0, (self.frames - 1) as f32);
        let f0 = pos as usize;
        let f1 = (f0 + 1).min(self.frames - 1);
        let ff = pos - f0 as f32;
        let x = phase * FRAME_LEN as f32;
        let i = (x as usize).min(FRAME_LEN - 1);
        let fx = x - i as f32;
        let a = &table[f0 * STRIDE + i..f0 * STRIDE + i + 2];
        let b = &table[f1 * STRIDE + i..f1 * STRIDE + i + 2];
        let sa = a[0] + (a[1] - a[0]) * fx;
        let sb = b[0] + (b[1] - b[0]) * fx;
        sa + (sb - sa) * ff
    }
}

fn shape(kind: usize, x: f32) -> f32 {
    match kind {
        0 => (TAU * x).sin(),
        // All shapes share the sine's phase so morphing between them never cancels.
        1 => (TAU * x).sin().asin() * std::f32::consts::FRAC_2_PI,
        2 => 1.0 - 2.0 * x,
        _ => if x < 0.5 { 1.0 } else { -1.0 },
    }
}

/// Linear resampling of one periodic cycle to `len` samples.
fn resample_cycle(cycle: &[f32], len: usize) -> Vec<f32> {
    if cycle.len() == len {
        return cycle.to_vec();
    }
    let n = cycle.len();
    (0..len)
        .map(|i| {
            let x = i as f32 * n as f32 / len as f32;
            let j = x as usize % n;
            let f = x - x.floor();
            cycle[j] + (cycle[(j + 1) % n] - cycle[j]) * f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_keep_stored_level_and_dc() {
        let half: Vec<f32> = (0..FRAME_LEN).map(|i| 0.5 * (TAU * i as f32 / FRAME_LEN as f32).sin() + 0.25).collect();
        let wt = Wavetable::from_cycles(&[half]);
        assert!((wt.read(0, 0.0, 0.25) - 0.75).abs() < 1e-3);
        assert!((wt.read(0, 0.0, 0.75) + 0.25).abs() < 1e-3);
    }

    #[test]
    fn sine_reads_back_and_is_band_limited() {
        let sine: Vec<f32> = (0..FRAME_LEN).map(|i| (TAU * i as f32 / FRAME_LEN as f32).sin()).collect();
        let wt = Wavetable::from_cycles(&[sine]);
        assert!((wt.read(0, 0.0, 0.25) - 1.0).abs() < 1e-3);
        // A pure sine survives even the most aggressive mip level.
        assert!((wt.read(MIP_LEVELS - 1, 0.0, 0.25) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn level_selection_keeps_harmonics_below_nyquist() {
        for hz in [20.0f32, 440.0, 5000.0, 15000.0] {
            let inc = hz / 48000.0;
            let level = Wavetable::level_for(inc);
            let top = (MAX_HARMONICS >> level) as f32 * hz;
            assert!(top <= 24000.0 || level == MIP_LEVELS - 1, "{hz} Hz -> level {level}, top {top}");
        }
    }

    #[test]
    fn frames_interpolate_and_clamp() {
        let wt = Wavetable::basic_shapes();
        assert_eq!(wt.frames(), 64);
        let a = wt.read(0, 0.0, 0.1);
        let b = wt.read(0, 63.0, 0.1);
        assert!((wt.read(0, -5.0, 0.1) - a).abs() < 1e-6);
        assert!((wt.read(0, 99.0, 0.1) - b).abs() < 1e-6);
    }

    #[test]
    fn resamples_odd_frame_lengths() {
        let samples: Vec<f32> = (0..1000).map(|i| (TAU * i as f32 / 500.0).sin()).collect();
        let wt = Wavetable::from_samples(&samples, 500).unwrap();
        assert_eq!(wt.frames(), 2);
        assert!((wt.read(0, 1.0, 0.25) - 1.0).abs() < 1e-2);
    }
}
