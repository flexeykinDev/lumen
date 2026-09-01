//! What to call a running program, and where its executable lives.
//!
//! Shared because three different features need the same answer: the taskbar
//! close button, the taskbar volume wheel, and the island's own wheel. All of
//! them must agree on what "Spotify" means, or the readout names one thing while
//! a different one gets quieter.

use windows::{
    Win32::{
        Foundation::CloseHandle,
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::Threading::{
            OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    core::{PCWSTR, w},
};

/// Full image path of a process, lower-cased for comparison.
///
/// `None` for pid 0 (the system-sounds audio session) and for anything this
/// process may not query, both of which simply fail to match anywhere.
pub fn image_path(pid: u32) -> Option<String> {
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

/// Just the filename part of an image path.
pub fn file_name(exe_path: &str) -> &str {
    exe_path.rsplit(['\\', '/']).next().unwrap_or(exe_path)
}

/// What to call an application on screen.
///
/// The executable's own `FileDescription` is the right answer and the one
/// Windows itself uses in Task Manager's Name column. It matters most for games,
/// whose binaries are named after the engine target rather than the product:
/// Demonologist ships as `Shivers-Win64-Shipping.exe`, and no amount of string
/// munging on that filename produces "Demonologist" — but its version resource
/// says so directly.
///
/// A window title is never used. Titles carry the document, the track or the
/// conversation ("Savage Ga$p - paranoia agent"), which is not what a volume
/// readout is naming.
pub fn friendly_name(exe_path: &str, process: &str) -> String {
    if let Some(name) = version_name(exe_path) {
        return crate::media::model::trim_generic_suffix(name);
    }

    // No version resource. Fall back to the filename, minus the build-target
    // decorations that engines append.
    let stem = process.strip_suffix(".exe").unwrap_or(process);
    let stem = ["-Win64-Shipping", "-Win32-Shipping", "-Shipping", "-win64", "-x64", "_x64"]
        .iter()
        .find_map(|suffix| stem.strip_suffix(suffix))
        .unwrap_or(stem);

    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => process.to_owned(),
    }
}

/// `FileDescription` from an executable's version resource, then `ProductName`.
fn version_name(exe_path: &str) -> Option<String> {
    let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
    let path = PCWSTR(wide.as_ptr());

    unsafe {
        let size = GetFileVersionInfoSizeW(path, None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(path, Some(0), size, block.as_mut_ptr().cast()).ok()?;

        // The resource is keyed by language and code page, and there is no fixed
        // one to assume: an application localised into Russian stores its strings
        // under a different key than a neutral one. The translation table says
        // which keys actually exist.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            block.as_ptr().cast(),
            w!("\\VarFileInfo\\Translation"),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || len < 2
        {
            return None;
        }
        let (lang, codepage) = *(ptr as *const (u16, u16));

        for field in ["FileDescription", "ProductName"] {
            let sub = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\{field}");
            let sub_w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();

            let mut value = std::ptr::null_mut();
            let mut chars = 0u32;
            if VerQueryValueW(
                block.as_ptr().cast(),
                PCWSTR(sub_w.as_ptr()),
                &mut value,
                &mut chars,
            )
            .as_bool()
                && chars > 0
            {
                let slice = std::slice::from_raw_parts(value as *const u16, chars as usize);
                let raw = String::from_utf16_lossy(slice);

                // Stop at the first NUL rather than trusting the reported
                // length. The value is padded to a 32-bit boundary and the
                // length counts the padding, so slicing to it reads through the
                // terminator and on into the *next* key — which is what turned
                // Spotify's readout into "Spotify\0\0\0File" on screen.
                let text = raw.split('\0').next().unwrap_or("").trim().to_owned();
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback path, for an executable with no version resource. A real
    /// path is never passed here, so `version_name` fails and the filename rules
    /// are what get exercised.
    #[test]
    fn falls_back_to_a_tidied_filename_without_version_info() {
        assert_eq!(friendly_name("", "firefox.exe"), "Firefox");
        assert_eq!(friendly_name("", "Discord.exe"), "Discord");
    }

    /// Unreal names its binaries after the build target. Without this the volume
    /// readout would say "Shivers-Win64-Shipping".
    #[test]
    fn strips_engine_build_target_suffixes() {
        assert_eq!(friendly_name("", "Shivers-Win64-Shipping.exe"), "Shivers");
        assert_eq!(friendly_name("", "game-Win32-Shipping.exe"), "Game");
    }

    #[test]
    fn file_name_takes_the_last_path_segment() {
        assert_eq!(file_name("c:\\a\\b\\spotify.exe"), "spotify.exe");
        assert_eq!(file_name("spotify.exe"), "spotify.exe");
    }
}
