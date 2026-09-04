//! The island: a small, always-composited window whose rectangle *is* the
//! visible capsule.
//!
//! # Why the window is never hidden
//!
//! `hide()`/`show()` tears down and rebuilds the DWM backdrop, and Acrylic
//! visibly re-blooms every time. Instead the window is parked beyond the right
//! edge of the virtual desktop. It stays composited, stays warm, and costs
//! nothing while parked because nothing is animating and no timers are running.
//!
//! # Why the window resizes instead of being clipped
//!
//! See `geometry.rs`: `SetWindowRgn` does not clip the DWM backdrop, so a
//! window larger than its capsule paints a slab of glass around it.

use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, WebviewWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

use super::{
    backdrop::{self, BackdropKind},
    geometry::{Anchor, GeometryAnimator, HAlign, Size, VAlign},
    taskbar,
};
use crate::{
    config::{Config, ConfigStore, DockMode},
    motion::{self, duration},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IslandState {
    Hidden,
    Collapsed,
    Expanded,
}

/// Sizes in *logical* pixels; every use is scaled by the monitor's DPI factor.
///
/// These MUST match `SIZE` in `src/components/Island.svelte`. The window is
/// exactly this size, and the renderer fills it — a mismatch shows as content
/// clipped at the capsule edge or as empty glass around it.
mod logical {
    pub const COLLAPSED_W: f64 = 268.0;
    pub const COLLAPSED_H: f64 = 44.0;
    pub const EXPANDED_W: f64 = 428.0;
    pub const EXPANDED_H: f64 = 148.0;
    /// The island shrinks to this before parking, so a conceal reads as a
    /// collapse into a point rather than as a disappearance.
    pub const SEED_W: f64 = 84.0;
}

/// One transition, announced to the renderer exactly once at its start.
///
/// The host and the renderer then run the *same* easing curve independently —
/// no per-frame IPC, and no clock to keep in sync. See ARCHITECTURE.md §4.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub state: IslandState,
    pub duration_ms: u64,
}

pub struct Island {
    app: AppHandle,
    // The `WebviewWindow` itself is deliberately not retained: since
    // `geometry.rs` took over positioning, every window operation goes through
    // the HWND, and holding both invites one path moving the window behind the
    // other's back.
    hwnd: isize,
    /// The DPI scale most recently applied, so a transition that starts before
    /// the next `reposition` still animates to a correctly-sized target.
    ///
    /// This is a *cache of the last resolved value*, never the source of truth:
    /// `reposition` re-reads the destination monitor's DPI every time. An
    /// earlier version captured `scale_factor()` once at startup, which left the
    /// capsule sized for the wrong display after moving to a scaled monitor.
    scale: Arc<RwLock<f64>>,
    geometry: Arc<GeometryAnimator>,
    state: Mutex<IslandState>,
    /// Guards the deferred park: a reveal that lands during a conceal must stop
    /// the pending park from firing afterwards. Shared with the park thread.
    epoch: Arc<AtomicU64>,
    /// True while the user is physically dragging the capsule.
    ///
    /// Automatic placement has to stand down for the duration. Media events
    /// keep arriving mid-drag, and a reveal transition calls `reposition`,
    /// which would yank the window back to its dock while the pointer is still
    /// holding it — the capsule visibly fights the cursor.
    dragging: AtomicBool,
    /// A state change that arrived mid-drag and is waiting for the hand to let
    /// go. See `set_state`.
    pending_state: Mutex<Option<IslandState>>,
    /// Last mirror state published to the renderer, so an unchanged placement
    /// costs no IPC.
    mirrored: AtomicBool,
    /// When the drag last showed signs of life. See `drag_is_stale`.
    drag_activity: Mutex<Instant>,
    /// A host drag loop is running. Guards against a second one starting.
    drag_thread: AtomicBool,
    cfg: Arc<ConfigStore>,
    backdrop: BackdropKind,
}

impl Island {
    pub fn attach(
        app: AppHandle,
        window: WebviewWindow,
        cfg: Arc<ConfigStore>,
    ) -> anyhow::Result<Arc<Self>> {
        let conf = cfg.get();
        let hwnd = window.hwnd()?.0 as isize;
        // Seed from the monitor we will actually dock to, not from the window's
        // current (arbitrary, pre-placement) position.
        let scale = effective_scale(&conf, taskbar::dock_for(conf.monitor).scale);

        let dark = matches!(conf.theme, crate::config::Theme::Dark | crate::config::Theme::System);
        let backdrop = backdrop::apply(&window, conf.backdrop, dark);
        backdrop::set_dark(hwnd, dark);
        // Now that the window rectangle *is* the capsule, DWM's own corner
        // rounding is the thing that shapes it — and unlike a region, it is
        // anti-aliased.
        backdrop::set_corners(hwnd, conf.shape);

        let island = Arc::new(Self {
            app,
            hwnd,
            scale: Arc::new(RwLock::new(scale)),
            geometry: Arc::new(GeometryAnimator::new(
                hwnd,
                park_anchor(),
                seed_size(scale),
            )),
            state: Mutex::new(IslandState::Hidden),
            epoch: Arc::new(AtomicU64::new(0)),
            dragging: AtomicBool::new(false),
            pending_state: Mutex::new(None),
            mirrored: AtomicBool::new(false),
            drag_activity: Mutex::new(Instant::now()),
            drag_thread: AtomicBool::new(false),
            cfg,
            backdrop,
        });

        island.park();
        island.watch_dpi(window);
        Ok(island)
    }

