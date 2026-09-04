//! Where the island should sit.
//!
//! We dock against the monitor's *work area* rather than against the taskbar
//! rectangle, because the work area already accounts for the taskbar on any
//! edge, for auto-hide, and for third-party appbars. The raw taskbar rect is
//! still exposed — Phase 2 needs it to decide whether the wheel is over it.

use windows::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
        MONITORINFO, MonitorFromPoint,
    },
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        Shell::{ABM_GETSTATE, ABM_GETTASKBARPOS, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage},
        WindowsAndMessaging::GetCursorPos,
    },
};

use crate::config::MonitorPick;

#[derive(Debug, Clone, Copy)]
pub struct Dock {
    /// Usable area of the chosen monitor, in physical pixels.
    pub work: RECT,
    /// Full monitor bounds, in physical pixels.
    pub monitor: RECT,
    /// The primary taskbar's rectangle, when the shell reports one.
    pub taskbar: Option<RECT>,
    /// The taskbar is set to hide itself.
    ///
    /// This changes where the capsule may sit. With auto-hide on, the work
    /// area is the *whole* screen — Windows does not reserve anything for a bar
    /// that is not there — so docking against the work area puts the capsule
    /// exactly where the taskbar appears when it slides back out, and the
    /// capsule (topmost, and re-asserted every frame) then covers it.
    pub autohide: bool,
    /// DPI scale of *this* monitor (1.0 = 96 dpi, 1.5 = 150%).
    ///
    /// Read per-lookup rather than cached at startup: with mixed-DPI displays
    /// the island must be sized for the monitor it is about to move to, not the
    /// one it happens to be on. Caching this is what made the capsule render at
    /// the wrong size after moving to a scaled display.
    pub scale: f64,
}

impl Dock {
    pub fn work_width(&self) -> i32 {
        self.work.right - self.work.left
    }
}

/// Resolve the monitor to dock on. Re-evaluated on every reveal so that
/// `monitor: "cursor"` follows the user between displays.
pub fn dock_for(pick: MonitorPick) -> Dock {
    let point = match pick {
        MonitorPick::Primary => POINT { x: 0, y: 0 },
        MonitorPick::Cursor => {
            let mut p = POINT::default();
            // A failed GetCursorPos (locked session, secure desktop) degrades to
            // the primary monitor rather than to an off-screen origin.
            if unsafe { GetCursorPos(&mut p) }.is_ok() { p } else { POINT { x: 0, y: 0 } }
        }
    };

    let flags = match pick {
        MonitorPick::Primary => MONITOR_DEFAULTTOPRIMARY,
        MonitorPick::Cursor => MONITOR_DEFAULTTONEAREST,
    };

    let hmon: HMONITOR = unsafe { MonitorFromPoint(point, flags) };
    let (work, monitor) = monitor_rects(hmon);

    Dock {
        work,
        monitor,
        taskbar: taskbar_rect(),
        autohide: taskbar_autohides(),
        scale: monitor_scale(hmon),
    }
}

/// Effective DPI scale of a monitor.
///
/// `MDT_EFFECTIVE_DPI` is the one that matches what the user chose in Display
/// Settings (including the per-monitor overrides), which is what the island has
/// to match to look the right size. Falls back to 1.0 rather than guessing.
fn monitor_scale(hmon: HMONITOR) -> f64 {
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    match unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
        Ok(()) if dpi_x > 0 => dpi_x as f64 / 96.0,
        _ => 1.0,
    }
}

fn monitor_rects(hmon: HMONITOR) -> (RECT, RECT) {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        (info.rcWork, info.rcMonitor)
    } else {
        // Last-resort fallback: a 1080p origin. Better a visible island in the
        // wrong place than an invisible one at (0,0,0,0).
        let r = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        (r, r)
    }
}

/// Whether the shell's taskbar is in auto-hide mode.
///
/// `ABM_GETSTATE` is the shell's own answer, which is the only one that stays
/// right when the setting is changed while running — inferring it from the work
/// area cannot tell auto-hide apart from a taskbar on another monitor.
pub fn taskbar_autohides() -> bool {
    let mut data = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, ..Default::default() };
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) };
    state as u32 & ABS_AUTOHIDE != 0
}

/// How much room to leave for a taskbar that is currently hidden.
///
/// Zero when the taskbar is docked normally: the work area has already
/// accounted for it. When it auto-hides, the work area has not, so this is the
/// thickness of the bar on the edge the capsule docks against — reserved
/// permanently rather than watched for, because a capsule that jumps out of the
/// way every time the bar slides out would be worse than one that sits above
/// where it will appear.
pub fn hidden_taskbar_inset(dock: &Dock) -> i32 {
    if !dock.autohide {
        return 0;
    }
    let Some(bar) = dock.taskbar else { return 0 };

    let height = bar.bottom - bar.top;
    let width = bar.right - bar.left;
    // The short side is the thickness, whichever edge it is docked to.
    let thickness = height.min(width).max(0);

    // A hidden bar reports the rectangle it occupies when shown, but some
    // shells report the two-pixel reveal strip instead. Anything under a
    // plausible bar height means the real one is not being described.
    if thickness < 24 { 40 } else { thickness }
}

fn taskbar_rect() -> Option<RECT> {
    let mut data = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        ..Default::default()
    };
    // Returns 0 on failure; the shell may legitimately have no appbar.
    if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } == 0 {
        None
    } else {
        Some(data.rc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dock(taskbar: Option<RECT>, autohide: bool) -> Dock {
        let screen = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        Dock { work: screen, monitor: screen, taskbar, autohide, scale: 1.0 }
    }

    #[test]
    fn a_normal_taskbar_needs_no_inset() {
        // The work area already excludes it, so reserving more would leave a
        // gap twice the size of the bar.
        let bar = RECT { left: 0, top: 1032, right: 1920, bottom: 1080 };
        assert_eq!(hidden_taskbar_inset(&dock(Some(bar), false)), 0);
    }

    #[test]
    fn an_auto_hiding_taskbar_reserves_its_own_thickness() {
        let bar = RECT { left: 0, top: 1032, right: 1920, bottom: 1080 };
        assert_eq!(hidden_taskbar_inset(&dock(Some(bar), true)), 48);
    }

    #[test]
    fn a_vertical_taskbar_reports_its_short_side() {
        // Docked left: the thickness is the width, not the height.
        let bar = RECT { left: 0, top: 0, right: 62, bottom: 1080 };
        assert_eq!(hidden_taskbar_inset(&dock(Some(bar), true)), 62);
    }

    #[test]
    fn a_reveal_strip_is_not_a_taskbar_height() {
        // Some shells report the two-pixel hover strip while hidden. Docking
        // against that would put the capsule under the bar when it slides out.
        let strip = RECT { left: 0, top: 1078, right: 1920, bottom: 1080 };
        assert_eq!(hidden_taskbar_inset(&dock(Some(strip), true)), 40);
    }

    #[test]
    fn no_taskbar_at_all_needs_no_inset() {
        assert_eq!(hidden_taskbar_inset(&dock(None, true)), 0);
    }
}
