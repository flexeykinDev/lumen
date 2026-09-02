//! Pause playback when the machine is locked, and pick it up again on return.
//!
//! # Why not `SHQueryUserNotificationState`
//!
//! That call answers "would a notification be unwelcome right now" — locked,
//! screen saver, full-screen game — and is the obvious primitive for this. But
//! it only answers when *asked*: there is no notification, so using it means a
//! timer, forever, for an event that happens a few times a day. Idle cost is a
//! hard constraint here (ARCHITECTURE.md §3), and a poll is the one thing that
//! guarantees the process never truly sleeps.
//!
//! Session lock and unlock, which is the case that actually matters, is
//! delivered as a *message* by `WTSRegisterSessionNotification`. So this is
//! event-driven and costs nothing between events, at the price of not knowing
//! about full-screen games. That trade is deliberate: a game is not a reason to
//! stop someone's music, but walking away from a locked machine is.
//!
//! # The rule that matters
//!
//! **Only ever resume what this module paused.** A user who paused their own
//! music before locking must not come back to it playing. So the pause is
//! recorded, and unlock resumes only if the session still looks exactly as it
//! was left: same source, still paused.

use std::sync::{Arc, OnceLock, mpsc::Sender};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, RemoteDesktop::*},
        // The session constants live under WindowsAndMessaging, not
        // RemoteDesktop beside the function that delivers them. Naming them
        // explicitly rather than relying on a glob is what makes a missing
        // import a compile error: as bare identifiers in a `match` they would
        // silently become catch-all bindings, and the first arm would swallow
        // every session event.
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            HWND_MESSAGE, MSG, PostQuitMessage, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE,
            WINDOW_STYLE, WM_DESTROY, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK,
            WTS_SESSION_UNLOCK,
        },
    },
    core::{PCWSTR, w},
};

use crate::media::{MediaBackend, NowPlaying, PlaybackState, TransportCmd};

/// What the window procedure reports. Kept tiny — it is produced inside a
/// message handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    Locked,
    Unlocked,
}

static EVENTS: OnceLock<Sender<Session>> = OnceLock::new();

/// What to do about a session event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Nothing,
    Pause,
    Resume,
}

/// The whole policy, as a function of what is known — no I/O, so every rule can
/// be tested directly.
///
/// Returns the action to take and what to remember: the id of the session this
/// module paused, or `None` if it has nothing outstanding.
///
/// The rule that matters is in the `Unlocked` arm. Resuming is only ever
/// undoing our own pause, so it requires the world to be exactly as it was
/// left. Anything else — a different app playing, the track changed, the user
/// having already pressed play — means this is no longer ours to undo, and
/// coming back to music you deliberately stopped is worse than coming back to
/// silence.
fn decide(
    event: Session,
    now: Option<&NowPlaying>,
    paused_by_us: Option<&str>,
    resume_on_unlock: bool,
) -> (Action, Option<String>) {
    match event {
        Session::Locked => match now {
            Some(np) if np.state == PlaybackState::Playing => {
                (Action::Pause, Some(np.session_id.clone()))
            }
            // Already quiet, so there is nothing of ours to undo later. Clearing
            // matters: a stale id would make the next unlock resume something
            // this module never paused.
            _ => (Action::Nothing, None),
        },
        Session::Unlocked => {
            let Some(session_id) = paused_by_us else { return (Action::Nothing, None) };
            if !resume_on_unlock {
                return (Action::Nothing, None);
            }
            match now {
                Some(np)
                    if np.session_id == session_id && np.state == PlaybackState::Paused =>
                {
                    (Action::Resume, None)
                }
                _ => (Action::Nothing, None),
            }
        }
    }
}

pub struct SmartPause {
    thread_id: u32,
}

impl SmartPause {
    /// Watch for lock and unlock, pausing and resuming `media` around them.
    pub fn start(media: Arc<dyn MediaBackend>, resume_on_unlock: bool) -> anyhow::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<Session>();
        if EVENTS.set(tx).is_err() {
            anyhow::bail!("smart pause is already running");
        }

        // Acting on the events, away from the window's message loop: a message
        // handler that blocks stops delivering everything else.
        std::thread::Builder::new().name("lumen-smart-pause".into()).spawn(move || {
            // Which source we paused, so a different one playing later is not
            // mistaken for ours.
            let mut paused_by_us: Option<String> = None;

            while let Ok(event) = rx.recv() {
                let now = media.snapshot();
                let (action, next) =
                    decide(event, now.as_ref(), paused_by_us.as_deref(), resume_on_unlock);
                paused_by_us = next;

                let verb = match action {
                    Action::Nothing => continue,
                    Action::Pause => "pausing",
                    Action::Resume => "resuming",
                };
                let source = now.as_ref().map(|n| n.source.as_str()).unwrap_or("playback");
                match media.control(TransportCmd::PlayPause) {
                    Ok(()) => tracing::info!("{verb} {source}"),
                    Err(e) => tracing::warn!("smart pause could not act: {e}"),
                }
            }
        })?;

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<u32>>();

