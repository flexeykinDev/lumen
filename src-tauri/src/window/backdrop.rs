//! Real DWM backdrops, with an honest fallback chain.
//!
//! Mica needs Windows 11 22H2 (build 22621). Acrylic via
//! `SetWindowCompositionAttribute` works back to Windows 10 1809. Below that we
//! tell the renderer it is on its own and it draws a flat translucent panel.
//!
//! The window is never hidden, only parked off-screen, precisely so that this
//! backdrop is applied once and stays warm — `show()` makes Mica visibly
//! re-bloom on every reveal. See ARCHITECTURE.md §4.

use tauri::WebviewWindow;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DWMWCP_ROUND, DwmSetWindowAttribute,
    },
};

use crate::config::{BackdropPref, Shape};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackdropKind {
    Mica,
    Acrylic,
    /// No system backdrop available; the renderer draws its own translucency.
    None,
}

/// Apply the best available backdrop and report what actually landed.
///
/// The renderer needs the answer: a CSS panel tuned for translucent Acrylic
/// looks muddy over nothing, so `BackdropKind` is forwarded to the frontend.
///
/// # Why `Auto` prefers Acrylic over Mica
///
/// These are not two flavours of the same effect:
///
/// - **Mica** (`DWMSBT_MAINWINDOW`) samples *only the desktop wallpaper*,
///   heavily blurred and near-opaque, and deliberately ignores windows behind
///   it. Microsoft designed it for large, long-lived app windows so they feel
///   rooted to the desktop. Over a maximised dark app it renders as flat dark
///   grey — correct behaviour, and completely wrong for a floating capsule.
/// - **Acrylic** (`DWMSBT_TRANSIENTWINDOW`) samples whatever is actually behind
///   the window. That is real see-through glass, and it is what Microsoft uses
///   for exactly this shape of UI: flyouts, the volume OSD, the taskbar jump
///   lists.
///
/// Lumen is a transient floating surface, so Acrylic is the right default.
/// `backdrop: "mica"` remains available for anyone who wants the flatter look.
pub fn apply(window: &WebviewWindow, pref: BackdropPref, dark: bool) -> BackdropKind {
    // Only consulted on Windows 10 / pre-22H2, where `apply_acrylic` falls back
    // to SetWindowCompositionAttribute. On 22H2+ DWM owns the tint and this is
    // ignored, so it is kept light to avoid double-darkening on older builds.
    const ACRYLIC_TINT: (u8, u8, u8, u8) = (18, 18, 24, 90);

    let try_mica = || window_vibrancy::apply_mica(window, Some(dark)).is_ok();
    let try_acrylic = || window_vibrancy::apply_acrylic(window, Some(ACRYLIC_TINT)).is_ok();

    let kind = match pref {
        BackdropPref::Mica => {
            if try_mica() {
                BackdropKind::Mica
            } else if try_acrylic() {
                BackdropKind::Acrylic
            } else {
                BackdropKind::None
            }
        }
        BackdropPref::Acrylic => {
            if try_acrylic() { BackdropKind::Acrylic } else { BackdropKind::None }
        }
        // Translucency first — see the note above.
        BackdropPref::Auto => {
            if try_acrylic() {
                BackdropKind::Acrylic
            } else if try_mica() {
                BackdropKind::Mica
            } else {
                BackdropKind::None
            }
        }
    };

    if kind == BackdropKind::None {
        tracing::warn!("no system backdrop available; falling back to CSS translucency");
    } else {
        tracing::info!("backdrop: {kind:?}");
    }
    kind
}

/// Keep DWM's own light/dark heuristics aligned with the app theme.
pub fn set_dark(hwnd: isize, dark: bool) {
    let value = windows::core::BOOL::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const value).cast(),
            size_of::<windows::core::BOOL>() as u32,
        );
    }
}

/// Shape the capsule.
///
/// The window rectangle *is* the capsule (see `geometry.rs`), so DWM's own
/// corner rounding is what shapes it — and unlike a window region, DWM's corners
/// are anti-aliased and it rounds the system backdrop along with everything else.
///
/// DWM offers three fixed radii and no custom value, so a true full-height pill
/// is not reachable this way; `DWMWCP_ROUND` (~8 px) is the closest, and matches
/// a stock Windows 11 flyout. A real capsule needs a Windows.UI.Composition
/// backdrop with a rounded-rectangle clip — see ARCHITECTURE.md §4.
pub fn set_corners(hwnd: isize, shape: Shape) {
    let pref = match shape {
        Shape::Round => DWMWCP_ROUND,
        Shape::Square => DWMWCP_DONOTROUND,
    };
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const pref).cast(),
            size_of::<i32>() as u32,
        );
    }
}
