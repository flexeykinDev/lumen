//! Animates the island's window rectangle.
//!
//! # Why not a window region
//!
//! The first implementation kept the window fixed at its expanded size and
//! clipped it to a capsule with `SetWindowRgn`, so that WebView2 never had to
//! resize. That is wrong, and the reason is worth recording because it is not
//! obvious:
//!
//! **`SetWindowRgn` does not clip the DWM system backdrop.** Mica and Acrylic
//! are composited by DWM across the window's whole rectangle, outside the
//! region clip entirely. The region only ever clipped the WebView's own output.
//!
//! The bug hid behind Mica: `DWMSBT_MAINWINDOW` is near-opaque and tinted from
//! the wallpaper, so over a dark desktop the unclipped area was almost the same
//! colour as what it covered. Switching to translucent Acrylic made it plain —
//! a 428x148 grey slab sitting over the screen with the capsule drawn inside it.
//!
//! So the window rectangle must *be* the capsule. This animates it directly.
//! WebView2 does resize every frame as a result; measurements live in
//! ARCHITECTURE.md §4.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING, SetWindowPos,
    },
};

use crate::motion::{Curve, FRAME, ease, ease_with, lerp_i32};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Bottom,
}

/// Where the capsule is pinned, and which of its edges is pinned there.
///
/// The alignment matters because the capsule changes size as it expands: the
/// pinned edge is the one that must *not* move during a transition. Docked above
/// the taskbar that is the bottom edge (the island grows upward out of it);
/// docked in a top corner it is the top edge, and the capsule grows downward.
/// Storing a point plus alignment expresses every docking mode with one struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub x: i32,
    pub y: i32,
    pub h: HAlign,
    pub v: VAlign,
}

impl Anchor {
    /// A raw top-left placement, used while dragging and mid-glide where the
    /// concept of a pinned edge does not apply.
    pub fn at(x: i32, y: i32) -> Self {
        Self { x, y, h: HAlign::Left, v: VAlign::Top }
    }