    /// React to `WM_DPICHANGED`, which Tauri surfaces as `ScaleFactorChanged`.
    ///
    /// Windows sends this when the window crosses onto a display with a
    /// different scale factor, and *also* when the user changes the scale of the
    /// current display. Both mean every physical dimension the island computed
    /// is now wrong, so the fix is the same: re-resolve the destination
    /// monitor's DPI and re-place. `reposition` already re-reads DPI and
    /// re-issues the capsule size when it changes, so this just triggers it.
    fn watch_dpi(self: &Arc<Self>, window: WebviewWindow) {
        let island = Arc::clone(self);
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                tracing::info!("WM_DPICHANGED: scale factor is now {scale_factor}");
                island.reposition(&island.cfg.get());
            }
        });
    }

    pub fn backdrop_kind(&self) -> BackdropKind {
        self.backdrop
    }

    /// The capsule's window handle, for the mouse hook's "is this our window"
    /// test.
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    pub fn state(&self) -> IslandState {
        *self.state.lock().expect("island state lock poisoned")
    }

    /// Drive the island to `next`, animating and announcing the transition.
    ///
    /// Idempotent: asking for the state we are already in does nothing, so hover
    /// jitter cannot restart an animation mid-flight.
    pub fn set_state(&self, next: IslandState) {
        // A gesture in progress outranks everything. Playback pausing, a track
        // ending or the pointer slipping off the capsule all arrive as ordinary
        // state changes, and any of them mid-drag would resize the window under
        // the hand or park it off-screen entirely — the capsule vanishing while
        // still being held. The request is remembered and applied the moment the
        // user lets go.
        if self.dragging.load(Ordering::SeqCst) && !self.drag_is_stale() {
            tracing::debug!("deferring {next:?} until the drag ends");
            *self.pending_state.lock().expect("pending state lock poisoned") = Some(next);
            return;
        }

        let previous = {
            let mut guard = self.state.lock().expect("island state lock poisoned");
            if *guard == next {
                return;
            }
            std::mem::replace(&mut *guard, next)
        };

        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let conf = self.cfg.get();

        let dur = match (previous, next) {
            (IslandState::Hidden, _) => duration::REVEAL,
            (_, IslandState::Hidden) => duration::CONCEAL,
            (IslandState::Collapsed, IslandState::Expanded) => duration::EXPAND,
            _ => duration::COLLAPSE,
        };

        // Coming back from parked: dock the window and seed it small, so the
        // reveal grows out of the taskbar rather than popping into place.
        if previous == IslandState::Hidden {
            self.geometry.snap_to(seed_size(self.scale()));
            self.reposition(&conf);
        }

        let target = match next {
            IslandState::Hidden => seed_size(self.scale()),
            IslandState::Collapsed => collapsed_size(self.scale()),
            IslandState::Expanded => expanded_size(self.scale()),
        };
        self.geometry.animate_to(target, dur);

        let _ = self.app.emit(
            crate::ipc::EVT_TRANSITION,
            Transition { state: next, duration_ms: dur.as_millis() as u64 },
        );

        // Park only after the conceal has actually played out, and only if no
        // newer transition happened in the meantime.
        if next == IslandState::Hidden {
            let geometry = Arc::clone(&self.geometry);
            let epoch_ref = Arc::clone(&self.epoch);
            let _ = std::thread::Builder::new().name("lumen-park".into()).spawn(move || {
                std::thread::sleep(dur + Duration::from_millis(20));
                // A reveal during the conceal bumps the epoch; parking now would
                // hide a window the user just asked to see.
                if epoch_ref.load(Ordering::SeqCst) == epoch {
                    geometry.set_anchor(park_anchor());
                }
            });
        }
    }

    /// Recompute the dock point *and* the DPI scale for the destination monitor.
    ///
    /// Both have to move together: on a mixed-DPI setup the gap, the capsule
    /// size and the anchor are all in the destination's pixels, so resolving the
    /// scale anywhere else produces a capsule that is the right shape for the
    /// wrong display.
    pub fn reposition(&self, conf: &Config) {
        // The user's hand outranks the layout engine.
        if self.dragging.load(Ordering::SeqCst) {
            return;
        }

        let dock = taskbar::dock_for(conf.monitor);
        let scale = effective_scale(conf, dock.scale);

        let changed = {
            let mut guard = self.scale.write().expect("scale lock poisoned");
            let changed = (*guard - scale).abs() > f64::EPSILON;
            *guard = scale;
            changed
        };

        // Gaps are configured in logical pixels and scaled by the *destination*
        // monitor, so the island sits the same visual distance from the edge on
        // a 100% display and a 200% one.
        let (gap, margin) = scaled_insets(conf, scale);
        let work = dock.work;

        let _ = work;
        let anchor = anchor_for(conf, &dock, gap, margin);
        self.geometry.set_anchor(anchor);
        self.publish_mirror(mirrored_for(anchor, self.geometry.size(), &dock));

        // Moving between displays of different scale changes the capsule's
        // physical size, so re-issue it rather than waiting for the next
        // transition to notice.
        if changed && self.state() != IslandState::Hidden {
            tracing::info!("effective scale is now {scale:.2}; resizing");
            let target = match self.state() {
                IslandState::Expanded => expanded_size(scale),
                _ => collapsed_size(scale),
            };
            self.geometry.snap_to(target);
        }
    }

    fn scale(&self) -> f64 {
        *self.scale.read().expect("scale lock poisoned")
    }

    /// Whether the capsule is currently laid out right-to-left.
    pub fn is_mirrored(&self) -> bool {
        self.mirrored.load(Ordering::SeqCst)
    }

    /// Tell the renderer which way round the contents belong, if it changed.
    fn publish_mirror(&self, mirrored: bool) {
        if self.mirrored.swap(mirrored, Ordering::SeqCst) == mirrored {
            return;
        }
        let _ = self.app.emit(crate::ipc::EVT_PLACEMENT, Placement { mirrored });
    }

    /// Re-apply everything a config edit can affect, without a restart.
    pub fn apply_config(&self, conf: &Config) {
        let dark = matches!(conf.theme, crate::config::Theme::Dark | crate::config::Theme::System);
        backdrop::set_dark(self.hwnd, dark);
        backdrop::set_corners(self.hwnd, conf.shape);

        if self.state() != IslandState::Hidden {
            self.reposition(conf);
        }
    }

    /// Current top-left of the window, in screen pixels.
    pub fn origin(&self) -> (i32, i32) {
        let (anchor, size) = self.geometry.placement();
        anchor.origin(size)
    }

    /// Whether the cursor is physically inside the capsule right now.
    ///
    /// Asked of Windows rather than inferred from a `pointerleave`, because the
    /// WebView does not reliably send one when the window moves or resizes out
    /// from under a stationary pointer — which is exactly what a collapse does.
    /// The window rectangle is read fresh: mid-animation it is neither the
    /// collapsed nor the expanded size, and a cached one would answer for a
    /// shape that is not on screen.
    pub fn contains_cursor(&self) -> bool {
        use windows::Win32::{
            Foundation::{POINT, RECT},
            UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect},
        };

        let hwnd = windows::Win32::Foundation::HWND(self.hwnd as *mut _);
        let mut rect = RECT::default();
        let mut cursor = POINT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut rect).is_err() || GetCursorPos(&mut cursor).is_err() {
                // Unanswerable: say "still inside" so a failed check can never
                // collapse a capsule the pointer is actually resting on.
                return true;
            }
        }

        (rect.left..rect.right).contains(&cursor.x) && (rect.top..rect.bottom).contains(&cursor.y)
    }

    /// Whether a drag has gone quiet for long enough to be treated as over.
    ///
    /// A live drag sends `drag_to` on every animation frame, so silence means
    /// the gesture ended without the host being told. That has happened twice
    /// now — once when the capsule was hidden mid-gesture, and once because a
    /// click with no movement returned without a matching end — and the symptom
    /// is severe and permanent: placement frozen and every state change
    /// deferred, for the rest of the session.
    ///
    /// Both specific causes are fixed. This exists because the host should not
    /// be one renderer bug away from locking up, and because the failure is
    /// invisible until someone notices the island has stopped responding.
    fn drag_is_stale(&self) -> bool {
        const DRAG_STALE: Duration = Duration::from_secs(3);
        let quiet = self
            .drag_activity
            .lock()
            .map(|at| at.elapsed() > DRAG_STALE)
            .unwrap_or(true);
        if quiet {
            tracing::warn!("drag went quiet for over 3s; treating it as finished");
            self.dragging.store(false, Ordering::SeqCst);
        }
        quiet
    }

    fn mark_drag_activity(&self) {
        if let Ok(mut at) = self.drag_activity.lock() {
            *at = Instant::now();
        }
    }

    /// Take manual control of placement until `end_drag`.
    pub fn begin_drag(&self) {
        // A request left over from a gesture that ended abnormally must not be
        // applied to this one.
        *self.pending_state.lock().expect("pending state lock poisoned") = None;
        self.mark_drag_activity();
        self.dragging.store(true, Ordering::SeqCst);
    }

    /// Apply a state change that arrived while the capsule was being dragged.
    ///
    /// `after` holds it back until an in-flight snap glide has landed.
    /// `GeometryAnimator` runs one animation at a time — a size change issued
    /// while the snap is still travelling bumps the generation and strands the
    /// glide wherever it had got to.
    fn flush_pending_state(self: &Arc<Self>, after: Duration) {
        let Some(next) = self.pending_state.lock().expect("pending state lock poisoned").take()
        else {
            return;
        };
        if next == self.state() {
            return;
        }

        tracing::debug!("drag ended; applying deferred {next:?} in {}ms", after.as_millis());

        if after.is_zero() {
            self.set_state(next);
            return;
        }

        // Anything that changes state during the glide is newer than what we are
        // holding, so the held value is stale and must be dropped rather than
        // replayed over the top of it.
        let epoch = self.epoch.load(Ordering::SeqCst);
        let me = Arc::clone(self);
        let _ = std::thread::Builder::new().name("lumen-drag-flush".into()).spawn(move || {
            std::thread::sleep(after + Duration::from_millis(20));
            if me.epoch.load(Ordering::SeqCst) == epoch {
                me.set_state(next);
            }
        });
    }

    /// Move the capsule to a raw screen origin, with no animation.
    ///
    /// Used for every frame of a drag: interpolation here would put the capsule
    /// behind the pointer for the whole gesture, which is the same mistake the
    /// seek bar made.
    ///
    /// The position is clamped so a usable piece of the capsule always remains
    /// on a monitor. Without that it is possible to drag the island entirely
    /// off the desktop, at which point there is nothing left to grab and no way
    /// to get it back short of editing the config by hand.
    pub fn drag_to(&self, x: i32, y: i32) {
        self.mark_drag_activity();
        let (x, y) = clamp_on_screen(x, y, self.geometry.size());
        self.geometry.set_anchor(Anchor::at(x, y));
    }

    /// Drag the capsule from the host, following the real cursor.
    ///
    /// The renderer used to own this: pointer capture on the WebView, a
    /// `pointermove` per frame, and a `drag_to` per move. It broke exactly as
    /// you would expect once the window started moving out from under the
    /// pointer — a fast flick outran the window, the WebView lost capture, and
    /// `lostpointercapture` aborted the gesture mid-flight. The pointer had not
    /// gone anywhere; the events had.
    ///
    /// So the gesture is read from the source instead. `GetCursorPos` is the
    /// real cursor wherever it is, including over other windows and off the
    /// edge of the desktop, and `GetAsyncKeyState` is the real button. Neither
    /// can be lost by a window that moved.
    ///
    /// Costs one thread for the length of the gesture and nothing afterwards.
    pub fn start_host_drag(self: &Arc<Self>, cfg: Arc<ConfigStore>, app: AppHandle) {
        use windows::Win32::UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
            WindowsAndMessaging::GetCursorPos,
        };

        if self.drag_thread.swap(true, Ordering::SeqCst) {
            // A gesture is already running. A second pointerdown without an up
            // means the first one is stale, and two loops fighting over the
            // window would be worse than either.
            return;
        }

        self.begin_drag();

        let me = Arc::clone(self);
        let _ = std::thread::Builder::new().name("lumen-drag".into()).spawn(move || {
            let cursor = || -> Option<(i32, i32)> {
                let mut p = windows::Win32::Foundation::POINT::default();
                unsafe { GetCursorPos(&mut p) }.ok().map(|()| (p.x, p.y))
            };
            let pressed =
                || unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 };

            let Some(start_cursor) = cursor() else {
                me.dragging.store(false, Ordering::SeqCst);
                me.drag_thread.store(false, Ordering::SeqCst);
                return;
            };
            let start_origin = me.origin();

            // Velocity, from the last few samples: a flick should still coast
            // into the corner it was aimed at.
            let mut samples: Vec<((i32, i32), Instant)> = Vec::with_capacity(8);
            let mut last = start_cursor;
            let mut moved = false;
            let began = Instant::now();

            loop {
                if !pressed() {
                    break;
                }
                // A gesture that has run for two minutes is a stuck button or a
                // lost release, not a drag.
                if began.elapsed() > Duration::from_secs(120) {
                    tracing::warn!("drag ran for two minutes; ending it");
                    break;
                }

                if let Some(now) = cursor() {
                    if now != last {
                        last = now;
                        moved = moved
                            || (now.0 - start_cursor.0).abs() > DRAG_SLOP
                            || (now.1 - start_cursor.1).abs() > DRAG_SLOP;
                        samples.push((now, Instant::now()));
                        if samples.len() > 6 {
                            samples.remove(0);
                        }
                        if moved {
                            me.drag_to(
                                start_origin.0 + (now.0 - start_cursor.0),
                                start_origin.1 + (now.1 - start_cursor.1),
                            );
                        }
                    }
                }

                // ~120 Hz: finer than any display refresh, coarse enough that
                // the thread is asleep almost all of the time.
                std::thread::sleep(Duration::from_millis(8));
            }

            let conf = cfg.get();
            if !moved {
                // A press that never moved is a click. The placement is left
                // alone, but the host still has to leave drag mode.
                me.cancel_drag(&conf);
                me.drag_thread.store(false, Ordering::SeqCst);
                return;
            }

            let velocity = release_velocity(&samples);
            let (x, y) = (
                start_origin.0 + (last.0 - start_cursor.0),
                start_origin.1 + (last.1 - start_cursor.1),
            );
            let (dock, free_x, free_y) = me.end_drag(x, y, velocity, &conf);
            let stored = cfg.update(|c| {
                c.dock = dock;
                c.free_x = free_x;
                c.free_y = free_y;
            });
            let _ = app.emit(crate::ipc::EVT_CONFIG, &stored);
            me.drag_thread.store(false, Ordering::SeqCst);
        });
    }

    /// Finish a drag: snap to the nearest anchor if close enough, otherwise keep
    /// the drop position. Returns the resulting mode so the caller can persist it.
    ///
    /// Distances are measured between window *origins* rather than centres, so
    /// "50 px from the anchor" means the capsule ends up within 50 px of exactly
    /// where that dock would place it — which is what the threshold should mean.
    pub fn end_drag(
        self: &Arc<Self>,
        x: i32,
        y: i32,
        velocity: (f64, f64),
        conf: &Config,
    ) -> (DockMode, i32, i32) {
        // Released before any of the placement below, so the snap glide and any
        // later automatic reposition both apply normally again.
        self.dragging.store(false, Ordering::SeqCst);

        let dock = taskbar::dock_for(conf.monitor);
        let (gap, margin) = scaled_insets(conf, dock.scale);
        let size = self.geometry.size();

        // Same clamp as the drag itself, in case a release arrives with a
        // position the drag frames never applied.
        let (x, y) = clamp_on_screen(x, y, size);

        let (vx, vy) = velocity;
        let speed = (vx * vx + vy * vy).sqrt();

        // Where inertia would carry the capsule if it kept coasting. A flick
        // aimed at a corner should land there even when the fingers let go well
        // short of it — the gesture expresses intent, not just position.
        let predicted = (
            x + (vx * COAST_MS).round() as i32,
            y + (vy * COAST_MS).round() as i32,
        );

        let base = (conf.snap_threshold.max(0) as f64 * dock.scale).round() as i32;
        // The predicted point is an estimate, so it gets a wider catchment than
        // the release point, which is exact.
        let flick = speed >= FLICK_SPEED;
        let predicted_threshold = if flick { base * 2 } else { 0 };

        let mut best: Option<(DockMode, i64, bool)> = None;
        for mode in SNAP_MODES {
            let probe = Config { dock: mode, ..conf.clone() };
            let (ax, ay) = anchor_for(&probe, &dock, gap, margin).origin(size);

            let release_sq = dist_sq(ax, ay, x, y);
            let predicted_sq = dist_sq(ax, ay, predicted.0, predicted.1);

            // Either route can qualify: the release point within the normal
            // threshold, or the predicted landing within the wider one.
            let by_release = release_sq <= sq(base);
            let by_flick = predicted_threshold > 0 && predicted_sq <= sq(predicted_threshold);
            if !by_release && !by_flick {
                continue;
            }

            // Rank by whichever measure actually qualified, so a flick is judged
            // on where it was heading rather than where it was let go.
            let score = if by_flick && !by_release { predicted_sq } else { release_sq };
            if best.is_none_or(|(_, b, _)| score < b) {
                best = Some((mode, score, by_flick && !by_release));
            }
        }

        if let Some((mode, score, via_flick)) = best {
            let probe = Config { dock: mode, ..conf.clone() };
            let target = anchor_for(&probe, &dock, gap, margin);
            let (tx, ty) = target.origin(size);
            let travel = (dist_sq(tx, ty, x, y) as f64).sqrt();

            let dur = glide_duration(travel, speed);
            tracing::info!(
                "snapping to {mode:?} ({:.0}px away{}, speed {:.2}px/ms, travel {:.0}px, glide {}ms)",
                (score as f64).sqrt(),
                if via_flick { " via flick prediction" } else { "" },
                speed,
                travel,
                dur.as_millis()
            );
            self.geometry.animate_anchor_to(target, dur, motion::MOMENTUM);
            self.publish_mirror(mirrored_for(target, size, &dock));
            // Held back until the snap lands, so a track that ended mid-drag
            // conceals *after* the capsule has flown home rather than during.
            self.flush_pending_state(dur);
            return (mode, conf.free_x, conf.free_y);
        }

        // Too far from anything: keep it where it was dropped, stored relative
        // to the work area in logical pixels.
        let free_x = ((x - dock.work.left) as f64 / dock.scale).round() as i32;
        let free_y = ((y - dock.work.top) as f64 / dock.scale).round() as i32;
        tracing::info!(
            "no anchor within reach (speed {speed:.2}px/ms); keeping free position"
        );
        self.publish_mirror(mirrored_for(Anchor::at(x, y), size, &dock));
        self.flush_pending_state(Duration::ZERO);
        (DockMode::Free, free_x, free_y)
    }

    /// Abandon a drag without applying a placement, for paths that end a
    /// gesture abnormally (the capsule hidden underneath it, pointer capture
    /// lost to another window).
    pub fn cancel_drag(self: &Arc<Self>, conf: &Config) {
        if self.dragging.swap(false, Ordering::SeqCst) {
            tracing::info!("drag cancelled; restoring the configured placement");
            self.reposition(conf);
            // `reposition` places instantly, so there is no glide to wait out.
            self.flush_pending_state(Duration::ZERO);
        }
    }

    fn park(&self) {
        self.dragging.store(false, Ordering::SeqCst);
        self.geometry.set_anchor(park_anchor());
    }
}

