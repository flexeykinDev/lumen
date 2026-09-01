//! Work out which application sits under a point on the taskbar.
//!
//! Two features need this: closing an application from its taskbar button, and
//! scrolling over that button to change *that application's* volume rather than
//! the system master.
//!
//! # Why this is harder than it looks
//!
//! On Windows 11 the taskbar is a XAML surface: its buttons are **not** windows.
//! `WindowFromPoint` returns the same `Shell_TrayWnd` wherever you point, so
//! there is no handle to map back to an application. The Windhawk mods that do
//! this properly inject into `explorer.exe` and hook `CTaskBtnGroup` /
//! `CWindowTaskItem::GetWindow`, reading the shell's own task list. Lumen is an
//! external process and has no access to any of that.
//!
//! The only route from outside is UI Automation — but not the obvious one.
//! `IUIAutomation::ElementFromPoint` does **not** descend into the taskbar's
//! XAML content: on Windows 11 it returns the bare `Shell_TrayWnd` pane, with an
//! empty name, for every point along the bar. (Measured directly: 24 probes
//! across three rows of the taskbar, all of them `ControlType.Pane`,
//! `ClassName=Shell_TrayWnd`, `Name=""`.) An implementation built on it silently
//! resolves nothing, forever, and looks exactly like a feature that was never
//! wired up.
//!
//! What does work is walking the tree from the taskbar's own window and
//! hit-testing the buttons' bounding rectangles ourselves.
//!
//! # Matching a button to a window
//!
//! A button's name is its window's title with a localised suffix appended —
//! `"KYSLINGO - The Law of Recognition —1 запущенное окно"`. The suffix differs
//! per language and per count, so it is never parsed. Instead the label is
//! matched by **prefix**: the button belongs to the window whose title the label
//! starts with, longest first. That is exact, needs no separator convention, and
//! works in any UI language.
//!
//! A pinned application that is not running is labelled differently again
//! (`"Приложение Telegram закреплено"`), and prefixes no window title at all, so
//! it correctly resolves to nothing.
//!
//! # The safety rule
//!
//! Closing the wrong application loses someone's work. So the matcher is built
//! to **refuse rather than guess**: if the label matches no window, or matches
//! windows belonging to more than one process, nothing is closed and the reason
//! is logged. A missed close is a minor annoyance; a wrong close is not.
//!
//! The same matcher serves volume, where the stakes are far lower, because a
//! rule that is right for the dangerous case is not worth relaxing for the safe
//! one.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        System::Variant::VARIANT,
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationCondition, TreeScope_Descendants,
                UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
            },
            WindowsAndMessaging::{
                EnumWindows, FindWindowExW, FindWindowW, GA_ROOT, GWL_EXSTYLE, GetAncestor,
                GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
                PostMessageW, WM_CLOSE, WS_EX_TOOLWINDOW, WindowFromPoint,
            },
        },
    },
    core::{BOOL, w},
};

use crate::appinfo::{file_name, friendly_name, image_path};
use crate::audio::volume::AppTarget;

/// Processes that must never be closed by an accidental click.
///
/// Terminating or closing these ranges from "the desktop disappears" to an
/// immediate bugcheck, and none of them is ever something a user meant to
/// close from a taskbar button.
const PROTECTED: &[&str] = &[
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "smss.exe",
    "dwm.exe",
    "explorer.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "textinputhost.exe",
    "sihost.exe",
];

/// Automation class-name prefix for the notification area's own controls.
///
/// The clock, the network and volume icons and the hidden-icon chevron are all
/// `Button`s inside the same tray, and none of them is an application button.
/// Filtering by class rather than by name keeps this working in any UI language.
const SYSTEM_TRAY_PREFIX: &str = "SystemTray.";

#[derive(Debug)]
pub enum CloseOutcome {
    Closed { process: String, windows: usize },
    /// Deliberately did nothing, with the reason.
    Refused(String),
}