    /// Resolve to a window origin for a capsule of the given size.
    pub fn origin(self, size: Size) -> (i32, i32) {
        let x = match self.h {
            HAlign::Left => self.x,
            HAlign::Center => self.x - size.w / 2,
            HAlign::Right => self.x - size.w,
        };
        let y = match self.v {
            VAlign::Top => self.y,
            VAlign::Bottom => self.y - size.h,
        };
        (x, y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub fn lerp(self, to: Size, t: f64) -> Size {
        Size { w: lerp_i32(self.w, to.w, t), h: lerp_i32(self.h, to.h, t) }
    }
}

pub struct GeometryAnimator {
    hwnd: isize,
    state: Mutex<(Anchor, Size)>,
    /// Bumped per transition so a superseded animator retires instead of
    /// fighting the newer one over `SetWindowPos`.
    generation: Arc<AtomicU64>,
}

impl GeometryAnimator {
    pub fn new(hwnd: isize, anchor: Anchor, size: Size) -> Self {
        let me = Self {
            hwnd,
            state: Mutex::new((anchor, size)),
            generation: Arc::new(AtomicU64::new(0)),
        };
        me.apply(anchor, size);
        me
    }

    pub fn size(&self) -> Size {
        self.state.lock().expect("geometry lock poisoned").1
    }

    pub fn anchor(&self) -> Anchor {
        self.state.lock().expect("geometry lock poisoned").0
    }

    /// Anchor and size together, read under one lock so they cannot describe
    /// two different moments mid-animation.
    pub fn placement(&self) -> (Anchor, Size) {
        let guard = self.state.lock().expect("geometry lock poisoned");
        (guard.0, guard.1)
    }

    /// Move the dock point without disturbing an in-flight size animation.
    pub fn set_anchor(&self, anchor: Anchor) {
        let size = {
            let mut guard = self.state.lock().expect("geometry lock poisoned");
            guard.0 = anchor;
            guard.1
        };
        self.apply(anchor, size);
    }

    pub fn snap_to(&self, size: Size) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let anchor = {
            let mut guard = self.state.lock().expect("geometry lock poisoned");
            guard.1 = size;
            guard.0
        };
        self.apply(anchor, size);
    }

    /// Animate to `to` over `dur` on the shared easing curve.
    pub fn animate_to(self: &Arc<Self>, to: Size, dur: Duration) {
        let from = self.size();
        if from == to {
            return;
        }
        if dur.is_zero() {
            self.snap_to(to);
            return;
        }

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let me = Arc::clone(self);

        let _ = std::thread::Builder::new().name("lumen-geometry".into()).spawn(move || {
            let start = Instant::now();
            loop {
                if me.generation.load(Ordering::SeqCst) != generation {
                    return;
                }

                let frame_start = Instant::now();
                let t = (start.elapsed().as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0);
                let size = from.lerp(to, ease(t));

                let anchor = {
                    let mut guard = me.state.lock().expect("geometry lock poisoned");
                    guard.1 = size;
                    guard.0
                };
                me.apply(anchor, size);

                if t >= 1.0 {
                    return;
                }
                if let Some(rest) = FRAME.checked_sub(frame_start.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
        });
    }

    /// Glide the capsule to a new anchor without changing its size.
    ///
    /// Interpolates the resolved *origin*, not the anchor itself: an anchor
    /// carries an alignment, and there is no meaningful halfway point between
    /// "pinned bottom-centre" and "pinned top-right". Start and target are both
    /// resolved to screen coordinates, the origin is tweened between them, and
    /// the real target anchor is installed on the final frame so subsequent
    /// resizes expand from the correct edge.
    pub fn animate_anchor_to(self: &Arc<Self>, target: Anchor, dur: Duration, curve: Curve) {
        let (from_anchor, size) = {
            let guard = self.state.lock().expect("geometry lock poisoned");
            (guard.0, guard.1)
        };

        let from = from_anchor.origin(size);
        let to = target.origin(size);
        if from == to {
            // Still install the anchor: the position matches but the alignment
            // may not, and that governs how the next expand behaves.
            self.set_anchor(target);
            return;
        }
        if dur.is_zero() {
            self.set_anchor(target);
            return;
        }

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let me = Arc::clone(self);

        let _ = std::thread::Builder::new().name("lumen-dock-glide".into()).spawn(move || {
            let start = Instant::now();
            loop {
                if me.generation.load(Ordering::SeqCst) != generation {
                    // Superseded — a resize started mid-glide, and it owns the
                    // window now. Still install the target *anchor* before
                    // leaving: the intermediate steps are raw left-aligned
                    // origins, so retiring on one leaves the capsule pinned by
                    // its left edge at a position computed for the size it had
                    // at that instant. On a right-hand dock the next collapse
                    // then shrinks away from the screen edge and strands the
                    // capsule with a gap beside it. Position is left to whoever
                    // superseded us; only the alignment is restored.
                    if let Ok(mut guard) = me.state.lock() {
                        guard.0 = target;
                    }
                    return;
                }

                let frame_start = Instant::now();
                let t = (start.elapsed().as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0);
                let eased = ease_with(curve, t);

                if t >= 1.0 {
                    // Land on the real anchor, alignment included.
                    let size = {
                        let mut guard = me.state.lock().expect("geometry lock poisoned");
                        guard.0 = target;
                        guard.1
                    };
                    me.apply(target, size);
                    return;
                }

                let x = lerp_i32(from.0, to.0, eased);
                let y = lerp_i32(from.1, to.1, eased);
                let step = Anchor::at(x, y);

                let size = {
                    let mut guard = me.state.lock().expect("geometry lock poisoned");
                    guard.0 = step;
                    guard.1
                };
                me.apply(step, size);

                if let Some(rest) = FRAME.checked_sub(frame_start.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
        });
    }

    fn apply(&self, anchor: Anchor, size: Size) {
        let size = Size { w: size.w.max(1), h: size.h.max(1) };
        let (w, h) = (size.w, size.h);
        let (x, y) = anchor.origin(size);

        unsafe {
            // NOACTIVATE so the island never steals focus mid-animation, and
            // TOPMOST re-asserted each frame so a newly-shown window cannot
            // slide over the capsule while it is expanding.
            let _ = SetWindowPos(
                HWND(self.hwnd as *mut _),
                Some(HWND_TOPMOST),
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
            );
        }
    }
}
