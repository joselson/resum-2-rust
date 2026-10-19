//! Attack-hold-decay-sustain-release envelope with per-segment curvature.

#[derive(Clone, Copy, Debug)]
pub struct EnvParams {
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    /// Curve amount per segment (attack, decay, release) in [−1, 1], applied to
    /// progress through the segment by `curve::shape`: 0 is linear, positive starts
    /// fast, negative slowly.
    pub curves: [f32; 3],
    /// Level the attack starts from (unless legato) and the release ends at (probes:
    /// start 0.3 attacks from 0.3, end 0.4 releases to 0.4 and holds).
    pub start: f32,
    pub end: f32,
}

impl Default for EnvParams {
    fn default() -> Self {
        EnvParams { attack: 0.0005, hold: 0.0, decay: 1.0, sustain: 1.0, release: 0.015, curves: [0.0; 3], start: 0.0, end: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Debug)]
pub struct Envelope {
    stage: Stage,
    /// Progress through the current segment, [0, 1].
    t: f32,
    level: f32,
    /// Level when the current segment started (release and retrigger start from here).
    from: f32,
    /// A non-legato trigger: the attack starts from the `start` level.
    fresh: bool,
}

impl Default for Envelope {
    fn default() -> Self {
        Envelope { stage: Stage::Idle, t: 0.0, level: 0.0, from: 0.0, fresh: false }
    }
}

use crate::curve::shape as bend;

impl Envelope {
    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// `legato` keeps the current level so retriggers don't click.
    pub fn trigger(&mut self, legato: bool) {
        self.from = if legato { self.level } else { 0.0 };
        self.fresh = !legato;
        self.stage = Stage::Attack;
        self.t = 0.0;
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle && self.stage != Stage::Release {
            self.from = self.level;
            self.stage = Stage::Release;
            self.t = 0.0;
        }
    }

    /// Advances `samples` steps and returns the level at the end.
    ///
    /// Each iteration either finishes the current stage or consumes every
    /// remaining sample, so the loop runs at most once per stage.
    pub fn advance(&mut self, p: &EnvParams, sample_rate: f32, samples: usize) -> f32 {
        let mut remaining = samples as f32;
        if std::mem::take(&mut self.fresh) {
            self.from = p.start;
        }
        loop {
            let duration = match self.stage {
                Stage::Idle => return self.level,
                Stage::Sustain => {
                    self.level = p.sustain;
                    return self.level;
                }
                Stage::Attack => p.attack,
                Stage::Hold => p.hold,
                Stage::Decay => p.decay,
                Stage::Release => p.release,
            };
            let len = (duration * sample_rate).max(1.0);
            let left_in_stage = (1.0 - self.t) * len;
            let finishes = remaining >= left_in_stage;
            self.t = if finishes { 1.0 } else { (self.t + remaining / len).min(1.0) };
            self.level = self.level_at(p);
            if !finishes {
                return self.level;
            }
            remaining -= left_in_stage;
            self.t = 0.0;
            self.from = self.level;
            self.stage = match self.stage {
                Stage::Attack if p.hold > 0.0 => Stage::Hold,
                Stage::Attack | Stage::Hold => Stage::Decay,
                Stage::Decay => Stage::Sustain,
                Stage::Release => {
                    self.level = p.end;
                    Stage::Idle
                }
                s => s,
            };
        }
    }

    fn level_at(&self, p: &EnvParams) -> f32 {
        match self.stage {
            Stage::Attack => self.from + (1.0 - self.from) * bend(self.t, p.curves[0]),
            Stage::Hold => 1.0,
            Stage::Decay => 1.0 + (p.sustain - 1.0) * bend(self.t, p.curves[1]),
            Stage::Release => p.end + (self.from - p.end) * (1.0 - bend(self.t, p.curves[2])),
            Stage::Idle => self.level,
            Stage::Sustain => p.sustain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_through_all_stages() {
        let p = EnvParams { attack: 0.01, hold: 0.01, decay: 0.01, sustain: 0.5, release: 0.01, ..EnvParams::default() };
        let mut e = Envelope::default();
        e.trigger(false);
        assert!((e.advance(&p, 1000.0, 5) - 0.5).abs() < 1e-3, "halfway up the attack");
        assert!((e.advance(&p, 1000.0, 5) - 1.0).abs() < 1e-3);
        assert_eq!(e.stage(), Stage::Hold);
        e.advance(&p, 1000.0, 30);
        assert_eq!(e.stage(), Stage::Sustain);
        assert!((e.level() - 0.5).abs() < 1e-6);
        e.release();
        e.advance(&p, 1000.0, 20);
        assert!(!e.is_active());
        assert_eq!(e.level(), 0.0);
    }

    #[test]
    fn release_during_attack_starts_from_current_level() {
        let p = EnvParams { attack: 1.0, release: 1.0, ..EnvParams::default() };
        let mut e = Envelope::default();
        e.trigger(false);
        let mid = e.advance(&p, 100.0, 50);
        e.release();
        let after = e.advance(&p, 100.0, 1);
        assert!(after < mid && after > mid * 0.9);
    }

    /// Regression: a float remainder after a stage boundary once underflowed to a
    /// zero step and spun forever (factory preset "ARP - Acid101", amp envelope).
    #[test]
    fn terminates_on_awkward_stage_lengths() {
        let p = EnvParams { attack: 0.000_239_684_42, hold: 0.0, decay: 0.618_478_35, sustain: 0.205_934_78, release: 0.113_769_09, curves: [0.0, 0.332, 0.332], ..EnvParams::default() };
        for sr in [44100.0, 48000.0, 96000.0] {
            let mut e = Envelope::default();
            e.trigger(false);
            for _ in 0..(sr as usize / 32) {
                e.advance(&p, sr, 32);
            }
            assert_eq!(e.stage(), Stage::Sustain);
            e.release();
            for _ in 0..(sr as usize / 32) {
                e.advance(&p, sr, 32);
            }
            assert!(!e.is_active());
        }
        // Sub-sample stages and huge blocks.
        let tiny = EnvParams { attack: 1e-9, hold: 1e-9, decay: 1e-9, sustain: 0.5, release: 1e-9, ..EnvParams::default() };
        let mut e = Envelope::default();
        e.trigger(false);
        assert_eq!(e.advance(&tiny, 48000.0, 1_000_000), 0.5);
    }

    #[test]
    fn curved_attack_starts_slowly_for_negative_amounts() {
        // Stored curve 25 (amount −0.5) measured 1/(1 + e^{5.41/2}) halfway up the attack.
        let p = EnvParams { attack: 1.0, curves: [-0.5, 0.0, 0.0], ..EnvParams::default() };
        let mut e = Envelope::default();
        e.trigger(false);
        let mid = e.advance(&p, 1000.0, 500);
        assert!((mid - 1.0 / (1.0 + 2.705f32.exp())).abs() < 1e-3, "{mid}");
    }
}