/// Label of the taskbar button whose rectangle contains a screen point.
///
/// COM-heavy and comparatively slow, so this must never run on the hook
/// callback — the caller does it from the action thread, behind a cache.
///
/// Both taskbars are searched: `Shell_TrayWnd` is the primary monitor's,
/// `Shell_SecondaryTrayWnd` every other monitor's.
fn button_label_at(pt: POINT) -> Option<String> {
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;

        // Server-side filtering. Reading the name and class of every element in
        // the taskbar subtree would be hundreds of cross-process calls; asking
        // for buttons only cuts it to about twenty.
        let buttons_only = automation
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_ButtonControlTypeId.0),
            )
            .ok()?;

        for class in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
            let mut tray = FindWindowW(class, None).unwrap_or_default();
            while !tray.is_invalid() {
                if let Some(label) = label_in_tray(&automation, tray, &buttons_only, pt) {
                    return Some(label);
                }
                // Every monitor beyond the first has its own secondary taskbar.
                tray = FindWindowExW(None, Some(tray), class, None).unwrap_or_default();
            }
        }
        None
    }
}

/// Search one taskbar's buttons for the one containing `pt`.
unsafe fn label_in_tray(
    automation: &IUIAutomation,
    tray: HWND,
    condition: &IUIAutomationCondition,
    pt: POINT,
) -> Option<String> {
    unsafe {
        // Entering the tree by handle rather than by a ClassName condition: the
        // handle is already known, and it avoids depending on the shell's
        // automation class names staying put across Windows builds.
        let root = automation.ElementFromHandle(tray).ok()?;
        let found = root.FindAll(TreeScope_Descendants, condition).ok()?;

        for i in 0..found.Length().unwrap_or(0) {
            let Ok(element) = found.GetElement(i) else { continue };
            let Ok(rect) = element.CurrentBoundingRectangle() else { continue };
            if pt.x < rect.left || pt.x >= rect.right || pt.y < rect.top || pt.y >= rect.bottom {
                continue;
            }

            // The clock and the tray icons are buttons too, and scrolling there
            // should keep moving the system master — which is what returning
            // nothing here produces.
            if let Ok(class) = element.CurrentClassName()
                && class.to_string().starts_with(SYSTEM_TRAY_PREFIX)
            {
                return None;
            }

            let Ok(name) = element.CurrentName() else { continue };
            let name = name.to_string();
            if !name.trim().is_empty() {
                return Some(name);
            }
        }
        None
    }
}

struct Candidate {
    hwnd: HWND,
    title: String,
    pid: u32,
    process: String,
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<Candidate>) };

    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL::from(true);
        }
        // Tool windows never appear on the taskbar, so they can never be what a
        // taskbar button refers to.
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL::from(true);
        }

        let mut title = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title);
        if len <= 0 {
            return BOOL::from(true);
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL::from(true);
        }

        out.push(Candidate {
            hwnd,
            title: String::from_utf16_lossy(&title[..len as usize]),
            pid,
            process: process_name(pid).unwrap_or_default(),
        });
    }
    BOOL::from(true)
}

fn process_name(pid: u32) -> Option<String> {
    image_path(pid).map(|full| file_name(&full).to_owned())
}

/// A taskbar button resolved to exactly one application.
struct Resolved {
    label: String,
    process: String,
    exe: String,
    windows: Vec<HWND>,
}

/// Map a screen point on the taskbar to the single application it belongs to.
///
/// `Err` carries the reason, which is the whole story whenever this feature
/// appears to do nothing.
fn resolve(pt: POINT) -> Result<Resolved, String> {
    let Some(label) = button_label_at(pt) else {
        return Err("no application button under the cursor".into());
    };

    let mut candidates: Vec<Candidate> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&raw mut candidates as isize));
    }

    let Some(anchor) = match_window(&label, &candidates) else {
        return Err(format!("no window matches button {label:?}"));
    };

    // Every window of the same application, not just the one that matched: a
    // grouped button stands for all of them, and closing one of three would look
    // like the feature half-worked.
    let matches: Vec<&Candidate> =
        candidates.iter().filter(|c| c.pid == anchor.pid || c.process == anchor.process).collect();

    // The refuse-rather-than-guess rule. Two different applications answering
    // to the same label means the mapping is genuinely ambiguous, and closing
    // either would be a coin flip with someone's unsaved work.
    let processes: HashSet<&str> = matches.iter().map(|c| c.process.as_str()).collect();
    if processes.len() > 1 {
        return Err(format!(
            "button {label:?} is ambiguous between {processes:?}; refusing to guess"
        ));
    }

    // The program's own name, not the button's label. The label is a window
    // title — "KYSLINGO - The Law of Recognition" — which names the song rather
    // than the program whose volume is about to move.
    let exe = image_path(anchor.pid).unwrap_or_default();
    Ok(Resolved {
        label: friendly_name(&exe, &anchor.process),
        process: anchor.process.clone(),
        exe,
        windows: matches.iter().map(|c| c.hwnd).collect(),
    })
}