/// Resolve a docking mode to an anchor on a given monitor.
///
/// The work area already excludes the taskbar on whichever edge it lives, so
/// docking against it is correct for a taskbar on any side, for auto-hide, and
/// for third-party appbars — without special-casing any of them.
/// The monitor's DPI scale multiplied by the user's own zoom.
///
/// Windows' own scale is what makes the capsule the same *physical* size on
/// every display; `ui_scale` is a preference on top of it, for a 4K panel run
/// at 100% where everything correct is also everything tiny. Clamped because
/// the capsule's proportions stop working long before the extremes: below 0.75
/// the text is unreadable, above 2.0 it stops being a capsule.
pub fn effective_scale(conf: &Config, dpi_scale: f64) -> f64 {
    dpi_scale * f64::from(conf.ui_scale).clamp(0.75, 2.0)
}

/// The gap and edge margin in physical pixels for a monitor at `scale`.
///
/// Both are configured in *logical* pixels so the capsule sits the same visual
/// distance from the edge on a 100% display and a 200% one. Scaling by the
/// destination monitor is the whole point on a mixed-DPI desktop.
fn scaled_insets(conf: &Config, scale: f64) -> (i32, i32) {
    (
        (conf.taskbar_gap as f64 * scale).round() as i32,
        (conf.edge_margin as f64 * scale).round() as i32,
    )
}

