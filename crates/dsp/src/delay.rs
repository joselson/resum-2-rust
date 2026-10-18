//! Delay line with fractional (linear or cubic) reads.

#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    /// Holds up to `max_samples` of delay.
    pub fn new(max_samples: usize) -> Self {
        DelayLine { buf: vec![0.0; max_samples.max(4) + 4], pos: 0 }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len() - 4
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 { self.buf.len() - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
    }

    /// Sample written `delay` samples ago (0 = most recent), clamped to capacity.
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(0.0, self.capacity() as f32);
        let i = d as usize;
        let f = d - i as f32;
        let n = self.buf.len();
        let a = self.buf[(self.pos + i) % n];
        let b = self.buf[(self.pos + i + 1) % n];
        a + (b - a) * f
    }

    /// Four-point Hermite read for modulated delays (chorus, flanger).
    #[inline]
    pub fn read_cubic(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, self.capacity() as f32 - 2.0);
        let i = d as usize;
        let f = d - i as f32;
        let n = self.buf.len();
        let y0 = self.buf[(self.pos + i - 1) % n];
        let y1 = self.buf[(self.pos + i) % n];
        let y2 = self.buf[(self.pos + i + 1) % n];
        let y3 = self.buf[(self.pos + i + 2) % n];
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * f + c2) * f + c1) * f + y1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_integer_and_fractional_delays() {
        let mut d = DelayLine::new(16);
        for i in 0..10 {
            d.push(i as f32);
        }
        assert_eq!(d.read(0.0), 9.0);
        assert_eq!(d.read(3.0), 6.0);
        assert!((d.read(2.5) - 6.5).abs() < 1e-6);
        assert!((d.read_cubic(3.0) - 6.0).abs() < 1e-5);
        assert_eq!(d.read(1000.0), d.read(d.capacity() as f32));
    }
}
