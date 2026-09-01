//! Global low-level mouse hook (`WH_MOUSE_LL`).
//!
//! # The hard constraint
//!
//! The callback runs on the thread that installed the hook and sits on the
//! *system-wide* input path — every mouse event on the machine passes through
//! it. Windows enforces `HKCU\Control Panel\Desktop\LowLevelHooksTimeout`
//! (300 ms by default) and **silently removes the hook** if a callback overruns
//! it. There is no error, no event; the feature simply stops working forever and
//! the app has no way to know.
//!
//! So the callback obeys three rules:
//!
//! 1. `WM_MOUSEMOVE` returns before doing anything at all. It is by far the
//!    highest-frequency message and is never interesting to us.
//! 2. No COM, no cross-process `SendMessage`, no blocking, no allocation. Only
//!    a handful of cheap local user32 calls.
//! 3. Anything real — changing volume, moving a window, quitting — is pushed
//!    onto a channel and performed by a worker thread.
//!
//! # Why class names rather than a cached HWND set
//!
//! Taskbar windows are recreated whenever Explorer restarts, so a cached handle
//! goes stale silently. `GetClassNameW` on the root window is a local call with
//! a stack buffer — a few microseconds — and is always correct, which is the
//! better trade inside a callback that must never be wrong *or* slow.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering},
    mpsc::{self, Sender},
};

use anyhow::Context;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint,
    },
    System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount},
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, GetCapture, VK_CONTROL, VK_MENU, VK_SHIFT},
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GA_PARENT, GA_ROOT, GetAncestor, GetClassNameW, GetMessageW,
            HHOOK, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW,
            TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL, WINDOWS_HOOK_ID, WM_MBUTTONDOWN,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WindowFromPoint,
        },
    },
};

/// What the hook asks the rest of the app to do. Deliberately tiny and `Copy`:
/// these are produced inside the callback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseAction {
    /// A wheel notch over the taskbar.
    ///
    /// Carries the screen point rather than a resolved target: working out which
    /// application that point belongs to needs UI Automation, which is far too
    /// slow for the callback. `covered` is true when the taskbar was not the
    /// window under the point — a full-screen window is drawing over the bar —
    /// which changes how the target is found.
    Volume { delta: f32, x: i32, y: i32, covered: bool },
    /// Middle-clicked the capsule: put it away.
    HideIsland,
    /// Alt + middle-clicked the capsule: quit.
    QuitApp,
    /// Clicked a taskbar button with the configured close button. Carries the
    /// screen point because resolving which application that button belongs to
    /// needs UI Automation, which is far too slow for the callback.
    CloseTaskbarApp(i32, i32),
}

/// Which button closes a taskbar app. Mirrors `config::TaskbarCloseButton`;
/// kept as a plain integer so it can live in an atomic the callback can read.
pub const CLOSE_NONE: isize = 0;
pub const CLOSE_MIDDLE: isize = 1;
pub const CLOSE_RIGHT: isize = 2;

/// Shared with the callback, which is a plain `extern "system" fn` and so can
/// only reach state through a static.
struct HookState {
    tx: Sender<MouseAction>,
    /// The island's window, so middle-click can tell "on the capsule" from
    /// "anywhere else on the desktop".
    island: AtomicIsize,
    /// Base volume step in scalar units, mirroring the config.
    step: AtomicIsize,
    taskbar_wheel: AtomicBool,
    /// Also act on the strip the shell reserves for the taskbar when a
    /// full-screen window is covering it.
    taskbar_wheel_covered: AtomicBool,
    middle_click: AtomicBool,
    alt_middle_quit: AtomicBool,
    /// One of the `CLOSE_*` constants.
    taskbar_close: AtomicIsize,
    /// Leftover wheel movement below one notch, carried between events.
    wheel_accum: AtomicI32,
    /// `GetTickCount` of the last wheel event, to expire a stale remainder.
    last_wheel_tick: AtomicU32,
}

static STATE: OnceLock<HookState> = OnceLock::new();

/// One classic wheel click. High-resolution devices send fractions of this.
const WHEEL_DELTA: i32 = 120;

/// Drop a carried remainder after this long — it belongs to an old gesture.
const WHEEL_ACCUM_RESET_MS: u32 = 5_000;

