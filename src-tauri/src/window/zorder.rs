//! Where the capsule sits in the window stack, and when that should change.
//!
//! Three modes, from `config::OnTop`:
//!
//! - `always` — topmost, including over full-screen games. The default.
//! - `games` — topmost, except while a full-screen or borderless game owns the
//!   foreground.
//! - `never` — an ordinary window.
//!
//! `games` is the one that needs work: it has to know when the foreground
//! changes. That is a `SetWinEventHook` on `EVENT_SYSTEM_FOREGROUND` rather
//! than a timer, so a machine that never switches windows never wakes this up —
//! the same rule the rest of Lumen follows.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook},
        WindowsAndMessaging::{
            EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetWindowRect, WINEVENT_OUTOFCONTEXT,
            WINEVENT_SKIPOWNPROCESS,
        },
    },
};

use crate::config::OnTop;

/// What the foreground hook reports into. A process has one capsule, so this is
/// a single global rather than something threaded through the callback.
static WATCHER: OnceLock<Arc<Watcher>> = OnceLock::new();

pub struct Watcher {
    island: Arc<super::Island>,
    /// The mode in force. Read by the hook callback on every foreground change.
    mode: Arc<std::sync::RwLock<OnTop>>,
    /// Whether a full-screen window currently owns the foreground.
    covered: AtomicBool,
}

impl Watcher {
    /// Apply `mode` now, and keep applying it as the foreground changes.
    pub fn install(island: Arc<super::Island>, mode: OnTop) {
        let watcher = WATCHER.get_or_init(|| {
            let watcher = Arc::new(Watcher {
                island,
                mode: Arc::new(std::sync::RwLock::new(mode)),
                covered: AtomicBool::new(false),
            });

            // Out-of-context so the callback runs on our own thread through the
            // message queue, and skip-own-process because our windows coming to
            // the foreground are never what this is asking about.
            unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(on_foreground),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            watcher
        });

        if let Ok(mut current) = watcher.mode.write() {
            *current = mode;
        }
        watcher.refresh();
    }

    /// Re-decide the band from the mode and what is in front right now.
    fn refresh(&self) {
        let mode = self.mode.read().map(|m| *m).unwrap_or_default();
        let covered = self.covered.load(Ordering::SeqCst);

        let topmost = match mode {
            OnTop::Always => true,
            OnTop::Never => false,
            // Only stands down while something is genuinely covering the
            // screen. A maximised window is not a full-screen one: it leaves
            // the taskbar visible, and so should the capsule.
            OnTop::Games => !covered,
        };
        self.island.set_topmost(topmost);
    }
}

/// Called by Windows whenever a different window takes the foreground.
unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let Some(watcher) = WATCHER.get() else { return };
    // Read the foreground window rather than trusting the event's handle: the
    // event fires for menus and tooltips too, and the question is only ever
    // "what is in front now".
    let covered = foreground_is_fullscreen();
    if watcher.covered.swap(covered, Ordering::SeqCst) != covered {
        tracing::debug!("foreground full-screen: {covered}");
        watcher.refresh();
    }
}

/// Whether the foreground window covers its whole monitor.
///
/// This is the borderless-windowed case as much as the exclusive one: a game
/// running borderless is an ordinary window whose rectangle happens to be the
/// screen, and it is the common case on modern titles. Comparing rectangles
/// catches both, and — unlike `SHQueryUserNotificationState` — says *which*
/// monitor, which matters on a multi-head desk where the game is on one screen
/// and the capsule on another.
fn foreground_is_fullscreen() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }

        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }

        covers(rect, info.rcMonitor)
    }
}

/// Whether `window` covers `monitor`, within a pixel of slack per edge.
///
/// The slack matters: some titles report a rectangle one pixel larger than the
/// display, and an exact comparison would call those windowed.
fn covers(window: RECT, monitor: RECT) -> bool {
    window.left <= monitor.left + 1
        && window.top <= monitor.top + 1
        && window.right >= monitor.right - 1
        && window.bottom >= monitor.bottom - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn a_window_filling_the_display_counts_as_full_screen() {
        let monitor = rect(0, 0, 1920, 1080);
        assert!(covers(rect(0, 0, 1920, 1080), monitor));
        // Borderless titles frequently overhang by a pixel.
        assert!(covers(rect(-1, -1, 1921, 1081), monitor));
    }

    #[test]
    fn a_maximised_window_does_not() {
        // Maximised leaves the taskbar visible, so the capsule should stay
        // above it — that is the whole point of a capsule above the taskbar.
        let monitor = rect(0, 0, 1920, 1080);
        assert!(!covers(rect(0, 0, 1920, 1032), monitor));
    }

    #[test]
    fn a_window_on_another_monitor_is_judged_against_that_monitor() {
        let second = rect(1920, 0, 3840, 1080);
        assert!(covers(rect(1920, 0, 3840, 1080), second));
        assert!(!covers(rect(1920, 0, 2500, 700), second));
    }

    #[test]
    fn an_ordinary_window_is_not_full_screen() {
        let monitor = rect(0, 0, 1920, 1080);
        assert!(!covers(rect(300, 200, 1200, 800), monitor));
    }
}
