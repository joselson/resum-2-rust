//! Stereo feedback-delay-network reverb: 8 modulated delay lines mixed by a
//! Householder matrix, with per-line damping and input diffusion.

use crate::delay::DelayLine;

const LINES: usize = 8;
/// Mutually prime base lengths in milliseconds, spread for a dense, even decay.
const BASE_MS: [f32; LINES] = [29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.3, 73.1];
const DIFFUSER_MS: [f32; 4] = [4.7, 3.6, 12.7, 9.3];

#[derive(Clone, Copy, Debug)]
pub struct ReverbParams {
    /// Room scale, 0–1 (line lengths ×0.3 … ×2.5).
    pub size: f32,
    /// Seconds for a −60 dB decay.
    pub decay: f32,
    pub predelay_s: f32,
    /// One-pole damping cutoff inside the loop, Hz.
    pub damping_hz: f32,
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
    lp: [f32; LINES],
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
            lp: [0.0; LINES],
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
        self.lp = [0.0; LINES];
        self.hp = [0.0; 2];
    }

    /// Processes in place; returns the wet signal only.
    pub fn process(&mut self, p: &ReverbParams, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sample_rate;
        let scale = 0.3 + 2.2 * p.size.clamp(0.0, 1.0);
        let lengths: [f32; LINES] = std::array::from_fn(|i| BASE_MS[i] * scale * sr / 1000.0);
        // Per-line gain for the requested T60: g = 10^(−3·len / (T60·sr)).
        let gains: [f32; LINES] = std::array::from_fn(|i| 10f32.powf(-3.0 * lengths[i] / (p.decay.max(0.05) * sr)));
        let damp = 1.0 - (-std::f32::consts::TAU * p.damping_hz.clamp(200.0, sr * 0.45) / sr).exp();
        let hp_coef = (-std::f32::consts::TAU * p.low_cut_hz.clamp(5.0, 2000.0) / sr).exp();
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
            // Allpass diffusion.
            for (d, len) in self.diffusers.iter_mut().zip(diffuse) {
                let delayed = d.read(len);
                let v = x + 0.6 * delayed;
                d.push(v);
                x = delayed - 0.6 * v;
            }
            self.lfo = (self.lfo + 0.13 / sr).fract();
            let mut outs = [0f32; LINES];
            for i in 0..LINES {
                let wobble = (std::f32::consts::TAU * (self.lfo + i as f32 / LINES as f32)).sin() * mod_depth;
                let y = self.lines[i].read(lengths[i] + wobble);
                self.lp[i] += (y - self.lp[i]) * damp;
                outs[i] = self.lp[i] * gains[i];
            }
            // Householder feedback: v − (2/N)·Σv.
            let sum: f32 = outs.iter().sum::<f32>() * (2.0 / LINES as f32);
            for (line, &o) in self.lines.iter_mut().zip(&outs) {
                line.push(o - sum + x);
            }
            // Alternating-sign taps decorrelate the lines so steady input does not
            // cancel as the modulated delays drift.
            let wl = outs[0] - outs[2] + outs[4] - outs[6];
            let wr = outs[1] - outs[3] + outs[5] - outs[7];
            let mid = 0.5 * (wl + wr);
            let side = 0.5 * (wl - wr) * p.width.clamp(0.0, 1.0);
            *l = (mid + side) * 0.5;
            *r = (mid - side) * 0.5;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_decays_near_requested_time() {
        let sr = 48000.0;
        let mut r = Reverb::new(sr);
        let p = ReverbParams { size: 0.5, decay: 1.0, predelay_s: 0.0, damping_hz: 20000.0, low_cut_hz: 5.0, modulation: 0.0, width: 1.0 };
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
