pub mod biquad;
pub mod convolver;
pub mod delay;
pub mod env;
pub mod filter;
pub mod hilbert;
pub mod lfo;
pub mod osc;
pub mod oversample;
pub mod reverb;
pub mod sampler;
pub mod shaper;
pub mod sub;
pub mod warp;
pub mod wav;
pub mod wavetable;

/// Equal-tempered frequency for a fractional MIDI note, A4 = `a4` Hz.
#[inline]
pub fn note_to_hz(note: f32, a4: f32) -> f32 {
    a4 * ((note - 69.0) / 12.0).exp2()
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Small xorshift generator: deterministic, allocation-free, audio-thread safe.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed.max(1))
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}
