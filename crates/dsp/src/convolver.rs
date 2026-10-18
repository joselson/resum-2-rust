//! Uniformly partitioned FFT convolution (overlap-save), stereo IR × stereo input.

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub const BLOCK: usize = 256;
const FFT: usize = 2 * BLOCK;
/// Longest IR accepted (seconds at 48 kHz ≈ 10 s).
pub const MAX_IR_SAMPLES: usize = 480_000;

pub struct Convolver {
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    /// IR partitions per channel.
    parts: [Vec<Vec<Complex32>>; 2],
    /// Frequency-domain delay line of input blocks per channel.
    history: [Vec<Vec<Complex32>>; 2],
    head: usize,
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
    fill: usize,
    scratch_in: Vec<f32>,
    scratch_spec: Vec<Complex32>,
    scratch_out: Vec<f32>,
}

impl Convolver {
    /// `ir` holds one or two channels; a mono IR feeds both outputs.
    pub fn new(ir: &[Vec<f32>]) -> Convolver {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(FFT);
        let inverse = planner.plan_fft_inverse(FFT);
        let channels: [&[f32]; 2] = match ir {
            [] => [&[], &[]],
            [m] => [m, m],
            [l, r, ..] => [l, r],
        };
        let mut scratch_in = forward.make_input_vec();
        let mut spec = forward.make_output_vec();
        let parts = channels.map(|ch| {
            let ch = &ch[..ch.len().min(MAX_IR_SAMPLES)];
            ch.chunks(BLOCK)
                .map(|chunk| {
                    scratch_in.fill(0.0);
                    scratch_in[..chunk.len()].copy_from_slice(chunk);
                    forward.process(&mut scratch_in, &mut spec).expect("sizes match");
                    spec.clone()
                })
                .collect::<Vec<_>>()
        });
        let n = parts[0].len().max(parts[1].len()).max(1);
        let bins = FFT / 2 + 1;
        Convolver {
            history: [vec![vec![Complex32::default(); bins]; n], vec![vec![Complex32::default(); bins]; n]],
            parts,
            head: 0,
            input: [vec![0.0; FFT], vec![0.0; FFT]],
            output: [vec![0.0; BLOCK], vec![0.0; BLOCK]],
            fill: 0,
            scratch_spec: forward.make_output_vec(),
            scratch_out: inverse.make_output_vec(),
            scratch_in: forward.make_input_vec(),
            forward,
            inverse,
        }
    }

    pub fn reset(&mut self) {
        for h in self.history.iter_mut().flatten() {
            h.fill(Complex32::default());
        }
        for b in self.input.iter_mut().chain(self.output.iter_mut()) {
            b.fill(0.0);
        }
        self.fill = 0;
    }

    /// Latency of one block.
    pub fn latency(&self) -> usize {
        BLOCK
    }

    /// Replaces input with the convolved (wet) signal, delayed by `latency()`.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for i in 0..left.len() {
            let pos = BLOCK + self.fill;
            self.input[0][pos] = left[i];
            self.input[1][pos] = right[i];
            left[i] = self.output[0][self.fill];
            right[i] = self.output[1][self.fill];
            self.fill += 1;
            if self.fill == BLOCK {
                self.fill = 0;
                self.run_block();
            }
        }
    }

    fn run_block(&mut self) {
        let n = self.history[0].len();
        self.head = (self.head + n - 1) % n;
        let bins = FFT / 2 + 1;
        for ch in 0..2 {
            self.scratch_in.copy_from_slice(&self.input[ch]);
            self.forward.process(&mut self.scratch_in, &mut self.history[ch][self.head]).expect("sizes match");
            self.scratch_spec.iter_mut().for_each(|c| *c = Complex32::default());
            for (k, part) in self.parts[ch].iter().enumerate() {
                let h = &self.history[ch][(self.head + k) % n];
                for b in 0..bins {
                    self.scratch_spec[b] += h[b] * part[b];
                }
            }
            // Real inverse needs purely real DC and Nyquist bins.
            self.scratch_spec[0].im = 0.0;
            self.scratch_spec[bins - 1].im = 0.0;
            self.inverse.process(&mut self.scratch_spec, &mut self.scratch_out).expect("sizes match");
            for (o, s) in self.output[ch].iter_mut().zip(&self.scratch_out[BLOCK..]) {
                *o = s / FFT as f32;
            }
            // Slide the input window by one block.
            self.input[ch].copy_within(BLOCK.., 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_direct_convolution() {
        let ir: Vec<f32> = (0..700).map(|i| ((i * 7919) % 101) as f32 / 101.0 - 0.5).collect();
        let x: Vec<f32> = (0..3000).map(|i| ((i * 104729) % 97) as f32 / 97.0 - 0.5).collect();
        let mut c = Convolver::new(std::slice::from_ref(&ir));
        let (mut l, mut r) = (x.clone(), x.clone());
        l.extend(vec![0.0; 1500]);
        r.extend(vec![0.0; 1500]);
        c.process(&mut l, &mut r);
        for n in [0usize, 10, 699, 1500, 2999] {
            let direct: f32 = (0..ir.len()).filter(|&k| k <= n).map(|k| ir[k] * x[n - k]).sum();
            assert!((l[n + BLOCK] - direct).abs() < 1e-3, "n={n}: {} vs {direct}", l[n + BLOCK]);
            assert!((r[n + BLOCK] - direct).abs() < 1e-3);
        }
    }
}