/// Step is stored as an integer of hundredths of a percent so it can live in an
/// atomic; f32 has no stable atomic on stable Rust.
const STEP_SCALE: f64 = 100_000.0;

/// Everything the hook's behaviour depends on, as one value.
///
/// Passed as a struct rather than as positional parameters because four of these
/// are adjacent `bool`s: transposing two of them compiles cleanly and silently
/// changes what the mouse does, with no way to notice short of using it.
#[derive(Debug, Clone, Copy)]
pub struct MouseSettings {
    /// Base volume step in scalar units; 0.02 is one Windows notch.
    pub volume_step: f32,
    pub taskbar_wheel: bool,
    /// Act on the taskbar's reserved band even when a window covers it.
    pub taskbar_wheel_covered: bool,
    pub middle_click: bool,
    pub alt_middle_quit: bool,
    /// One of the `CLOSE_*` constants.
    pub taskbar_close: isize,
}

pub struct MouseHook {
    thread_id: u32,
}

impl MouseHook {
    /// Install the hook on its own thread and start dispatching actions.
    pub fn start(
        island_hwnd: isize,
        settings: MouseSettings,
        mut on_action: impl FnMut(MouseAction) + Send + 'static,
    ) -> anyhow::Result<Arc<Self>> {
        let (tx, rx) = mpsc::channel::<MouseAction>();

        let state = HookState {
            tx,
            island: AtomicIsize::new(island_hwnd),
            step: AtomicIsize::new((settings.volume_step as f64 * STEP_SCALE) as isize),
            taskbar_wheel: AtomicBool::new(settings.taskbar_wheel),
            taskbar_wheel_covered: AtomicBool::new(settings.taskbar_wheel_covered),
            middle_click: AtomicBool::new(settings.middle_click),
            alt_middle_quit: AtomicBool::new(settings.alt_middle_quit),
            taskbar_close: AtomicIsize::new(settings.taskbar_close),
            wheel_accum: AtomicI32::new(0),
            last_wheel_tick: AtomicU32::new(0),
        };
        if STATE.set(state).is_err() {
            anyhow::bail!("the mouse hook is already installed");
        }

        // Consumer: everything expensive happens here, never in the callback.
        //
        // COM is initialised for the thread's lifetime because resolving a
        // taskbar button to an application needs UI Automation. That is exactly
        // the kind of work the callback must never do — it can take tens of
        // milliseconds, against a ~300 ms budget shared with every mouse event
        // on the system.
        std::thread::Builder::new()
            .name("lumen-mouse-actions".into())
            .spawn(move || {
                let com = unsafe {
                    windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                    )
                };
                if com.is_err() {
                    tracing::warn!("COM unavailable on the mouse action thread; \
                                    taskbar close will not work");
                }

                while let Ok(action) = rx.recv() {
                    on_action(action);
                }

                if com.is_ok() {
                    unsafe { windows::Win32::System::Com::CoUninitialize() };
                }
            })
            .context("failed to spawn the mouse action thread")?;

        let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<u32>>();

