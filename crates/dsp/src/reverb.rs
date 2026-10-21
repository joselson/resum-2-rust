//! Stereo feedback-delay-network reverb: 8 modulated delay lines mixed by a
//! Householder matrix, with per-line decay set in four bands, input diffusion and an
//! early tap of the diffused input. The output is normalised so its energy does not
//! grow with the decay time.

use crate::delay::DelayLine;

const LINES: usize = 8;
/// Mutually prime base lengths in milliseconds, spread for a dense, even decay.
const BASE_MS: [f32; LINES] = [29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.3, 73.1];
const DIFFUSER_MS: [f32; 4] = [4.7, 3.6, 12.7, 9.3];

#[derive(Clone, Copy, Debug)]
pub struct ReverbParams {
    /// Room scale, 0–1 (line lengths ×0.3 … ×2.5).
    pub size: f32,
    /// Seconds for a −60 dB decay around 500 Hz.
    pub decay: f32,
    /// Decay times around 125 Hz, 2 kHz and 8 kHz relative to `decay`.
    pub decay_low: f32,
    pub decay_upper: f32,
    pub decay_high: f32,
    /// Delay before the first output (the early tap), seconds.
    pub predelay_s: f32,
    /// Low cut on the input, Hz.
    pub low_cut_hz: f32,
    /// Delay-length modulation depth, 0–1 (chorused tails).
    pub modulation: f32,
    /// Stereo width of the output, 0–1.
    pub width: f32,
}

#[derive(Clone)]
pub struct Reverb {
    lines: Vec<DelayLine>,
    /// Band-split states per line (one-pole lowpasses at SPLIT_HZ).
    split: [[f32; 3]; LINES],
    diffusers: Vec<DelayLine>,
    predelay: DelayLine,
    hp: [f32; 2],
    lfo: f32,
    sample_rate: f32,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let max = |ms: f32| ((ms * 2.6 + 8.0) * sample_rate / 1000.0) as usize;
        Reverb {
            lines: BASE_MS.iter().map(|&ms| DelayLine::new(max(ms))).collect(),
            split: [[0.0; 3]; LINES],
            diffusers: DIFFUSER_MS.iter().map(|&ms| DelayLine::new(max(ms))).collect(),
            predelay: DelayLine::new((sample_rate * 2.6) as usize),
            hp: [0.0; 2],
            lfo: 0.0,
            sample_rate,
        }
    }

    pub fn reset(&mut self) {
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.diffusers.iter_mut().for_each(DelayLine::clear);
        self.predelay.clear();
        self.split = [[0.0; 3]; LINES];
        self.hp = [0.0; 2];
    }

    /// Processes in place; returns the wet signal only.
    pub fn process(&mut self, p: &ReverbParams, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sample_rate;
        let scale = 0.3 + 2.2 * p.size.clamp(0.0, 1.0);
        let lengths: [f32; LINES] = std::array::from_fn(|i| BASE_MS[i] * scale * sr / 1000.0);
        // Per-line gain for a T60: g = 10^(−3·len / (T60·sr)); mid and low bands.
        let gain = |len: f32, t60: f32| 10f32.powf(-3.0 * len / (t60.max(0.02) * sr));
        let decay = p.decay.max(0.02);
        let gains: [f32; LINES] = std::array::from_fn(|i| gain(lengths[i], decay));
        // Loop gain per band, solved so the summed split response reaches the target
        // decay at 125 Hz, 500 Hz, 2 kHz and 8 kHz; each line scales it by its length.
        let split_coef = SPLIT_HZ.map(|hz| 1.0 - (-std::f32::consts::TAU * hz / sr).exp());
        let mean_len = lengths.iter().sum::<f32>() / LINES as f32;
        let ratios = [p.decay_low, 1.0, p.decay_upper, p.decay_high];
        let targets = ratios.map(|q| gain(mean_len, decay * q.max(0.02)));
        let band = solve_band_gains(&split_responses(split_coef, sr), targets);
        let band_gains: [[f32; 4]; LINES] = std::array::from_fn(|i| band.map(|g| g.powf(lengths[i] / mean_len)));
        // Energy grows as 1 / (1 − g²) with recirculation; divide it out.
        let norm = (1.0 - gains.iter().map(|g| g * g).sum::<f32>() / LINES as f32).max(1e-6).sqrt() * NORM;
        let hp_coef = (-std::f32::consts::TAU * p.low_cut_hz.clamp(5.0, 2000.0) / sr).exp();
        // Width scales the side signal; keep the total energy (uncorrelated lines).
        let width = p.width.clamp(0.0, 8.0);
        let width_norm = (2.0 / (1.0 + width * width)).sqrt();
        let mod_depth = p.modulation * 0.004 * sr;
        let predelay = p.predelay_s.max(0.0) * sr;
        let diffuse: [f32; 4] = std::array::from_fn(|i| DIFFUSER_MS[i] * scale.min(1.5) * sr / 1000.0);

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.predelay.push(0.5 * (*l + *r));
            let mut x = self.predelay.read(predelay);
            // Input low cut (one-pole high-pass).
            self.hp[0] = hp_coef * (self.hp[0] + x - self.hp[1]);
            self.hp[1] = x;
            x = self.hp[0];
            // Allpass diffusion; the early tap reads the last two stages (left, right).
            let mut stages = [0f32; 4];
            for ((d, len), stage) in self.diffusers.iter_mut().zip(diffuse).zip(&mut stages) {
                let delayed = d.read(len);
                let v = x + 0.6 * delayed;
                d.push(v);
                x = delayed - 0.6 * v;
                *stage = x;
            }
            let prev = stages[2];
            self.lfo = (self.lfo + 0.13 / sr).fract();
            let mut outs = [0f32; LINES];
            for i in 0..LINES {
                let wobble = (std::f32::consts::TAU * (self.lfo + i as f32 / LINES as f32)).sin() * mod_depth;
                let y = self.lines[i].read(lengths[i] + wobble);
                // Complementary splits: each band is a lowpass of what the lower ones left.
                let mut rest = y;
                let mut out = 0.0;
                for (k, (state, coef)) in self.split[i].iter_mut().zip(split_coef).enumerate() {
                    *state += (rest - *state) * coef;
                    out += *state * band_gains[i][k];
                    rest -= *state;
                }
                outs[i] = out + rest * band_gains[i][3];
            }
            // Householder feedback: v − (2/N)·Σv.
            let sum: f32 = outs.iter().sum::<f32>() * (2.0 / LINES as f32);
            for (line, &o) in self.lines.iter_mut().zip(&outs) {
                line.push(o - sum + x);
            }
            // Alternating-sign taps decorrelate the lines so steady input does not
            // cancel as the modulated delays drift.
            let wl = (outs[0] - outs[2] + outs[4] - outs[6]) * norm + EARLY * x;
            let wr = (outs[1] - outs[3] + outs[5] - outs[7]) * norm + EARLY * prev;
            let mid = 0.5 * (wl + wr) * width_norm;
            let side = 0.5 * (wl - wr) * width * width_norm;
            *l = mid + side;
            *r = mid - side;
        }
    }
}