fn anchor_for(conf: &Config, dock: &taskbar::Dock, gap: i32, margin: i32) -> Anchor {
    let work = dock.work;
    match conf.dock {
        DockMode::TaskbarCenter => Anchor {
            x: work.left + dock.work_width() / 2,
            y: work.bottom - gap,
            h: HAlign::Center,
            v: VAlign::Bottom,
        },
        DockMode::BottomLeft => Anchor {
            x: work.left + margin,
            y: work.bottom - gap,
            h: HAlign::Left,
            v: VAlign::Bottom,
        },
        DockMode::BottomRight => Anchor {
            x: work.right - margin,
            y: work.bottom - gap,
            h: HAlign::Right,
            v: VAlign::Bottom,
        },
        DockMode::TopLeft => Anchor {
            x: work.left + margin,
            y: work.top + gap,
            h: HAlign::Left,
            v: VAlign::Top,
        },
        DockMode::TopRight => Anchor {
            x: work.right - margin,
            y: work.top + gap,
            h: HAlign::Right,
            v: VAlign::Top,
        },
        // Stored relative to the work area so the island lands sensibly after a
        // resolution change instead of ending up off-screen.
        DockMode::Free => Anchor::at(
            work.left + (conf.free_x as f64 * dock.scale).round() as i32,
            work.top + (conf.free_y as f64 * dock.scale).round() as i32,
        ),
    }
}

