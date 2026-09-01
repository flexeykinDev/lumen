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
        Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage},
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

    Dock { work, monitor, taskbar: taskbar_rect(), scale: monitor_scale(hmon) }
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