/// Crossovers between the decay bands (low, mid, upper, high), and the frequencies
/// whose decay each band's gain is solved for.
const SPLIT_HZ: [f32; 3] = [250.0, 1000.0, 4000.0];
const BAND_HZ: [f32; 4] = [125.0, 500.0, 2000.0, 8000.0];

/// Complex response of each complementary split band at each BAND_HZ: [freq][band],
/// including the delay lines' linear interpolation loss averaged over fractions
/// (|H|² = 1 − (1 − cos w)/3; the modulation sweeps the fraction).
fn split_responses(coef: [f32; 3], sr: f32) -> [[(f32, f32); 4]; 4] {
    let mul = |a: (f32, f32), b: (f32, f32)| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
    let sub = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0, a.1 - b.1);
    BAND_HZ.map(|hz| {
        let w = std::f32::consts::TAU * hz / sr;
        // One-pole lowpass c / (1 − (1 − c)·e^{−jw}).
        let lp = |c: f32| {
            let (re, im) = (1.0 - (1.0 - c) * w.cos(), (1.0 - c) * w.sin());
            let d = re * re + im * im;
            (c * re / d, -c * im / d)
        };
        let mut rest = (1.0, 0.0);
        let mut out = [(0.0, 0.0); 4];
        for (k, &c) in coef.iter().enumerate() {
            out[k] = mul(lp(c), rest);
            rest = sub(rest, out[k]);
        }
        out[3] = rest;
        let interp = (1.0 - (1.0 - w.cos()) / 3.0).sqrt();
        out.map(|(re, im)| (re * interp, im * interp))
    })
}

/// Real band gains g whose summed response |Σ H_k(f_j)·g_k| matches `targets[j]`.
fn solve_band_gains(h: &[[(f32, f32); 4]; 4], targets: [f32; 4]) -> [f32; 4] {
    let mut g = targets;
    for _ in 0..12 {
        for j in 0..4 {
            let (re, im) = h[j].iter().zip(&g).fold((0.0, 0.0), |(a, b), (&(hr, hi), &gk)| (a + hr * gk, b + hi * gk));
            let mag = (re * re + im * im).sqrt().max(1e-9);
            g[j] = (g[j] * targets[j] / mag).clamp(0.0, 0.99999);
        }
    }
    g
}
/// Output scale after normalisation, and the early tap's level.
const NORM: f32 = 1.0;
const EARLY: f32 = 0.25;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_decays_near_requested_time() {
        let sr = 48000.0;
        let mut r = Reverb::new(sr);
        let p = ReverbParams { size: 0.5, decay: 1.0, decay_low: 1.0, decay_upper: 1.0, decay_high: 1.0, predelay_s: 0.0, low_cut_hz: 5.0, modulation: 0.0, width: 1.0 };
        let mut l = vec![0.0; 96000];
        let mut rr = vec![0.0; 96000];
        l[0] = 1.0;
        rr[0] = 1.0;
        r.process(&p, &mut l, &mut rr);
        let energy = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>();
        let early = energy(&l[4800..14400]);
        let late = energy(&l[52800..62400]);
        // About 1 s between the windows: expect roughly −60 dB (allow broad tolerance).
        let drop = 10.0 * (late / early).log10();
        assert!(drop < -40.0 && drop > -85.0, "{drop}");
        assert!(l.iter().chain(&rr).all(|x| x.is_finite()));
    }
}
