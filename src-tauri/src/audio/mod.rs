//! System audio.
//!
//! Two levels, deliberately separate because Windows applies them at different
//! points in the pipeline: `volume` is the master endpoint slider, `session` is
//! the per-application one the Volume Mixer shows. `session` explains why both
//! are needed.
//!
//! Kept behind its own module because every call in here is COM and must stay on
//! the actor thread that owns the apartment — see `volume.rs`.

pub mod boost;
pub mod dsp;
pub mod loopback;
pub mod session;
pub mod volume;

pub use volume::{AppVolumeState, VolumeControl, VolumeState};