/// How much of the capsule must stay within the desktop while dragging.
///
/// Enough to grab and drag back. A fully off-screen capsule is unrecoverable
/// through the UI, which is a trap rather than a feature.
const MIN_VISIBLE: i32 = 56;

/// Keep a window origin within reach of the virtual desktop.
///
/// The virtual screen spans every monitor, so this permits dragging onto any
/// display and even hanging off a side — it only prevents the capsule leaving
/// entirely.
fn clamp_on_screen(x: i32, y: i32, size: Size) -> (i32, i32) {
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 0 || height <= 0 {
        return (x, y);
    }
    clamp_within((left, top, left + width, top + height), x, y, size)
}

/// The arithmetic behind `clamp_on_screen`, with the desktop passed in.
///
/// Split out to be testable, and because the rule is worth stating exactly:
/// horizontally the capsule may hang off either side as long as a grabbable
/// strip remains, but its *top edge* may never rise above the desktop. A window
/// pushed up off the top can still be partly visible and yet impossible to drag
/// back, because the part you grab is the part that has gone.
///
/// The height also has to survive a later collapse. The clamp runs against
/// whatever size is on screen at the time, and the capsule loses a hundred
/// pixels of height when it collapses — which is how a drag released at the top
/// while expanded ended up with the collapsed capsule half off the screen.
/// Clamping the top edge rather than a fraction of the height is what makes
/// that safe at any size.
fn clamp_within(bounds: (i32, i32, i32, i32), x: i32, y: i32, size: Size) -> (i32, i32) {
    let (left, top, right, bottom) = bounds;
    let keep_x = MIN_VISIBLE.min(size.w);
    let keep_y = MIN_VISIBLE.min(size.h);

    (
        x.clamp(left - (size.w - keep_x), right - keep_x),
        y.clamp(top, (bottom - keep_y).max(top)),
    )
}