/// Which window a taskbar button label refers to.
///
/// Primary rule: the label is the window's title plus a localised suffix, so the
/// title is a prefix of the label. The longest such title wins — otherwise a
/// window titled "C" would beat "Claude" for the label "Claude —1 running
/// window".
///
/// Fallback: a pinned or grouped button is labelled with the application's name
/// instead, so look for an executable stem appearing in the label. Only used
/// when no title matches, because it is much the weaker signal.
fn match_window<'a>(label: &str, candidates: &'a [Candidate]) -> Option<&'a Candidate> {
    let lower = label.to_lowercase();

    let by_title = candidates
        .iter()
        .filter(|c| {
            let title = c.title.trim();
            !title.is_empty() && lower.starts_with(&title.to_lowercase())
        })
        .max_by_key(|c| c.title.trim().chars().count());
    if by_title.is_some() {
        return by_title;
    }

    candidates
        .iter()
        .filter(|c| {
            let stem = c.process.strip_suffix(".exe").unwrap_or(&c.process).to_lowercase();
            stem.chars().count() >= 3 && lower.contains(&stem)
        })
        .max_by_key(|c| c.process.chars().count())
}

/// Close whatever application the taskbar button under `pt` represents.
///
/// Returns what happened, including every reason it declined to act.
pub fn close_at(pt: POINT) -> CloseOutcome {
    let resolved = match resolve(pt) {
        Ok(r) => r,
        Err(why) => return CloseOutcome::Refused(why),
    };

    if PROTECTED.iter().any(|p| p.eq_ignore_ascii_case(&resolved.process)) {
        return CloseOutcome::Refused(format!("{} is protected", resolved.process));
    }

    // WM_CLOSE is a *request*: the app can prompt to save, or refuse. That is
    // exactly the desired behaviour — this feature must never be able to
    // discard work without the application getting a say.
    let mut closed = 0usize;
    for hwnd in &resolved.windows {
        unsafe {
            if PostMessageW(Some(*hwnd), WM_CLOSE, Default::default(), Default::default()).is_ok() {
                closed += 1;
            }
        }
    }

    if closed == 0 {
        return CloseOutcome::Refused(format!(
            "{} accepted no WM_CLOSE (it may be elevated)",
            resolved.process
        ));
    }
    CloseOutcome::Closed { process: resolved.process, windows: closed }
}

// --- resolution cache -------------------------------------------------------
//
// One wheel flick produces a burst of notches over the same button, and walking
// the automation tree costs tens of milliseconds. Resolving once per gesture
// rather than once per notch is the difference between a fluid scroll and a
// stuttering one — and this runs on the action thread, so the cost would show up
// as lag between the wheel and the sound.

/// How long a resolution stays good for.
const CACHE_TTL: Duration = Duration::from_millis(1500);
/// How far the pointer may drift and still count as the same button.
const CACHE_SLOP: i32 = 12;

struct Cache {
    pt: POINT,
    at: Instant,
    target: Option<AppTarget>,
}

static CACHE: LazyLock<Mutex<Option<Cache>>> = LazyLock::new(|| Mutex::new(None));

