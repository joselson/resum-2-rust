//! Lumen's synthesis engine.
//!
//! `params` holds every module's parameter table (generated from measurements of
//! Serum 2), `patch` loads a preset through it, `modulation` resolves mod slots,
//! `notes` is the note-processing stage, and `synth` renders voices.

pub mod filters;
pub mod fx;
pub mod modulation;
pub mod notes;
pub mod offline;
pub mod params;
pub mod sampleosc;
pub mod sfz;
pub mod patch;
pub mod synth;
pub mod tables;
mod voice;

pub use patch::{Coverage, Patch};
pub use synth::{PreparedPatch, Synth};
pub use tables::TableStore;