/// Pointer speed at release, in px/ms, from the tail of the sample buffer.
///
/// Measured over the last ~80 ms rather than the last event: a single pair of
/// samples 8 ms apart turns one jittery pixel into a phantom flick.
fn release_velocity(samples: &[((i32, i32), Instant)]) -> (f64, f64) {
    let Some((last, at)) = samples.last() else { return (0.0, 0.0) };
    let window = Duration::from_millis(80);
    let Some((first, first_at)) = samples
        .iter()
        .find(|(_, t)| at.duration_since(*t) <= window)
        .or_else(|| samples.first())
    else {
        return (0.0, 0.0);
    };

    let ms = at.duration_since(*first_at).as_secs_f64() * 1000.0;
    if ms < 1.0 {
        return (0.0, 0.0);
    }
    (
        f64::from(last.0 - first.0) / ms,
        f64::from(last.1 - first.1) / ms,
    )
}

/// How far the cursor must travel before a press becomes a drag, in pixels.
const DRAG_SLOP: i32 = 3;

/// How long the capsule is assumed to keep coasting after release, in ms.
/// Multiplied by the release velocity to predict where a flick was aimed.
const COAST_MS: f64 = 180.0;

/// Release speed, in px/ms, above which a gesture counts as a flick and the
/// predicted landing point is allowed to select a target. Roughly a brisk
/// throw; an ordinary drag-and-place stays well under it.
const FLICK_SPEED: f64 = 0.6;

/// Bounds on the snap glide.
const GLIDE_MIN_MS: f64 = 350.0;
const GLIDE_MAX_MS: f64 = 550.0;

#[inline]
fn sq(v: i32) -> i64 {
    i64::from(v) * i64::from(v)
}

#[inline]
fn dist_sq(ax: i32, ay: i32, bx: i32, by: i32) -> i64 {
    let dx = i64::from(ax - bx);
    let dy = i64::from(ay - by);
    dx * dx + dy * dy
}

/// Choose a glide duration whose *opening speed* matches the speed the pointer
/// was travelling at when it let go.
///
/// This is the whole trick behind a flick that does not visibly hitch. A
/// bezier's initial slope is `y1/x1` — for the momentum curve, 10 — and the
/// animation's starting speed is therefore `slope * distance / duration`.
/// Setting that equal to the release speed and solving for duration makes the
/// glide begin at exactly the velocity the drag ended at, so the two motions
/// join without a seam. A fixed duration cannot do this: it either lurches
/// (too fast) or stalls (too slow) at the moment of release.
///
/// The result is clamped, so a wild flick cannot produce an absurdly short
/// glide and a gentle one cannot produce a crawl.
fn glide_duration(distance: f64, speed: f64) -> Duration {
    let ms = if speed > 0.05 {
        motion::MOMENTUM.initial_slope() * distance / speed
    } else {
        // Barely moving: no momentum to match, so use the middle of the range.
        (GLIDE_MIN_MS + GLIDE_MAX_MS) / 2.0
    };
    Duration::from_millis(ms.clamp(GLIDE_MIN_MS, GLIDE_MAX_MS) as u64)
}

