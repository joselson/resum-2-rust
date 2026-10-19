//! Band-limited wavetables.
//!
//! Every frame is resampled to `FRAME_LEN` and stored at mip levels a quarter
//! octave apart, each keeping the lowest `harmonics` partials. Serum stays flat to
//! about 0.49·fs at every pitch with no measurable aliasing (osc2 pitch probes), so
//! a voice picks the richest level whose top harmonic folds back no lower than
//! `FOLD_LIMIT`·fs. Levels are stored at ~8 samples per harmonic (capped at
//! `FRAME_LEN`), which bounds linear-interpolation droop to about 0.5 dB; a frame
//! takes about 36k samples, 1.6× the old one-level-per-octave layout.

use realfft::RealFftPlanner;
use std::f32::consts::TAU;

pub const FRAME_LEN: usize = 2048;
const MAX_HARMONICS: usize = FRAME_LEN / 2;
/// Levels per octave of harmonic count.
const LEVELS_PER_OCTAVE: f32 = 4.0;
/// Top harmonic at most this fraction of the sample rate: anything above Nyquist
/// folds back to at least (1 − FOLD_LIMIT)·fs, above 20 kHz at 44.1 kHz and up.
const FOLD_LIMIT: f32 = 0.54;
/// Upper bound on frames; keeps a hostile file from exhausting memory (256 frames ×
/// ~36k samples ≈ 37 MB).
pub const MAX_FRAMES: usize = 256;

/// Below this many harmonics every count gets a level (quarter octaves are finer
/// than whole partials there).
const DENSE_BELOW: usize = 32;

/// Harmonic count of every level, richest first (1024, 861, 724, ..., 38, 32, 31, ..., 1).
fn level_harmonics() -> &'static [usize] {
    static COUNTS: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();
    COUNTS.get_or_init(|| {
        let mut counts: Vec<usize> = (0..)
            .map(|j| ((MAX_HARMONICS as f32) * (-(j as f32) / LEVELS_PER_OCTAVE).exp2()).round() as usize)
            .take_while(|&n| n > DENSE_BELOW)
            .collect();
        counts.extend((1..=DENSE_BELOW).rev());
        counts
    })
}

/// Stored samples per cycle for a level keeping `harmonics` partials.
fn level_len(harmonics: usize) -> usize {
    (harmonics * 8).next_power_of_two().clamp(64, FRAME_LEN)
}

struct Level {
    len: usize,
    /// `data[frame * (len + 1) + i]`, each frame padded with one wrap sample.
    data: Vec<f32>,
}

