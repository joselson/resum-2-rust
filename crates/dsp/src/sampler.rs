//! Sample playback with loop modes, crossfaded loops and cubic interpolation.

/// Decoded audio: one or two channels at `rate`.
pub struct SampleData {
    pub rate: f32,
    pub channels: Vec<Vec<f32>>,
}

impl SampleData {
    pub fn len(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Four-point Hermite read at fractional frame `pos` (clamped to the data).
    #[inline]
    fn read(&self, ch: usize, pos: f64) -> f32 {
        let data = &self.channels[ch.min(self.channels.len() - 1)];
        let n = data.len();
        if n == 0 {
            return 0.0;
        }
        let i = pos.floor() as isize;
        let f = (pos - i as f64) as f32;
        let at = |k: isize| data[k.clamp(0, n as isize - 1) as usize];
        let (y0, y1, y2, y3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * f + c2) * f + c1) * f + y1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    /// Play once from start to end.
    OneShot,
    Forward,
    /// After reaching the loop end, the loop plays backwards.
    Reverse,
    PingPong,
    /// Forward loop while held; on release, plays on past the loop to the end.
    Tailed,
}

/// Playback region in frames of the sample data.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub start: f64,
    pub end: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_mode: LoopMode,
    /// Crossfade length at the loop seam, frames.
    pub crossfade: f64,
    /// Play the whole region backwards.
    pub reverse: bool,
}

impl Default for Region {
    fn default() -> Self {
        Region::whole(0)
    }
}

impl Region {
    /// The whole sample, one-shot.
    pub fn whole(len: usize) -> Region {
        Region { start: 0.0, end: len as f64, loop_start: 0.0, loop_end: len as f64, loop_mode: LoopMode::OneShot, crossfade: 0.0, reverse: false }
    }

    fn valid_loop(&self) -> bool {
        self.loop_mode != LoopMode::OneShot && self.loop_end - self.loop_start >= 2.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Playhead {
    pos: f64,
    /// +1 forward, −1 backward.
    dir: f64,
    looping: bool,
    done: bool,
}

impl Playhead {
    pub fn trigger(&mut self, region: &Region, offset: f64) {
        self.dir = if region.reverse { -1.0 } else { 1.0 };
        self.pos = if region.reverse { region.end - 1.0 - offset } else { region.start + offset };
        self.looping = false;
        self.done = false;
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Next stereo frame; `step` is frames per output sample.
    #[inline]
    pub fn next(&mut self, data: &SampleData, r: &Region, step: f64, held: bool) -> (f32, f32) {
        if self.done || data.is_empty() {
            return (0.0, 0.0);
        }
        let read = |pos: f64| (data.read(0, pos), data.read(1, pos));
        let (mut l, mut rr) = read(self.pos);
        // Crossfade toward the loop start as the forward seam approaches.
        if r.valid_loop() && r.crossfade > 0.0 && self.dir > 0.0 && matches!(r.loop_mode, LoopMode::Forward | LoopMode::Tailed) {
            let into = self.pos - (r.loop_end - r.crossfade);
            if into > 0.0 && (held || r.loop_mode == LoopMode::Forward) {
                let t = (into / r.crossfade).clamp(0.0, 1.0) as f32;
                let (ol, or) = read(self.pos - (r.loop_end - r.loop_start));
                l += (ol - l) * t;
                rr += (or - rr) * t;
            }
        }
        self.pos += step * self.dir;
        let loop_len = r.loop_end - r.loop_start;
        let loops = r.valid_loop() && !(r.loop_mode == LoopMode::Tailed && !held);
        if loops && !r.reverse {
            match r.loop_mode {
                LoopMode::Forward | LoopMode::Tailed => {
                    if self.pos >= r.loop_end {
                        self.pos -= loop_len;
                    }
                }
                LoopMode::Reverse => {
                    if self.pos >= r.loop_end && self.dir > 0.0 {
                        self.dir = -1.0;
                        self.pos = r.loop_end - (self.pos - r.loop_end);
                        self.looping = true;
                    } else if self.looping && self.pos < r.loop_start {
                        self.pos += loop_len;
                    }
                }
                LoopMode::PingPong => {
                    if self.pos >= r.loop_end {
                        self.dir = -1.0;
                        self.pos = 2.0 * r.loop_end - self.pos;
                    } else if self.dir < 0.0 && self.pos < r.loop_start {
                        self.dir = 1.0;
                        self.pos = 2.0 * r.loop_start - self.pos;
                    }
                }
                LoopMode::OneShot => {}
            }
        }
        if self.pos >= r.end || self.pos < r.start.min(r.end) - 1.0 || self.pos < 0.0 {
            self.done = true;
        }
        (l, rr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> SampleData {
        SampleData { rate: 48000.0, channels: vec![(0..n).map(|i| i as f32).collect()] }
    }

    #[test]
    fn one_shot_plays_once_at_rate() {
        let d = ramp(100);
        let r = Region::whole(100);
        let mut p = Playhead::default();
        p.trigger(&r, 0.0);
        let out: Vec<f32> = (0..60).map(|_| p.next(&d, &r, 2.0, true).0).collect();
        assert_eq!(out[1], 2.0);
        assert!(p.is_done(), "100 frames at step 2 finish within 50 samples");
        assert_eq!(out[55], 0.0);
    }

    #[test]
    fn loops_stay_inside_their_region() {
        let d = ramp(1000);
        for mode in [LoopMode::Forward, LoopMode::Reverse, LoopMode::PingPong, LoopMode::Tailed] {
            let r = Region { loop_start: 200.0, loop_end: 400.0, loop_mode: mode, ..Region::whole(1000) };
            let mut p = Playhead::default();
            p.trigger(&r, 0.0);
            for i in 0..5000 {
                let x = p.next(&d, &r, 1.3, true).0;
                if i > 400 {
                    assert!((199.0..=401.0).contains(&x), "{mode:?} escaped: {x}");
                }
            }
            assert!(!p.is_done());
        }
    }

    #[test]
    fn tailed_loop_plays_out_after_release() {
        let d = ramp(1000);
        let r = Region { loop_start: 200.0, loop_end: 400.0, loop_mode: LoopMode::Tailed, ..Region::whole(1000) };
        let mut p = Playhead::default();
        p.trigger(&r, 0.0);
        for _ in 0..3000 {
            p.next(&d, &r, 1.0, true);
        }
        for _ in 0..1000 {
            p.next(&d, &r, 1.0, false);
        }
        assert!(p.is_done());
    }

    #[test]
    fn reverse_starts_at_the_end() {
        let d = ramp(100);
        let r = Region { reverse: true, ..Region::whole(100) };
        let mut p = Playhead::default();
        p.trigger(&r, 0.0);
        assert_eq!(p.next(&d, &r, 1.0, true).0, 99.0);
        assert_eq!(p.next(&d, &r, 1.0, true).0, 98.0);
    }
}
