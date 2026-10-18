//! Signal measurements for comparing two renders of the same job.
//!
//! Renders are never sample-identical (random phases, noise), so comparisons
//! use level, envelope, spectral and pitch features rather than waveforms.

use realfft::RealFftPlanner;
use serde::Serialize;

/// Envelope frame length in seconds.
const FRAME: f32 = 0.010;
/// Frames quieter than this (dBFS) in both renders are ignored.
const FLOOR_DB: f32 = -70.0;
const FFT_LEN: usize = 4096;

pub struct Audio {
    pub rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Audio {
    pub fn mono(&self) -> Vec<f32> {
        self.left.iter().zip(&self.right).map(|(l, r)| 0.5 * (l + r)).collect()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Features {
    pub rms_db: f32,
    pub peak_db: f32,
    /// Envelope in dB per `FRAME`.
    #[serde(skip)]
    pub envelope_db: Vec<f32>,
    /// Seconds from start to the envelope first reaching 3 dB below its maximum.
    pub attack_s: Option<f32>,
    /// Seconds from note-off until the level falls 60 dB below its maximum.
    pub release_s: Option<f32>,
    /// Long-term spectrum in third-octave bands (dB, relative to total power).
    #[serde(skip)]
    pub bands_db: Vec<f32>,
    pub centroid_hz: f32,
    /// Estimated fundamental during the held part.
    pub pitch_hz: Option<f32>,
    /// Side energy relative to mid energy (dB); −∞ for mono is clamped to −60.
    pub width_db: f32,
    pub silent: bool,
}

fn db(power: f64) -> f32 {
    (10.0 * power.max(1e-14).log10()) as f32
}

/// Third-octave band centres from 25 Hz to 16 kHz.
pub fn band_centres() -> Vec<f32> {
    (0..29).map(|i| 25.0 * 2f32.powf(i as f32 / 3.0)).collect()
}

pub fn analyze(audio: &Audio, note_off_s: f32) -> Features {
    let mono = audio.mono();
    let rate = audio.rate as f32;
    let energy: f64 = mono.iter().map(|&x| (x as f64).powi(2)).sum();
    let rms_db = db(energy / mono.len().max(1) as f64);
    let peak = audio.left.iter().chain(&audio.right).fold(0f32, |m, x| m.max(x.abs()));
    let peak_db = 20.0 * peak.max(1e-7).log10();
    let silent = peak < 1e-4;

    let frame = ((FRAME * rate) as usize).max(1);
    let envelope_db: Vec<f32> = mono.chunks(frame).map(|c| db(c.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / c.len() as f64)).collect();
    let env_max = envelope_db.iter().copied().fold(f32::MIN, f32::max);
    let attack_s = (!silent).then(|| envelope_db.iter().position(|&e| e >= env_max - 3.0)).flatten().map(|i| i as f32 * FRAME);
    let off_frame = (note_off_s / FRAME) as usize;
    let release_s = (!silent)
        .then(|| envelope_db.iter().skip(off_frame).position(|&e| e < env_max - 60.0))
        .flatten()
        .map(|i| i as f32 * FRAME);

    let (bands_db, centroid_hz) = spectrum(&mono, rate);
    let held_end = ((note_off_s * rate) as usize).min(mono.len());
    let pitch_hz = pitch(&mono[(held_end / 3).min(held_end)..held_end], rate);

    let (mut mid, mut side) = (0f64, 0f64);
    for (l, r) in audio.left.iter().zip(&audio.right) {
        mid += (0.5 * (l + r) as f64).powi(2);
        side += (0.5 * (l - r) as f64).powi(2);
    }
    let width_db = if mid < 1e-12 { -60.0 } else { (10.0 * (side.max(1e-12) / mid).log10() as f32).max(-60.0) };

    Features { rms_db, peak_db, envelope_db, attack_s, release_s, bands_db, centroid_hz, pitch_hz, width_db, silent }
}

/// Welch-averaged power spectrum folded into third-octave bands.
fn spectrum(x: &[f32], rate: f32) -> (Vec<f32>, f32) {
    let centres = band_centres();
    let mut bands = vec![0f64; centres.len()];
    if x.len() < FFT_LEN {
        return (vec![FLOOR_DB; centres.len()], 0.0);
    }
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);
    let window: Vec<f32> = (0..FFT_LEN).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_LEN as f32).cos()).collect();
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let mut power = vec![0f64; FFT_LEN / 2 + 1];
    for start in (0..=x.len() - FFT_LEN).step_by(FFT_LEN / 2) {
        for (i, v) in input.iter_mut().enumerate() {
            *v = x[start + i] * window[i];
        }
        fft.process(&mut input, &mut output).expect("FFT sizes match");
        for (p, c) in power.iter_mut().zip(&output) {
            *p += c.norm_sqr() as f64;
        }
    }
    let bin_hz = rate / FFT_LEN as f32;
    let (mut total, mut weighted) = (0f64, 0f64);
    for (bin, &p) in power.iter().enumerate().skip(1) {
        let f = bin as f32 * bin_hz;
        total += p;
        weighted += p * f as f64;
        // Band edges at ±1/6 octave around each centre.
        if let Some(b) = centres.iter().position(|&c| f >= c * 2f32.powf(-1.0 / 6.0) && f < c * 2f32.powf(1.0 / 6.0)) {
            bands[b] += p;
        }
    }
    let centroid = if total > 0.0 { (weighted / total) as f32 } else { 0.0 };
    let bands_db = bands.iter().map(|&b| if total > 0.0 { db(b / total).max(FLOOR_DB) } else { FLOOR_DB }).collect();
    (bands_db, centroid)
}

