//! Per-application volume, via Core Audio's session API.
//!
//! # Why this exists, and why master volume is not enough
//!
//! Windows applies the two volumes at different points in the audio pipeline:
//!
//! ```text
//!   app renders samples
//!        |
//!        v
//!   ISimpleAudioVolume   <-- per-session ("Volume Mixer"), THIS module
//!        |
//!        v
//!   audio engine mix
//!        |
//!        v
//!   IAudioEndpointVolume <-- master, `volume.rs`
//!        |
//!        v
//!   the speakers
//! ```
//!
//! Anything that captures an application's audio for streaming or recording taps
//! it *before* the endpoint, so turning the master volume down quietens the
//! listener's own speakers and nothing else — a stream keeps hearing the original
//! level. The session volume is upstream of the mix, so it is the one lever that
//! moves what a capture actually receives.
//!
//! # Matching sessions to an application
//!
//! A session reports the process id that created it, but that is rarely the
//! process the *user* thinks of: browsers, Electron apps and most game engines
//! render audio from a child process, so the pid behind a taskbar button and the
//! pid behind its sound are routinely different. Matching on the full executable
//! path instead catches the whole family, because those children run the same
//! image as their parent.
//!
//! It also sidesteps pid reuse entirely, which a parent-chain walk would have to
//! defend against with process creation times.

use anyhow::{Context, anyhow};
use windows::{
    Win32::{
        Foundation::CloseHandle,
        Media::Audio::{
            EDataFlow, ERole, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
            ISimpleAudioVolume, MMDeviceEnumerator, eConsole, eRender,
            Endpoints::IAudioMeterInformation,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance},
            Threading::{
                OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
    },
    core::Interface,
};

/// Where an application's volume ended up after a change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppVolume {
    pub scalar: f32,
    pub muted: bool,
    /// How many audio sessions were moved. Useful in logs: a browser commonly
    /// has several, a game exactly one, and zero means the app is silent right
    /// now (the level is still remembered by Windows).
    pub sessions: usize,
}

/// Full image path of a process, lower-cased for comparison.
///
/// `None` for pid 0 (the system-sounds session) and for anything this process is
/// not allowed to query, both of which simply fail to match.
pub fn exe_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]).to_lowercase())
    }
}

/// Current peak level and mute state of `exe`'s first session.
///
/// Only used by tests, which is exactly what it is for: "is this application
/// actually making sound right now" is otherwise unanswerable from outside.
pub fn peak_and_mute(exe: &str) -> Option<(f32, bool)> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .ok()?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
        let list = manager.GetSessionEnumerator().ok()?;

        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(control) = list.GetSession(i) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = control2.GetProcessId() else { continue };
            if exe_path(pid).as_deref() != Some(exe) {
                continue;
            }
            let peak = control
                .cast::<IAudioMeterInformation>()
                .and_then(|m| m.GetPeakValue())
                .unwrap_or(0.0);
            let muted = control
                .cast::<ISimpleAudioVolume>()
                .and_then(|v| v.GetMute())
                .map(|m| m.as_bool())
                .unwrap_or(false);
            return Some((peak, muted));
        }
        None
    }
}

/// A process id belonging to `exe`'s audio, or `None` when it is playing none.
///
/// Any one of them: process loopback captures the target's whole process tree,
/// and a browser's audio pids are siblings under the same parent, so whichever
/// is found first leads to the same audio.
pub fn any_pid(exe: &str) -> Option<u32> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .ok()?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
        let list = manager.GetSessionEnumerator().ok()?;

        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(control) = list.GetSession(i) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = control2.GetProcessId() else { continue };
            if exe_path(pid).as_deref() == Some(exe) {
                return Some(pid);
            }
        }
        None
    }
}

/// The application currently making the most noise, as `(exe path, pid)`.
///
/// Peak level rather than "is a session active": several applications keep an
/// active session open having played nothing for hours, and a silent one is
/// never the answer to "what is playing".
pub fn loudest_session() -> Option<(String, u32)> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .ok()?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
        let list = manager.GetSessionEnumerator().ok()?;

        let mut best: Option<(String, u32, f32)> = None;
        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(control) = list.GetSession(i) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = control2.GetProcessId() else { continue };
            let Some(exe) = exe_path(pid) else { continue };
            let Ok(meter) = control.cast::<IAudioMeterInformation>() else { continue };
            let Ok(peak) = meter.GetPeakValue() else { continue };
            if best.as_ref().is_none_or(|(_, _, loudest)| peak > *loudest) {
                best = Some((exe, pid, peak));
            }
        }

        best.filter(|(_, _, peak)| *peak > 0.0).map(|(exe, pid, _)| (exe, pid))
    }
}

