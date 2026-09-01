//! Global input capture.
//!
//! `hotkeys` ships in Phase 1. `mouse_hook` (taskbar wheel → volume,
//! middle-click → close, Alt+middle → kill) lands in Phase 2 and will live
//! beside it behind the same module boundary.

pub mod hotkeys;
pub mod mouse_hook;
pub mod taskbar_target;

pub use hotkeys::HotkeyService;
pub use mouse_hook::{MouseAction, MouseHook};