#[cfg(test)]
mod tests {
    /// A capsule that cannot be reached is worse than one in an odd place.
    #[test]
    fn the_top_edge_never_leaves_the_desktop() {
        let desktop = (0, 0, 1920, 1080);
        let collapsed = Size { w: 268, h: 44 };

        let (x, y) = clamp_within(desktop, -900, -400, collapsed);
        assert_eq!(y, 0, "the top edge went above the desktop");
        assert!(x > -268, "the whole capsule went off the left edge");

        // The case that actually happened: dragged while expanded, released
        // above the top, then collapsed to a third of the height.
        let expanded = Size { w: 428, h: 148 };
        let (_, y) = clamp_within(desktop, 500, -22, expanded);
        assert_eq!(y, 0);
    }

    #[test]
    fn a_grabbable_strip_always_remains() {
        let desktop = (0, 0, 1920, 1080);
        let size = Size { w: 268, h: 44 };

        let (x, y) = clamp_within(desktop, 5000, 5000, size);
        assert!(x <= 1920 - MIN_VISIBLE.min(size.w), "nothing left to grab on the right");
        assert!(y <= 1080 - MIN_VISIBLE.min(size.h), "nothing left to grab at the bottom");

        let (x, _) = clamp_within(desktop, -5000, 100, size);
        assert!(x + size.w >= MIN_VISIBLE.min(size.w), "nothing left to grab on the left");
    }

    #[test]
    fn a_position_already_on_screen_is_left_alone() {
        let desktop = (0, 0, 1920, 1080);
        let size = Size { w: 268, h: 44 };
        assert_eq!(clamp_within(desktop, 800, 900, size), (800, 900));
    }

    #[test]
    fn a_monitor_left_of_the_primary_is_still_reachable() {
        // A second display to the left makes the virtual desktop start negative;
        // clamping to zero there would forbid the whole monitor.
        let desktop = (-1920, 0, 1920, 1080);
        let size = Size { w: 268, h: 44 };
        assert_eq!(clamp_within(desktop, -1800, 500, size), (-1800, 500));
    }

    use super::*;

    #[test]
    fn glide_opens_at_the_speed_the_drag_ended_at() {
        // The curve's opening speed is slope * distance / duration. Solving for
        // the duration should reproduce the release speed within the clamp.
        let slope = motion::MOMENTUM.initial_slope();
        for (distance, speed) in [(400.0, 2.0), (250.0, 1.2), (900.0, 3.0)] {
            let dur = glide_duration(distance, speed).as_millis() as f64;
            let opening = slope * distance / dur;
            // Only meaningful where the ideal duration is inside the clamp.
            let ideal = slope * distance / speed;
            if ideal > GLIDE_MIN_MS && ideal < GLIDE_MAX_MS {
                assert!(
                    (opening - speed).abs() < 0.1,
                    "distance {distance}, speed {speed}: glide opens at {opening}"
                );
            }
        }
    }

    #[test]
    fn glide_duration_stays_within_bounds() {
        // Absurd inputs must not produce an instant jump or a crawl.
        for (distance, speed) in
            [(10.0, 50.0), (5000.0, 0.001), (0.0, 0.0), (1200.0, 0.6)]
        {
            let ms = glide_duration(distance, speed).as_millis() as f64;
            assert!(
                (GLIDE_MIN_MS..=GLIDE_MAX_MS).contains(&ms),
                "distance {distance}, speed {speed} produced {ms}ms"
            );
        }
    }

    // --- placement across DPI scales and monitor origins ---------------------
    //
    // This is the half of multi-monitor support that can be tested without a
    // second monitor, and it is the half where the bugs are. The failure mode is
    // always the same shape: code that quietly assumes the work area starts at
    // (0, 0) and that one logical pixel is one physical pixel. Both hold on a
    // single 100% display and neither holds anywhere else, so a single-monitor
    // machine cannot catch it by running the app.
    //
    // The remaining, untestable part is one OS call — whether `GetDpiForMonitor`
    // reports the destination monitor's scale — and it is thin glue by design.

    fn dock_at(left: i32, top: i32, w: i32, h: i32, scale: f64) -> taskbar::Dock {
        use windows::Win32::Foundation::RECT;
        let work = RECT { left, top, right: left + w, bottom: top + h };
        taskbar::Dock { work, monitor: work, taskbar: None, scale }
    }

    /// Resolve a mode to a window origin, the way `reposition` does.
    fn origin_for(mode: DockMode, dock: &taskbar::Dock, size: Size) -> (i32, i32) {
        let conf = Config { dock: mode, ..Config::default() };
        // Deliberately the production function, not a copy of its arithmetic:
        // a test that reimplements what it is checking only ever proves itself.
        let (gap, margin) = scaled_insets(&conf, dock.scale);
        anchor_for(&conf, dock, gap, margin).origin(size)
    }

    #[test]
    fn every_dock_lands_on_its_edge_at_100_percent() {
        let dock = dock_at(0, 0, 1920, 1032, 1.0);
        let size = collapsed_size(1.0);
        let (gap, margin) = (10, 16);

        assert_eq!(origin_for(DockMode::TopLeft, &dock, size), (margin, gap));
        assert_eq!(
            origin_for(DockMode::TopRight, &dock, size),
            (1920 - margin - size.w, gap)
        );
        assert_eq!(
            origin_for(DockMode::BottomLeft, &dock, size),
            (margin, 1032 - gap - size.h)
        );
        assert_eq!(
            origin_for(DockMode::BottomRight, &dock, size),
            (1920 - margin - size.w, 1032 - gap - size.h)
        );
        assert_eq!(
            origin_for(DockMode::TaskbarCenter, &dock, size).0,
            960 - size.w / 2
        );
    }