        std::thread::Builder::new()
            .name("lumen-mouse-hook".into())
            .spawn(move || {
                // SAFETY: a module handle for the current process, and a static
                // callback that outlives the hook.
                let hook = unsafe {
                    let module = GetModuleHandleW(None).ok();
                    SetWindowsHookExW(
                        WINDOWS_HOOK_ID(WH_MOUSE_LL.0),
                        Some(low_level_proc),
                        module.map(Into::into),
                        0,
                    )
                };

                let hook: HHOOK = match hook {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!("SetWindowsHookExW: {e}")));
                        return;
                    }
                };

                let thread_id = unsafe {
                    windows::Win32::System::Threading::GetCurrentThreadId()
                };
                let _ = ready_tx.send(Ok(thread_id));

                // A low-level hook is delivered through this thread's message
                // queue, so the pump is mandatory — without it the callback
                // never fires. `GetMessageW` blocks, so this thread costs
                // nothing while the mouse is still.
                let mut msg = MSG::default();
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                }
            })
            .context("failed to spawn the mouse hook thread")?;

        let thread_id = ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .context("mouse hook thread did not report readiness")??;

        tracing::info!("WH_MOUSE_LL installed on thread {thread_id}");
        Ok(Arc::new(Self { thread_id }))
    }

    /// Keep the hook's copy of the settings in step with the config.
    pub fn update(&self, settings: MouseSettings) {
        let Some(state) = STATE.get() else { return };
        state.step.store((settings.volume_step as f64 * STEP_SCALE) as isize, Ordering::Relaxed);
        state.taskbar_wheel.store(settings.taskbar_wheel, Ordering::Relaxed);
        state.taskbar_wheel_covered.store(settings.taskbar_wheel_covered, Ordering::Relaxed);
        state.middle_click.store(settings.middle_click, Ordering::Relaxed);
        state.alt_middle_quit.store(settings.alt_middle_quit, Ordering::Relaxed);
        state.taskbar_close.store(settings.taskbar_close, Ordering::Relaxed);
    }

    pub fn set_island(&self, hwnd: isize) {
        if let Some(state) = STATE.get() {
            state.island.store(hwnd, Ordering::Relaxed);
        }
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        // Ends the message pump, which unhooks on the way out.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// True while the named virtual key is held.
///
/// `GetAsyncKeyState` reads a cached keyboard state — no message round-trip, no
/// blocking. Safe to call from the callback.
#[inline]
fn key_down(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

/// Root window under a screen point, or null.
#[inline]
fn root_window_at(pt: POINT) -> HWND {
    let hwnd = unsafe { WindowFromPoint(pt) };
    if hwnd.is_invalid() {
        return hwnd;
    }
    unsafe { GetAncestor(hwnd, GA_ROOT) }
}

/// Whether this exact window is one of the shell's taskbars.
#[inline]
fn class_is_taskbar(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    let name = String::from_utf16_lossy(&buf[..len as usize]);
    name == "Shell_TrayWnd" || name == "Shell_SecondaryTrayWnd"
}

/// Whether a screen point lands on a taskbar.
///
/// Walks the ancestor chain rather than trusting `GA_ROOT`. On Windows 11 the
/// taskbar's buttons live inside XAML islands
/// (`Windows.UI.Composition.DesktopWindowContentBridge` →
/// `Windows.UI.Input.InputSite.WindowClass`) which the shell creates in their
/// own band, so `GA_ROOT` from a point over a *button* returns the island, not
/// `Shell_TrayWnd`. Empty stretches of the bar do resolve directly, which is why
/// scrolling worked while clicking a button did nothing.
///
/// Still cheap: a handful of local calls, and only for wheel and button events —
/// never for mouse movement.
#[inline]
fn is_taskbar_at(pt: POINT) -> bool {
    let mut hwnd = unsafe { WindowFromPoint(pt) };
    // Bounded so a malformed chain cannot spin inside the hook callback.
    for _ in 0..8 {
        if hwnd.is_invalid() {
            return false;
        }
        if class_is_taskbar(hwnd) {
            return true;
        }
        hwnd = unsafe { GetAncestor(hwnd, GA_PARENT) };
    }
    false
}

/// Whether a point lies in the band the shell reserves for the taskbar.
///
/// This is the answer to "the taskbar is *there*, but something is drawn over
/// it". A borderless full-screen game covers the bar completely, so
/// `is_taskbar_at` correctly says no while the user is still pointing at exactly
/// the place they scroll to change volume — which is the case that matters most,
/// because a game is precisely when reaching for a mixer is least welcome.
///
/// The monitor's work area is the monitor minus whatever appbars have reserved,
/// so the difference between the two rectangles *is* the taskbar's band. That
/// makes this correct for any edge, any monitor, and any taskbar thickness, using
/// two cheap local calls and no window handles that could go stale.
///
/// An auto-hiding taskbar reserves nothing, so the band is empty and this is
/// always false — which is right: there is no bar there to scroll on.
#[inline]
fn in_reserved_strip(pt: POINT) -> bool {
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONULL) };
    if monitor.is_invalid() {
        return false;
    }

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return false;
    }

    let work = info.rcWork;
    let inside_work =
        pt.x >= work.left && pt.x < work.right && pt.y >= work.top && pt.y < work.bottom;
    // `MONITOR_DEFAULTTONULL` already guaranteed the point is on this monitor.
    !inside_work
}