/// Fundamental by normalized autocorrelation (YIN-style difference function), 30 Hz–2 kHz.
fn pitch(x: &[f32], rate: f32) -> Option<f32> {
    let n = x.len().min((rate * 0.5) as usize);
    if n < 2048 {
        return None;
    }
    let x = &x[..n];
    let min_lag = (rate / 2000.0) as usize;
    let max_lag = ((rate / 30.0) as usize).min(n / 2);
    let w = n - max_lag;
    let mut d = vec![0f32; max_lag + 1];
    for (lag, slot) in d.iter_mut().enumerate().skip(1) {
        *slot = (0..w).map(|i| (x[i] - x[i + lag]).powi(2)).sum();
    }
    // Cumulative mean normalized difference.
    let mut running = 0f32;
    let mut cmnd = vec![1f32; max_lag + 1];
    for lag in 1..=max_lag {
        running += d[lag];
        cmnd[lag] = if running > 0.0 { d[lag] * lag as f32 / running } else { 1.0 };
    }
    let lag = (min_lag..max_lag).find(|&l| cmnd[l] < 0.15 && cmnd[l] <= cmnd[l + 1])?;
    // Parabolic interpolation around the minimum.
    let (a, b, c) = (cmnd[lag - 1], cmnd[lag], cmnd[lag + 1]);
    let shift = if (a - 2.0 * b + c).abs() > 1e-9 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
    Some(rate / (lag as f32 + shift))
}

#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    /// Lumen minus reference.
    pub loudness_db: f32,
    /// Mean |Δ| of level-matched envelopes over audible frames.
    pub envelope_db: f32,
    /// Mean |Δ| across audible third-octave bands (spectral balance).
    pub spectrum_db: f32,
    /// log2 ratio of centroids, in octaves.
    pub centroid_oct: f32,
    pub pitch_cents: Option<f32>,
    pub attack_s: Option<f32>,
    pub release_s: Option<f32>,
    pub width_db: f32,
    /// Weighted summary; 0 is a perfect match.
    pub score: f32,
}

