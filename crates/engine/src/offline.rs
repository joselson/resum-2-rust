//! Offline rendering of a held chord, shared by the CLI tools.

use crate::notes::NoteEvent;
use crate::{Patch, Synth};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct NoteJob {
    pub notes: Vec<u8>,
    /// [0, 1]
    pub velocity: f32,
    /// Seconds the notes are held.
    pub hold: f32,
    /// Seconds rendered after note-off.
    pub tail: f32,
    pub sample_rate: u32,
    pub bpm: f32,
}

/// Renders `job` and returns (left, right).
pub fn render(patch: Patch, job: &NoteJob) -> (Vec<f32>, Vec<f32>) {
    let rate = job.sample_rate as f32;
    let mut synth = Synth::new(rate, Arc::new(patch));
    synth.set_bpm(job.bpm);
    let hold = (job.hold * rate) as usize;
    let total = hold + (job.tail * rate) as usize;
    let (mut left, mut right) = (vec![0.0; total], vec![0.0; total]);
    for &n in &job.notes {
        synth.event(NoteEvent::On { note: n, velocity: job.velocity });
    }
    synth.process(&mut left[..hold], &mut right[..hold]);
    for &n in &job.notes {
        synth.event(NoteEvent::Off { note: n, velocity: 0.5 });
    }
    synth.process(&mut left[hold..], &mut right[hold..]);
    (left, right)
}
