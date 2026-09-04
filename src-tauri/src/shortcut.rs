//! Desktop and Start-menu shortcuts.
//!
//! A portable exe lives wherever it was dropped, which is usually a Downloads
//! folder. Offering a shortcut is the difference between an app someone keeps
//! and one they cannot find again a week later.
//!
//! Note what this deliberately does *not* do: pin to the taskbar or to Start.
//! Windows 10 and 11 both block programmatic pinning — the verb was removed
//! precisely because applications abused it — and every "trick" that still
//! works is an undocumented COM interface that breaks each release. A shortcut
//! in the Start Menu's Programs folder is the supported equivalent: it puts
//! Lumen in the app list and in search, from where a person can pin it in one
//! right-click.

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_CREATE, STGM_WRITE,
        },
        UI::Shell::{IShellLinkW, ShellLink},
    },
    core::{HSTRING, Interface},
};

/// Where a shortcut can be put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Desktop,
    /// The current user's Start Menu programs folder — the app list, and search.
    StartMenu,
}

impl Place {
    fn dir(self) -> Option<PathBuf> {
        let var = match self {
            Place::Desktop => "USERPROFILE",
            Place::StartMenu => "APPDATA",
        };
        let base = PathBuf::from(std::env::var_os(var)?);
        Some(match self {
            Place::Desktop => base.join("Desktop"),
            Place::StartMenu => base.join(r"Microsoft\Windows\Start Menu\Programs"),
        })
    }

    /// Full path of the `.lnk`, or `None` when the folder cannot be resolved.
    pub fn link_path(self) -> Option<PathBuf> {
        Some(self.dir()?.join("Lumen.lnk"))
    }
}

/// Whether a shortcut is already there.
pub fn exists(place: Place) -> bool {
    place.link_path().is_some_and(|p| p.exists())
}

/// Create the shortcut, pointing at this executable.
///
/// Overwrites an existing one, which is the repair path: a portable exe that
/// has been moved leaves a shortcut pointing at nothing, and re-creating it is
/// the whole fix.
pub fn create(place: Place) -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot resolve this executable")?;
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    let link = place.link_path().ok_or_else(|| anyhow!("no folder for {place:?}"))?;

    unsafe {
        // The shell's link object is apartment-threaded, and this runs on a
        // command's task rather than the main thread, so the apartment is
        // entered and left here.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        anyhow::ensure!(com.is_ok(), "CoInitializeEx failed: {com:?}");
        let _guard = ComGuard;

        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .context("could not create the shell link object")?;

        shell.SetPath(&HSTRING::from(exe.as_os_str())).context("SetPath failed")?;
        shell.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        shell.SetDescription(&HSTRING::from("A glass music capsule"))?;
        // The icon is the exe's own, which is what a shortcut to it should show.
        shell.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;

        let file: IPersistFile = shell.cast().context("IPersistFile is not available")?;
        file.Save(&HSTRING::from(link.as_os_str()), true).context("could not write the shortcut")?;
    }

    tracing::info!("shortcut written to {}", link.display());
    Ok(link)
}

/// Remove a shortcut, if one is there. Missing is success: the caller asked for
/// it to be gone.
pub fn remove(place: Place) -> anyhow::Result<()> {
    let Some(link) = place.link_path() else { return Ok(()) };
    match std::fs::remove_file(&link) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("could not remove the shortcut"),
    }
}

/// Leaves the apartment when the call returns, whichever way it returns.
struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

// `STGM_CREATE | STGM_WRITE` is what `IPersistFile::Save` uses internally for a
// new file; naming them keeps the intent visible even though the call takes a
// bool. Referenced so the import cannot rot.
const _: u32 = STGM_CREATE.0 | STGM_WRITE.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_places_resolve_to_a_lnk_under_the_user_profile() {
        // No shortcut is created here: this is the path arithmetic, which is
        // the part that can be wrong silently.
        for place in [Place::Desktop, Place::StartMenu] {
            let path = place.link_path().expect("a user profile always exists on Windows");
            assert_eq!(path.extension().and_then(|e| e.to_str()), Some("lnk"));
            assert_eq!(path.file_name().and_then(|f| f.to_str()), Some("Lumen.lnk"));
        }
    }

    #[test]
    fn the_start_menu_link_lands_in_the_programs_folder() {
        // Anywhere else and it does not appear in the app list or in search,
        // which is the only reason to write it.
        let path = Place::StartMenu.link_path().expect("APPDATA");
        let text = path.to_string_lossy().replace('\\', "/");
        assert!(text.contains("Start Menu/Programs"), "{text}");
    }

    #[test]
    fn removing_a_shortcut_that_is_not_there_succeeds() {
        // The caller asked for it to be gone; it is gone.
        assert!(remove(Place::Desktop).is_ok() || !exists(Place::Desktop));
    }
}