pub struct Wavetable {
    frames: usize,
    levels: Vec<Level>,
}

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
        let counts = level_harmonics();
        let mut levels: Vec<Level> = counts.iter().map(|&n| Level { len: level_len(n), data: vec![0.0; frames * (level_len(n) + 1)] }).collect();
        let mut spectrum = forward.make_output_vec();
        let mut scratch_in = forward.make_input_vec();
        // One inverse plan and buffer pair per level length.
        let mut inverses: Vec<_> = levels
            .iter()
            .map(|l| {
                let plan = planner.plan_fft_inverse(l.len);
                let (input, output) = (plan.make_input_vec(), plan.make_output_vec());
                (plan, input, output)
            })
            .collect();
        for (f, cycle) in cycles.iter().take(frames).enumerate() {
            scratch_in.copy_from_slice(&cycle[..FRAME_LEN]);
            forward.process(&mut scratch_in, &mut spectrum).expect("FFT sizes match");
            // DC is kept: frames play as stored (pulse frames carry offsets). Each
            // level's Nyquist bin is dropped.
            for ((level, &keep), (inverse, filtered, out)) in levels.iter_mut().zip(counts).zip(&mut inverses) {
                let len = level.len;
                for (bin, dst) in filtered.iter_mut().enumerate() {
                    *dst = if bin <= keep && bin < len / 2 { spectrum[bin] } else { Default::default() };
                }
                inverse.process(filtered, out).expect("FFT sizes match");
                let dst = &mut level.data[f * (len + 1)..(f + 1) * (len + 1)];
                for (d, s) in dst.iter_mut().zip(out.iter()) {
                    *d = s / FRAME_LEN as f32;
                }
                dst[len] = dst[0];
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
        let limit = FOLD_LIMIT / increment.abs().max(1e-9);
        let counts = level_harmonics();
        // Start from the log estimate, then step to the richest level within the limit.
        let mut level = if limit >= DENSE_BELOW as f32 {
            ((MAX_HARMONICS as f32 / limit).log2() * LEVELS_PER_OCTAVE).floor().clamp(0.0, (counts.len() - 1) as f32) as usize
        } else {
            counts.len() - (limit as usize).max(1)
        };
        while level > 0 && counts[level - 1] as f32 <= limit {
            level -= 1;
        }
        while level + 1 < counts.len() && counts[level] as f32 > limit {
            level += 1;
        }
        level
    }

    /// Number of mip levels.
    pub fn levels() -> usize {
        level_harmonics().len()
    }

    /// Reads at `phase` ∈ [0, 1) and fractional frame `position` ∈ [0, frames - 1].
    #[inline]
    pub fn read(&self, level: usize, position: f32, phase: f32) -> f32 {
        let Level { len, data } = &self.levels[level];
        let (len, stride) = (*len, *len + 1);
        let pos = position.clamp(0.0, (self.frames - 1) as f32);
        let f0 = pos as usize;
        let f1 = (f0 + 1).min(self.frames - 1);
        let ff = pos - f0 as f32;
        let x = phase * len as f32;
        let i = (x as usize).min(len - 1);
        let fx = x - i as f32;
        let a = &data[f0 * stride + i..f0 * stride + i + 2];
        let b = &data[f1 * stride + i..f1 * stride + i + 2];
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
        assert!((wt.read(Wavetable::levels() - 1, 0.0, 0.25) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn level_selection_keeps_the_top_octave_without_audible_folding() {
        let counts = level_harmonics();
        assert_eq!((counts[0], *counts.last().unwrap()), (MAX_HARMONICS, 1));
        for i in 0..400 {
            let hz = 20.0 * 1000f32.powf(i as f32 / 400.0);
            let level = Wavetable::level_for(hz / 48000.0);
            let top = counts[level] as f32 * hz;
            // Folded partials stay above 22 kHz; the band below ~20.5 kHz is kept.
            assert!(top <= 0.54 * 48000.0 + 1.0 || level == counts.len() - 1, "{hz} Hz -> {top}");
            // The next richer level would cross the limit, and it is at most a quarter octave richer.
            if level > 0 {
                assert!(counts[level - 1] as f32 * hz > 0.54 * 48000.0, "{hz} Hz: level {level} not the richest");
                let (richer, n) = (counts[level - 1] as f32, counts[level] as f32);
                assert!(richer <= n + 1.0 || richer <= n * 2f32.powf(0.25) * 1.01, "{richer} after {n}");
            }
            if hz < 1500.0 && level > 0 {
                assert!(top > 21000.0, "{hz} Hz keeps only {top}");
            }
        }
    }

    #[test]
    fn short_levels_read_without_droop() {
        // A harmonic just inside a level's band reads back within 0.6 dB.
        let counts = level_harmonics();
        let level = counts.iter().position(|&n| n <= 40).unwrap();
        let k = counts[level];
        let cycle: Vec<f32> = (0..FRAME_LEN).map(|i| (TAU * (k * i) as f32 / FRAME_LEN as f32).sin()).collect();
        let wt = Wavetable::from_cycles(&[cycle]);
        let n = 4096;
        let peak = (0..n).map(|i| wt.read(level, 0.0, (i as f32 + 0.37) / n as f32).abs()).fold(0.0f32, f32::max);
        assert!(peak > 0.93 && peak < 1.001, "{k} harmonics in {} samples: {peak}", level_len(k));
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