pub fn compare(lumen: &Features, reference: &Features) -> Comparison {
    let loudness_db = lumen.rms_db - reference.rms_db;
    let n = lumen.envelope_db.len().min(reference.envelope_db.len());
    let (mut sum, mut count) = (0f32, 0usize);
    for i in 0..n {
        let (a, b) = (lumen.envelope_db[i] - loudness_db, reference.envelope_db[i]);
        if a > FLOOR_DB || b > FLOOR_DB {
            sum += (a.max(FLOOR_DB) - b.max(FLOOR_DB)).abs();
            count += 1;
        }
    }
    let envelope_db = if count > 0 { sum / count as f32 } else { 0.0 };
    let audible: Vec<(f32, f32)> = lumen.bands_db.iter().zip(&reference.bands_db).filter(|(a, b)| **a > -50.0 || **b > -50.0).map(|(a, b)| (*a, *b)).collect();
    let spectrum_db = if audible.is_empty() { 0.0 } else { audible.iter().map(|(a, b)| (a - b).abs()).sum::<f32>() / audible.len() as f32 };
    let centroid_oct = if lumen.centroid_hz > 0.0 && reference.centroid_hz > 0.0 { (lumen.centroid_hz / reference.centroid_hz).log2() } else { 0.0 };
    let pitch_cents = match (lumen.pitch_hz, reference.pitch_hz) {
        (Some(a), Some(b)) => Some(1200.0 * (a / b).log2()),
        _ => None,
    };
    let diff = |a: Option<f32>, b: Option<f32>| a.zip(b).map(|(a, b)| a - b);
    let width_db = lumen.width_db - reference.width_db;
    // Pitch errors under an octave count fully; octave errors are usually detector ambiguity.
    let pitch_term = pitch_cents.map(|c| (c.abs() / 100.0).min(3.0)).unwrap_or(0.0);
    let score = loudness_db.abs() / 6.0 + envelope_db / 6.0 + spectrum_db / 6.0 + centroid_oct.abs() + pitch_term + (width_db.abs() / 12.0).min(1.0);
    Comparison { loudness_db, envelope_db, spectrum_db, centroid_oct, pitch_cents, attack_s: diff(lumen.attack_s, reference.attack_s), release_s: diff(lumen.release_s, reference.release_s), width_db, score }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, secs: f32, gain: f32, rate: u32) -> Audio {
        let n = (secs * rate as f32) as usize;
        let s: Vec<f32> = (0..n).map(|i| gain * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin()).collect();
        Audio { rate, left: s.clone(), right: s }
    }

    #[test]
    fn identical_audio_scores_zero() {
        let a = analyze(&tone(220.0, 2.0, 0.5, 48000), 1.0);
        let c = compare(&a, &a);
        assert!(c.score.abs() < 1e-4, "{c:?}");
    }

    #[test]
    fn detects_level_and_pitch_offsets() {
        let a = analyze(&tone(220.0, 2.0, 0.5, 48000), 1.0);
        let b = analyze(&tone(233.08, 2.0, 0.25, 48000), 1.0);
        let c = compare(&b, &a);
        assert!((c.loudness_db + 6.02).abs() < 0.1, "{c:?}");
        assert!((c.pitch_cents.unwrap() - 100.0).abs() < 3.0, "{c:?}");
        assert!(c.envelope_db < 0.5, "level-matched envelopes should agree: {c:?}");
    }

    #[test]
    fn pitch_tracks_harmonic_tones() {
        let rate = 48000;
        let n = rate as usize;
        let saw: Vec<f32> = (0..n).map(|i| ((i as f32 * 110.0 / rate as f32).fract() * 2.0 - 1.0) * 0.3).collect();
        let f = analyze(&Audio { rate, left: saw.clone(), right: saw }, 1.0);
        assert!((f.pitch_hz.unwrap() - 110.0).abs() < 1.0, "{:?}", f.pitch_hz);
        assert!(f.width_db <= -59.0);
    }
}