    /// A second monitor does not start at zero, and one to the *left* of the
    /// primary has negative coordinates. Anything that treats the work area as
    /// an origin rather than an offset lands the capsule on the wrong screen.
    #[test]
    fn placement_follows_a_monitor_that_is_not_at_the_origin() {
        for (left, top) in [(1920, 0), (-1920, 0), (1920, -540), (-2560, 120)] {
            let dock = dock_at(left, top, 1920, 1032, 1.0);
            let size = collapsed_size(1.0);

            assert_eq!(
                origin_for(DockMode::TopLeft, &dock, size),
                (left + 16, top + 10),
                "top-left on a monitor at ({left},{top})"
            );
            assert_eq!(
                origin_for(DockMode::BottomRight, &dock, size),
                (left + 1920 - 16 - size.w, top + 1032 - 10 - size.h),
                "bottom-right on a monitor at ({left},{top})"
            );
        }
    }

    /// Gaps and margins are configured in *logical* pixels, so the visual inset
    /// must be the same physical distance at every scale — which means scaling
    /// them by the destination monitor, not the one we happen to be on.
    #[test]
    fn gaps_and_margins_scale_with_the_monitor() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let dock = dock_at(0, 0, 1920, 1032, scale);
            let size = collapsed_size(scale);
            let gap = (10.0 * scale).round() as i32;
            let margin = (16.0 * scale).round() as i32;

            assert_eq!(
                origin_for(DockMode::TopLeft, &dock, size),
                (margin, gap),
                "top-left at {scale}x"
            );
            assert_eq!(
                origin_for(DockMode::BottomRight, &dock, size),
                (1920 - margin - size.w, 1032 - gap - size.h),
                "bottom-right at {scale}x"
            );
        }
    }

    /// The capsule itself is laid out in logical pixels and must grow with the
    /// display, or it renders postage-stamp sized on a 200% monitor.
    #[test]
    fn capsule_sizes_scale_with_the_monitor() {
        assert_eq!(collapsed_size(2.0).w, collapsed_size(1.0).w * 2);
        assert_eq!(expanded_size(2.0).h, expanded_size(1.0).h * 2);
        assert!(expanded_size(1.5).w > collapsed_size(1.5).w);
    }

    /// Mirroring is decided by which half of *its own* monitor the capsule sits
    /// in. Comparing against a bare screen width would mirror everything on a
    /// right-hand second monitor and nothing on a left-hand one.
    #[test]
    fn mirroring_is_relative_to_the_monitor_not_the_desktop() {
        let size = collapsed_size(1.0);
        for left in [0, 1920, -1920] {
            let dock = dock_at(left, 0, 1920, 1032, 1.0);

            let near_left = Anchor::at(left + 40, 0);
            let near_right = Anchor::at(left + 1920 - 40 - size.w, 0);
            assert!(!mirrored_for(near_left, size, &dock), "left edge at {left}");
            assert!(mirrored_for(near_right, size, &dock), "right edge at {left}");

            // The docked modes carry their alignment explicitly.
            let conf = Config { dock: DockMode::BottomRight, ..Config::default() };
            let anchor = anchor_for(&conf, &dock, 10, 16);
            assert!(mirrored_for(anchor, size, &dock), "bottom-right at {left}");
        }
    }

    #[test]
    fn a_still_release_gets_a_mid_range_glide() {
        // No momentum to match, so it should not be derived from the velocity.
        let ms = glide_duration(300.0, 0.0).as_millis() as f64;
        assert_eq!(ms, (GLIDE_MIN_MS + GLIDE_MAX_MS) / 2.0);
    }
}

/// Which way round the capsule's contents should be laid out.
///
/// The window shrinks and grows toward its *pinned* edge. Docked on the right
/// that edge is the right one, so with a left-to-right layout the artwork and
/// title slide sideways across the screen every time the capsule collapses,
/// while the pinned edge sits still — the collapse reads as the contents
/// running away rather than the capsule closing. Mirroring the layout puts the
/// contents against the edge that does not move, and the animation becomes the
/// same clean one it is on the left.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub mirrored: bool,
}

fn mirrored_for(anchor: Anchor, size: Size, dock: &taskbar::Dock) -> bool {
    match anchor.h {
        HAlign::Right => true,
        HAlign::Center => false,
        // A free placement carries no alignment — `Anchor::at` is always
        // left-aligned — so the only honest answer is where it actually sits.
        HAlign::Left => {
            let (x, _) = anchor.origin(size);
            x + size.w / 2 > dock.work.left + dock.work_width() / 2
        }
    }
}

/// Every mode a drop can snap to, in a fixed order.
const SNAP_MODES: [DockMode; 5] = [
    DockMode::TaskbarCenter,
    DockMode::BottomLeft,
    DockMode::BottomRight,
    DockMode::TopLeft,
    DockMode::TopRight,
];

/// Just past the right edge of the virtual desktop. The window stays composited
/// — so the backdrop stays warm — but nothing can see it.
fn park_anchor() -> Anchor {
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) }
        + 512;
    Anchor { x, y: 0, h: HAlign::Left, v: VAlign::Top }
}

fn collapsed_size(scale: f64) -> Size {
    Size {
        w: (logical::COLLAPSED_W * scale).round() as i32,
        h: (logical::COLLAPSED_H * scale).round() as i32,
    }
}

fn expanded_size(scale: f64) -> Size {
    Size {
        w: (logical::EXPANDED_W * scale).round() as i32,
        h: (logical::EXPANDED_H * scale).round() as i32,
    }
}

fn seed_size(scale: f64) -> Size {
    Size {
        w: (logical::SEED_W * scale).round() as i32,
        h: (logical::COLLAPSED_H * scale).round() as i32,
    }
}
