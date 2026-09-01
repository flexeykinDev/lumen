//! Single-instance guard.
//!
//! A portable tray app gets double-launched constantly — the exe is a loose file
//! people click twice, and there is no installer to register a launcher. Without
//! a guard the second instance docks a second island at the same coordinates and
//! silently loses every global hotkey to the first (`RegisterHotKey` is
//! first-come, first-served), which looks exactly like "the hotkeys broke".
//!
//! A named mutex in the Local namespace is the Win32 answer: it costs nothing,
//! needs no dependency, and the kernel releases it if we crash — so a hard kill
//! cannot lock the user out of their own app.

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
    },
    core::w,
};

/// Held for the lifetime of the process. Dropping it releases the claim.
pub struct InstanceGuard(HANDLE);

impl InstanceGuard {
    /// Claim the single-instance slot.
    ///
    /// Returns `None` when another Lumen already owns it. `Local\` scopes the
    /// name to the session, so two users on one machine each get their own
    /// island rather than blocking each other.
    pub fn acquire() -> Option<Self> {
        // SAFETY: a null security descriptor and a static name; the only
        // out-param is the returned handle, which we own from here on.
        let handle = unsafe { CreateMutexW(None, true, w!("Local\\dev.lumen.island.instance")) };

        match handle {
            Ok(handle) => {
                // CreateMutexW succeeds either way; ERROR_ALREADY_EXISTS is how
                // it reports that someone else got there first.
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    unsafe { let _ = CloseHandle(handle); }
                    None
                } else {
                    Some(Self(handle))
                }
            }
            Err(e) => {
                // Never let a guard failure stop the app from running.
                tracing::warn!("single-instance mutex unavailable ({e}); continuing unguarded");
                Some(Self(HANDLE::default()))
            }
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}