/// The application whose taskbar button is under `pt`, for volume control.
///
/// `None` means the point is not on a button that maps to exactly one running
/// application — empty bar, shell furniture, or an ambiguous label — and the
/// caller should fall back to the system master volume.
pub fn app_at(pt: POINT) -> Option<AppTarget> {
    if let Ok(guard) = CACHE.lock()
        && let Some(cache) = guard.as_ref()
        && cache.at.elapsed() < CACHE_TTL
        && (cache.pt.x - pt.x).abs() <= CACHE_SLOP
        && (cache.pt.y - pt.y).abs() <= CACHE_SLOP
    {
        return cache.target.clone();
    }

    let target = match resolve(pt) {
        Ok(r) if !r.exe.is_empty() => Some(AppTarget { exe: r.exe, label: r.label }),
        Ok(r) => {
            tracing::debug!("taskbar volume: could not read the image path for {}", r.process);
            None
        }
        Err(why) => {
            tracing::debug!("taskbar volume: {why}");
            None
        }
    };

    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(Cache { pt, at: Instant::now(), target: target.clone() });
    }
    target
}

/// The application owning the window that happens to be under `pt`.
///
/// Used when a full-screen window is covering the taskbar: there is no button to
/// read, but the thing drawing over the bar is exactly the application whose
/// volume the gesture is almost certainly aimed at.
///
/// `exclude` is Lumen's own window, which must never be treated as the target.
pub fn window_app_at(pt: POINT, exclude: isize) -> Option<AppTarget> {
    let root = unsafe {
        let hwnd = WindowFromPoint(pt);
        if hwnd.is_invalid() {
            return None;
        }
        GetAncestor(hwnd, GA_ROOT)
    };
    if root.is_invalid() || root.0 as isize == exclude {
        return None;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
    let exe = image_path(pid)?;

    // The shell's own windows are what a *visible* taskbar looks like, and they
    // are never the intended target of this fallback.
    let file = file_name(&exe);
    if PROTECTED.iter().any(|p| p.eq_ignore_ascii_case(file)) {
        return None;
    }

    let label = friendly_name(&exe, file);
    if label.is_empty() {
        return None;
    }

    Some(AppTarget { exe, label })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(title: &str, process: &str, pid: u32) -> Candidate {
        Candidate {
            hwnd: HWND(std::ptr::null_mut()),
            title: title.to_owned(),
            pid,
            process: process.to_owned(),
        }
    }

    /// The real label observed on this machine, in Russian, with the em-dash
    /// suffix the shell appends. Nothing parses that suffix — that is the point.
    #[test]
    fn matches_a_button_by_its_window_title_prefix() {
        let windows = [
            candidate("KYSLINGO - The Law of Recognition", "Spotify.exe", 2288),
            candidate("Claude", "claude.exe", 19500),
        ];
        let hit = match_window(
            "KYSLINGO - The Law of Recognition —1 запущенное окно",
            &windows,
        )
        .expect("the Spotify window should match its own button");
        assert_eq!(hit.process, "Spotify.exe");
    }

    /// A one-character title is a prefix of almost everything, so shortest-first
    /// would hand every button to whichever window happened to be titled "C".
    #[test]
    fn prefers_the_longest_matching_title() {
        let windows = [
            candidate("C", "other.exe", 1),
            candidate("Claude", "claude.exe", 2),
        ];
        let hit = match_window("Claude —1 запущенное окно", &windows).unwrap();
        assert_eq!(hit.process, "claude.exe");
    }

    /// A pinned application is labelled by name, not by window title, and in the
    /// user's language: "Приложение Telegram закреплено".
    #[test]
    fn falls_back_to_the_executable_name_when_no_title_matches() {
        let windows = [candidate("Saved Messages", "Telegram.exe", 7)];
        let hit = match_window("Приложение Telegram закреплено", &windows).unwrap();
        assert_eq!(hit.process, "Telegram.exe");
    }

    /// Two-letter executables would match far too much by substring alone.
    #[test]
    fn ignores_very_short_executable_stems() {
        let windows = [candidate("something", "ai.exe", 9)];
        assert!(match_window("Приложение Telegram закреплено", &windows).is_none());
    }

    #[test]
    fn returns_nothing_when_no_window_corresponds() {
        let windows = [candidate("Claude", "claude.exe", 2)];
        assert!(match_window("Приложение Проводник закреплено", &windows).is_none());
    }

    // Naming is covered by `crate::appinfo`, which owns it now — the taskbar,
    // the island and the audio session lookup all have to agree on what a
    // program is called.
}