        std::thread::Builder::new().name("lumen-session-watch".into()).spawn(move || {
            let hwnd = match create_listener() {
                Ok(h) => h,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };

            // SAFETY: the window outlives the registration; it is unregistered
            // on the way out below.
            let registered =
                unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
            if let Err(e) = registered {
                let _ = ready_tx.send(Err(anyhow::anyhow!(
                    "WTSRegisterSessionNotification failed: {e}"
                )));
                unsafe { let _ = DestroyWindow(hwnd); };
                return;
            }

            let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
            let _ = ready_tx.send(Ok(thread_id));

            // Session notifications arrive as window messages, so the pump is
            // mandatory. `GetMessageW` blocks, so this thread costs nothing
            // between events — which is the whole reason for this design.
            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let _ = WTSUnRegisterSessionNotification(hwnd);
                let _ = DestroyWindow(hwnd);
            }
        })?;

        let thread_id = ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| anyhow::anyhow!("session watcher did not report readiness"))??;

        tracing::info!("smart pause watching session lock on thread {thread_id}");
        Ok(Self { thread_id })
    }
}

impl Drop for SmartPause {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.thread_id,
                windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// A message-only window: never shown, never painted, exists purely to receive
/// `WM_WTSSESSION_CHANGE`.
fn create_listener() -> anyhow::Result<HWND> {
    const CLASS: PCWSTR = w!("LumenSessionWatch");

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // A zero return can also mean "already registered", which is fine on a
        // second start; CreateWindowExW is the real test.
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Lumen session watch"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )?;
        Ok(hwnd)
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_WTSSESSION_CHANGE => {
            let event = match wparam.0 as u32 {
                WTS_SESSION_LOCK => Some(Session::Locked),
                WTS_SESSION_UNLOCK => Some(Session::Unlocked),
                _ => None,
            };
            // Nothing but a channel send happens here. Pausing means a WinRT
            // round-trip, and a window procedure that blocks stops the session
            // notifications that follow it.
            if let Some(event) = event
                && let Some(tx) = EVENTS.get()
            {
                let _ = tx.send(event);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Timeline;

    fn track(session: &str, state: PlaybackState) -> NowPlaying {
        NowPlaying {
            session_id: session.into(),
            source: "Spotify".into(),
            title: "t".into(),
            artist: "a".into(),
            album: String::new(),
            state,
            timeline: Timeline::default(),
            art_data_uri: None,
            accent: None,
            revision: 1,
        }
    }

    #[test]
    fn locking_pauses_what_is_playing_and_remembers_it() {
        let np = track("spotify", PlaybackState::Playing);
        let (action, held) = decide(Session::Locked, Some(&np), None, true);
        assert_eq!(action, Action::Pause);
        assert_eq!(held.as_deref(), Some("spotify"));
    }

    #[test]
    fn locking_something_already_paused_does_nothing() {
        let np = track("spotify", PlaybackState::Paused);
        let (action, held) = decide(Session::Locked, Some(&np), None, true);
        assert_eq!(action, Action::Nothing);
        assert_eq!(held, None, "nothing was paused, so nothing is owed a resume");
    }

    #[test]
    fn locking_with_nothing_playing_does_nothing() {
        let (action, held) = decide(Session::Locked, None, None, true);
        assert_eq!(action, Action::Nothing);
        assert_eq!(held, None);
    }

    #[test]
    fn unlocking_resumes_the_session_we_paused() {
        let np = track("spotify", PlaybackState::Paused);
        let (action, held) = decide(Session::Unlocked, Some(&np), Some("spotify"), true);
        assert_eq!(action, Action::Resume);
        assert_eq!(held, None);
    }

    /// The rule this whole module is built around: never restart music the user
    /// stopped themselves.
    #[test]
    fn unlocking_never_resumes_a_pause_we_did_not_cause() {
        let np = track("spotify", PlaybackState::Paused);
        let (action, _) = decide(Session::Unlocked, Some(&np), None, true);
        assert_eq!(action, Action::Nothing);
    }

    /// While away, the user started something else. Resuming would now be
    /// pressing play on an app that was never paused by us.
    #[test]
    fn unlocking_does_not_resume_a_different_session() {
        let np = track("firefox", PlaybackState::Paused);
        let (action, _) = decide(Session::Unlocked, Some(&np), Some("spotify"), true);
        assert_eq!(action, Action::Nothing);
    }

    /// Already playing again — pressing play/pause here would *stop* it.
    #[test]
    fn unlocking_does_not_touch_playback_that_already_resumed() {
        let np = track("spotify", PlaybackState::Playing);
        let (action, _) = decide(Session::Unlocked, Some(&np), Some("spotify"), true);
        assert_eq!(action, Action::Nothing);
    }

    #[test]
    fn resume_can_be_switched_off() {
        let np = track("spotify", PlaybackState::Paused);
        let (action, _) = decide(Session::Unlocked, Some(&np), Some("spotify"), false);
        assert_eq!(action, Action::Nothing);
    }

    /// A lock that paused nothing must clear the memory, or the *next* unlock
    /// would resume a session this module never touched.
    #[test]
    fn a_lock_that_pauses_nothing_forgets_any_earlier_pause() {
        let idle = track("spotify", PlaybackState::Paused);
        let (_, held) = decide(Session::Locked, Some(&idle), Some("spotify"), true);
        assert_eq!(held, None);
    }
}
