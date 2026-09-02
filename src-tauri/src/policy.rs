//! Auto show / auto hide.
//!
//! Everything that decides *when* the island is visible lives here, so the
//! window module only ever answers "go to this state" and the media module never
//! knows a window exists.
//!
//! The rules, in priority order:
//!
//! 1. Nothing is playing and nothing is paused-and-worth-showing → **Hidden**.
//! 2. The pointer is over the capsule, or a track just started → **Expanded**.
//! 3. Otherwise → **Collapsed**.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use crate::{
    config::ConfigStore,
    media::{MediaEvent, PlaybackState},
    window::{Island, IslandState},
};

pub struct Policy {
    island: Arc<Island>,
    cfg: Arc<ConfigStore>,
    /// The pointer is inside the capsule. Set by the renderer.
    hover: AtomicBool,
    /// There is something worth showing at all.
    present: AtomicBool,
    /// What `present` would be from media alone. A transient reveal overrides
    /// `present`; this is what it restores to, so a flash cannot leave the
    /// capsule stranded on screen or wrongly hide a playing track.
    media_present: AtomicBool,
    /// A track just changed and we are briefly showing the full panel.
    peeking: AtomicBool,
    /// Retires a peek timer that a newer track change has superseded.
    peek_epoch: AtomicU64,
    /// The same, for transient reveals. Deliberately separate from `peek_epoch`:
    /// hovering cancels a peek, and if the two shared a token a pointer resting
    /// on the capsule would retire the flash timer as well — leaving `present`
    /// stuck true and the capsule on screen indefinitely with nothing playing.
    flash_epoch: AtomicU64,
    /// Stay expanded rather than collapsing back. Set by the hotkey and by the
    /// config, so the choice survives a restart.
    pinned: AtomicBool,
}

impl Policy {
    pub fn new(island: Arc<Island>, cfg: Arc<ConfigStore>) -> Arc<Self> {
        let pinned = cfg.get().always_expanded;
        Arc::new(Self {
            island,
            cfg,
            hover: AtomicBool::new(false),
            present: AtomicBool::new(false),
            media_present: AtomicBool::new(false),
            peeking: AtomicBool::new(false),
            peek_epoch: AtomicU64::new(0),
            flash_epoch: AtomicU64::new(0),
            pinned: AtomicBool::new(pinned),
        })
    }

    pub fn set_hover(&self, hovering: bool) {
        // A deliberate hover cancels the automatic peek, so moving the pointer
        // away collapses immediately instead of waiting out the leftover timer.
        if hovering {
            self.cancel_peek();
        }
        self.hover.store(hovering, Ordering::SeqCst);
        self.resolve();
    }

    pub fn on_media(self: &Arc<Self>, event: &MediaEvent) {
        let conf = self.cfg.get();

        match event {
            MediaEvent::TrackChanged(np) => {
                let worth_showing =
                    np.state.is_active() || (conf.show_while_paused && np.state == PlaybackState::Paused);
                self.set_present(worth_showing);

                // Only peek for a track that is actually playing — a paused
                // session re-reporting its metadata should not fling the panel open.
                if worth_showing && conf.auto_expand_on_track_change && np.state.is_active() {
                    self.peek(Duration::from_millis(conf.auto_expand_ms));
                    return; // `peek` resolves for us.
                }
            }
            MediaEvent::PlaybackChanged(np) => {
                let worth_showing =
                    np.state.is_active() || (conf.show_while_paused && np.state == PlaybackState::Paused);
                self.set_present(worth_showing);
                if !worth_showing {
                    self.cancel_peek();
                }
            }
            MediaEvent::Vanished => {
                self.set_present(false);
                self.cancel_peek();
            }
            // A seek must never move the island.
            MediaEvent::TimelineChanged(_) | MediaEvent::SessionsChanged(_) => return,
        }

        self.resolve();
    }

    /// Show the full panel for `dur`, then fall back to whatever the rules say.
    fn peek(self: &Arc<Self>, dur: Duration) {
        let epoch = self.peek_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.peeking.store(true, Ordering::SeqCst);
        self.resolve();

        let me = Arc::clone(self);
        let _ = std::thread::Builder::new().name("lumen-peek".into()).spawn(move || {
            std::thread::sleep(dur);
            // A newer peek (or a hover) owns the panel now; leave it alone.
            if me.peek_epoch.load(Ordering::SeqCst) == epoch {
                me.peeking.store(false, Ordering::SeqCst);
                me.resolve();
            }
        });
    }

