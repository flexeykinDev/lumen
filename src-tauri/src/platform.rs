//! Which Windows this is, and what that means for the features that differ.
//!
//! Lumen supports Windows 10 1809 and up, and several things behave differently
//! across that range. Rather than scatter version checks through the code, every
//! one of them is answered here, in terms of the feature rather than the build
//! number — so the call sites read as "can we do this" instead of "is this
//! 22000 or later".
//!
//! Nothing here gates anything off defensively. The graceful paths already
//! exist (DWM attributes fail silently, `window-vibrancy` falls back on its
//! own); this is so the *log and the About tab* can say which paths are in use,
//! which is the difference between "Mica does not work on my machine" being a
//! bug report and being a footnote.

use std::sync::OnceLock;

use windows::{
    Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
    core::{PCWSTR, w},
};

/// Windows build number, e.g. 19045 for Windows 10 22H2, 22621 for Win11 22H2.
///
/// Read from the registry rather than from `GetVersionExW`, which lies. That
/// API reports the highest version the *calling binary* is manifested for, so
/// an unmanifested one is told it is on Windows 8 — measured, not assumed: it
/// returned build 9200 here, on Windows 11, and every capability below would
/// have answered "no" on the strength of it.
///
/// Read once: the OS does not change under a running process.
pub fn build() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| {
        read_string(w!("CurrentBuildNumber")).and_then(|s| s.trim().parse().ok()).unwrap_or(0)
    })
}

/// One `REG_SZ` from the Windows version key.
fn read_string(name: PCWSTR) -> Option<String> {
    const KEY: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");

    let mut buffer = [0u16; 64];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }

    // `size` is in bytes and includes the terminator.
    let chars = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..chars.min(buffer.len())]))
}

/// Windows 11, by the only definition Microsoft gives: build 22000 or later.
pub fn is_windows_11() -> bool {
    build() >= 22000
}

/// Whether DWM will round the window's corners for us.
///
/// `DWMWA_WINDOW_CORNER_PREFERENCE` arrived with Windows 11. On Windows 10 the
/// call fails silently and the window stays square — which is why the capsule
/// also carries a CSS radius, and why the radius setting is the one that
/// actually shapes it there.
pub fn dwm_rounds_corners() -> bool {
    is_windows_11()
}

/// Whether Mica is available.
///
/// `DWMSBT_MAINWINDOW` needs Windows 11 22H2. Below that, `apply` falls through
/// to Acrylic, which on Windows 10 is the legacy
/// `SetWindowCompositionAttribute` path — visually close enough that most
/// people never notice, and the reason the fallback chain exists.
pub fn supports_mica() -> bool {
    build() >= 22621
}

/// Whether per-process audio capture is available.
///
/// `AUDCLNT_ACTIVATION_TYPE_PROCESS_LOOPBACK` arrived in Windows 10 2004
/// (19041). Below that the activation fails at runtime with a COM error, which
/// is a poor way to learn that a feature needs a newer Windows — so boost asks
/// this first and says so plainly instead.
pub fn supports_process_loopback() -> bool {
    build() >= 19041
}

/// One line for the log and the About tab.
pub fn summary() -> String {
    let build = build();
    let name = if build >= 22000 { "Windows 11" } else { "Windows 10" };
    format!(
        "{name} build {build} — mica: {}, dwm corners: {}, volume boost: {}",
        yes_no(supports_mica()),
        yes_no(dwm_rounds_corners()),
        yes_no(supports_process_loopback()),
    )
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_number_is_plausible() {
        // Any supported Windows is 17763 (1809) or later. A zero, or a build
        // from the Windows 8 era, means the lookup fell for the version lie —
        // which would make every capability below answer "no" and quietly
        // disable features that work.
        let build = build();
        assert!(build >= 17763, "implausible build number: {build}");
    }

    #[test]
    fn the_capability_thresholds_are_ordered() {
        // Each feature arrived in this order, so the answers have to nest: a
        // machine with the later one necessarily has the earlier ones. Asserted
        // against the functions rather than the literals, which would only be
        // comparing two constants to each other.
        if is_windows_11() {
            assert!(supports_process_loopback(), "Windows 11 always has process loopback");
        }
        if supports_mica() {
            assert!(dwm_rounds_corners(), "anything with Mica also rounds corners");
        }
    }

    #[test]
    fn the_summary_names_the_operating_system() {
        let text = summary();
        println!("{text}");
        assert!(text.starts_with("Windows 1"), "{text}");
        assert!(text.contains("build "), "{text}");
    }
}
