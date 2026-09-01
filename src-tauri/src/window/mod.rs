//! Everything that owns the island's HWND.

pub mod backdrop;
pub mod geometry;
pub mod island;
pub mod taskbar;

pub use backdrop::BackdropKind;
pub use island::{Island, IslandState, Placement, Transition};
