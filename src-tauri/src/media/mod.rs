//! Media session subsystem.
//!
//! The only public surface is [`MediaBackend`]. `smtc` is the Windows
//! implementation; keeping it behind a trait means the island, the tray, the
//! hotkeys and (later) Discord presence never touch WinRT.

pub mod model;
pub mod smtc;

pub use model::{
    MediaEvent, NowPlaying, PlaybackState, SessionSummary, Timeline, TransportCmd, pretty_source,
};

use tokio::sync::broadcast;

pub trait MediaBackend: Send + Sync + 'static {
    /// Every subscriber gets its own lagging-tolerant view of the event stream.
    fn subscribe(&self) -> broadcast::Receiver<MediaEvent>;

    /// Last known state, for late joiners (the WebView reconnecting after a reload).
    fn snapshot(&self) -> Option<NowPlaying>;

    /// Fire-and-forget: transport commands are queued onto the backend's own
    /// thread, because every WinRT call must happen in one apartment.
    fn control(&self, cmd: TransportCmd) -> anyhow::Result<()>;

    fn sessions(&self) -> Vec<SessionSummary>;

    /// Focus a specific source when several are publishing at once.
    fn focus(&self, session_id: &str) -> anyhow::Result<()>;

    /// Move to the next session in the list, wrapping at the end.
    ///
    /// Pins the result: an explicit choice must survive another app starting
    /// playback, otherwise the automatic "prefer whatever is playing" rule would
    /// immediately undo the user's selection.
    fn cycle(&self) -> anyhow::Result<()>;
}