/// The hook callback. Everything here is on the system-wide input path.
unsafe extern "system" fn low_level_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Negative codes must be passed straight through, untouched.
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let message = wparam.0 as u32;

    // Rule 1: the common case leaves immediately, before any other work.
    if message == WM_MOUSEMOVE {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let Some(state) = STATE.get() else {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };

    let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };

    match message {
        WM_MOUSEWHEEL if state.taskbar_wheel.load(Ordering::Relaxed) => {
            // A drag is in progress somewhere (a scrollbar, a slider). Stealing
            // the wheel mid-capture would interfere with whatever owns it.
            if !unsafe { GetCapture() }.is_invalid() {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            }

            // Either the bar itself is under the pointer, or something is
            // covering the band it occupies.
            let on_bar = is_taskbar_at(info.pt);
            let covered = !on_bar
                && state.taskbar_wheel_covered.load(Ordering::Relaxed)
                && in_reserved_strip(info.pt);
            if !on_bar && !covered {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            }

            // High word of mouseData is the signed wheel delta.
            let delta = ((info.mouseData >> 16) & 0xFFFF) as i16 as i32;
            if delta == 0 {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            }

            // Accumulate sub-notch movement.
            //
            // A classic wheel sends WHEEL_DELTA (120) per click, but
            // high-resolution wheels and precision touchpads send much smaller
            // increments — often 8 or 16 at a time. Treating each of those as a
            // full notch makes the volume rocket from 0 to 100 on a light
            // flick. Only whole notches are acted on; the remainder is carried.
            let now = unsafe { GetTickCount() };
            let last = state.last_wheel_tick.swap(now, Ordering::Relaxed);
            // A long pause means a new gesture; a stale remainder from minutes
            // ago must not tip the next scroll over the line.
            let carried = if now.wrapping_sub(last) < WHEEL_ACCUM_RESET_MS {
                state.wheel_accum.load(Ordering::Relaxed)
            } else {
                0
            };

            let total = carried + delta;
            let notches = total / WHEEL_DELTA;
            state.wheel_accum.store(total % WHEEL_DELTA, Ordering::Relaxed);

            if notches == 0 {
                // Swallowed anyway: the movement was consumed into the
                // accumulator, and letting it through would scroll the taskbar.
                return LRESULT(1);
            }

            let step = state.step.load(Ordering::Relaxed) as f64 / STEP_SCALE;
            // Same grain as the island wheel, so the two feel identical.
            let grain = if key_down(VK_CONTROL.0) {
                5.0
            } else if key_down(VK_SHIFT.0) {
                0.25
            } else {
                1.0
            };

            let _ = state.tx.send(MouseAction::Volume {
                delta: (step * grain * f64::from(notches)) as f32,
                x: info.pt.x,
                y: info.pt.y,
                covered,
            });

            // Swallow it: the taskbar must not also act on this scroll.
            LRESULT(1)
        }

        WM_MBUTTONDOWN => {
            let root = root_window_at(info.pt);
            let island = state.island.load(Ordering::Relaxed);

            // Our own capsule first: hide, or quit with Alt.
            if state.middle_click.load(Ordering::Relaxed)
                && island != 0
                && root.0 as isize == island
            {
                let action =
                    if key_down(VK_MENU.0) && state.alt_middle_quit.load(Ordering::Relaxed) {
                        MouseAction::QuitApp
                    } else {
                        MouseAction::HideIsland
                    };
                let _ = state.tx.send(action);
                return LRESULT(1);
            }

            // A taskbar button, if middle-click is the configured close button.
            if state.taskbar_close.load(Ordering::Relaxed) == CLOSE_MIDDLE && is_taskbar_at(info.pt) {
                let _ = state.tx.send(MouseAction::CloseTaskbarApp(info.pt.x, info.pt.y));
                return LRESULT(1);
            }

            // Anywhere else, middle-click belongs to whatever is under it.
            unsafe { CallNextHookEx(None, code, wparam, lparam) }
        }

        WM_RBUTTONDOWN if state.taskbar_close.load(Ordering::Relaxed) == CLOSE_RIGHT => {
            if !is_taskbar_at(info.pt) {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            }
            let _ = state.tx.send(MouseAction::CloseTaskbarApp(info.pt.x, info.pt.y));
            // Swallowed, which also suppresses the jump list — the documented
            // cost of choosing right-click for this.
            LRESULT(1)
        }

        _ => unsafe { CallNextHookEx(None, code, wparam, lparam) },
    }
}