    fn cancel_peek(&self) {
        self.peek_epoch.fetch_add(1, Ordering::SeqCst);
        self.peeking.store(false, Ordering::SeqCst);
    }

    /// Collapse the flags into a single target state and hand it to the window.
    /// `Island::set_state` is idempotent, so calling this spuriously is free.
    pub fn resolve(&self) {
        let pinned = self.pinned.load(Ordering::SeqCst);
        let next = if !self.present.load(Ordering::SeqCst) {
            IslandState::Hidden
        } else if pinned
            || self.hover.load(Ordering::SeqCst)
            || self.peeking.load(Ordering::SeqCst)
        {
            IslandState::Expanded
        } else {
            IslandState::Collapsed
        };
        self.island.set_state(next);
    }

    /// Hold the panel open instead of collapsing it between peeks.
    ///
    /// Pinning does not force the capsule *on screen* — with nothing playing
    /// there is still nothing to show, and a permanently visible empty panel is
    /// a different feature. It only decides which state it settles into when it
    /// is up.
    pub fn set_pinned(&self, pinned: bool) {
        self.pinned.store(pinned, Ordering::SeqCst);
        self.resolve();
    }

    pub fn pinned(&self) -> bool {
        self.pinned.load(Ordering::SeqCst)
    }

    /// Whether the capsule is currently on screen at all.
    pub fn visible(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }

    /// Force the island on screen — the tray's "Show now" and, later, a hotkey.
    pub fn reveal(self: &Arc<Self>) {
        self.present.store(true, Ordering::SeqCst);
        self.peek(Duration::from_millis(self.cfg.get().auto_expand_ms));
    }

    /// Put the island away on request (middle-click on the capsule).
    ///
    /// It stays hidden until the next thing worth showing — a track change or a
    /// play/pause — brings it back, which is the same rule the automatic hide
    /// already follows.
    pub fn conceal(&self) {
        self.cancel_peek();
        // An explicit "go away" also retires a transient reveal; otherwise its
        // timer would fire later and re-resolve a capsule already put away.
        self.flash_epoch.fetch_add(1, Ordering::SeqCst);
        self.hover.store(false, Ordering::SeqCst);
        self.set_present(false);
        self.resolve();
    }

    /// Both flags move together: `media_present` is the value a transient reveal
    /// restores to, so it must never drift from what media actually said.
    fn set_present(&self, worth_showing: bool) {
        self.present.store(worth_showing, Ordering::SeqCst);
        self.media_present.store(worth_showing, Ordering::SeqCst);
    }

    /// Put the capsule on screen briefly, *collapsed*, then restore whatever the
    /// media rules say.
    ///
    /// This is for feedback that is not about the track — a per-application
    /// volume change, which Windows draws no indicator for. It deliberately does
    /// not expand: the readout lives in the collapsed layer, and throwing the
    /// full panel open for a scroll gesture would be a much louder answer than
    /// the question deserves.
    pub fn flash(self: &Arc<Self>, dur: Duration) {
        let epoch = self.flash_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        // Collapsed rather than expanded: the volume readout lives in the
        // collapsed layer, so an expanded panel would hide the very thing this
        // reveal exists to show.
        self.peeking.store(false, Ordering::SeqCst);
        self.present.store(true, Ordering::SeqCst);
        self.resolve();

        let me = Arc::clone(self);
        let _ = std::thread::Builder::new().name("lumen-flash".into()).spawn(move || {
            loop {
                std::thread::sleep(dur);
                // A newer flash, or a conceal, owns the capsule now.
                if me.flash_epoch.load(Ordering::SeqCst) != epoch {
                    return;
                }
                // Never yank the capsule out from under a pointer that is
                // resting on it, or out of a drag. Wait for the hand to leave.
                if me.hover.load(Ordering::SeqCst) {
                    continue;
                }
                me.present.store(me.media_present.load(Ordering::SeqCst), Ordering::SeqCst);
                me.resolve();
                return;
            }
        });
    }

    pub fn island(&self) -> &Arc<Island> {
        &self.island
    }
}