/// Every audio session on the default render device that belongs to `exe`.
fn sessions_for(exe: &str) -> anyhow::Result<Vec<ISimpleAudioVolume>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("could not create the device enumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .context("no default render endpoint")?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .context("could not activate IAudioSessionManager2")?;
        let list = manager
            .GetSessionEnumerator()
            .context("could not enumerate audio sessions")?;

        let count = list.GetCount().unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..count {
            // One unreadable session must not abandon the rest: sessions come and
            // go while we walk the list, so a failure here is routine.
            let Ok(control) = list.GetSession(i) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = control2.GetProcessId() else { continue };
            if exe_path(pid).as_deref() != Some(exe) {
                continue;
            }
            if let Ok(volume) = control.cast::<ISimpleAudioVolume>() {
                out.push(volume);
            }
        }
        Ok(out)
    }
}

/// Move `exe`'s volume by `delta`, or set it outright.
///
/// All of the application's sessions are driven to the same level rather than
/// each being nudged from wherever it happened to sit. A browser with three
/// sessions at three different levels would otherwise stay permanently out of
/// step with itself, and the Volume Mixer shows one slider per application.
pub fn adjust(exe: &str, delta: f32, absolute: Option<f32>) -> anyhow::Result<AppVolume> {
    let sessions = sessions_for(exe)?;
    let first = sessions
        .first()
        .ok_or_else(|| anyhow!("no audio session belongs to {exe}"))?;

    unsafe {
        let base = first.GetMasterVolume().unwrap_or(0.0);
        let next = absolute.unwrap_or(base + delta).clamp(0.0, 1.0);

        // Windows treats a level under half a percent as zero on the mixer, and
        // an application left "unmuted at 0%" reads as broken rather than quiet.
        let mute = next < 0.005;

        for session in &sessions {
            let _ = session.SetMasterVolume(next, std::ptr::null());
            if session.GetMute().map(|m| m.as_bool()) != Ok(mute) {
                let _ = session.SetMute(mute, std::ptr::null());
            }
        }

        Ok(AppVolume { scalar: next, muted: mute, sessions: sessions.len() })
    }
}

/// Find the audio session belonging to an application named `display`.
///
/// This is how the island's own wheel finds its target: SMTC identifies the
/// playing app by an AUMID, which is not a process and carries no pid, so the
/// only reliable bridge is the display name both sides already agree on —
/// `appinfo::friendly_name` for the audio session, `pretty_source` for the media
/// session. Both resolve Spotify to "Spotify" and Firefox to "Firefox".
///
/// Returns the executable path and the name to show, or `None` when the playing
/// application is rendering no audio through this device (a browser tab that has
/// gone silent, or playback on a different endpoint).
pub fn find_by_display_name(display: &str) -> Option<(String, String)> {
    let display = display.trim();
    if display.is_empty() {
        return None;
    }

    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .ok()?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
        let list = manager.GetSessionEnumerator().ok()?;

        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(control) = list.GetSession(i) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = control2.GetProcessId() else { continue };
            let Some(exe) = crate::appinfo::image_path(pid) else { continue };

            let file = crate::appinfo::file_name(&exe);
            let label = crate::appinfo::friendly_name(&exe, file);
            let stem = file.strip_suffix(".exe").unwrap_or(file);

            // Either the friendly name or the bare executable stem: SMTC's
            // AUMID for a desktop app is often literally "Spotify.exe", and the
            // version resource may say something longer.
            if label.eq_ignore_ascii_case(display) || stem.eq_ignore_ascii_case(display) {
                return Some((exe, label));
            }
        }
        None
    }
}

/// Current level of an application, without changing it.
pub fn read(exe: &str) -> Option<AppVolume> {
    let sessions = sessions_for(exe).ok()?;
    let first = sessions.first()?;
    unsafe {
        Some(AppVolume {
            scalar: first.GetMasterVolume().ok()?,
            muted: first.GetMute().ok()?.as_bool(),
            sessions: sessions.len(),
        })
    }
}

/// Flip mute for every session belonging to `exe`.
pub fn toggle_mute(exe: &str) -> anyhow::Result<AppVolume> {
    let sessions = sessions_for(exe)?;
    let first = sessions
        .first()
        .ok_or_else(|| anyhow!("no audio session belongs to {exe}"))?;

    unsafe {
        let next = !first.GetMute().map(|m| m.as_bool()).unwrap_or(false);
        for session in &sessions {
            let _ = session.SetMute(next, std::ptr::null());
        }
        Ok(AppVolume {
            scalar: first.GetMasterVolume().unwrap_or(0.0),
            muted: next,
            sessions: sessions.len(),
        })
    }
}
