//! The note-processing stage between incoming MIDI and voice allocation, where
//! Serum 2 places its arpeggiator and clip player.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteEvent {
    On { note: u8, velocity: f32 },
    Off { note: u8, velocity: f32 },
}

/// Transforms note events before they reach the voices. `samples` is the
/// length of the block being rendered, for processors that generate notes over time.
pub trait NoteProcessor: Send {
    fn process(&mut self, input: &[NoteEvent], samples: usize, out: &mut Vec<NoteEvent>);
    /// Forget held notes and pending output (transport stop, patch change).
    fn reset(&mut self);
}

/// Forwards events unchanged; the stage when no arp or clip player is active.
#[derive(Default)]
pub struct Passthrough;

impl NoteProcessor for Passthrough {
    fn process(&mut self, input: &[NoteEvent], _samples: usize, out: &mut Vec<NoteEvent>) {
        out.extend_from_slice(input);
    }

    fn reset(&mut self) {}
}
