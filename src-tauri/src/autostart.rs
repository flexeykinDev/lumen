//! Start with Windows.
//!
//! # Why the registry and not the Startup folder
//!
//! The Startup folder needs a `.lnk`, which means COM and `IShellLink` to write
//! one — more machinery, and a shortcut that silently rots when the exe moves.
//! `HKCU\...\CurrentVersion\Run` is a single string value, needs no elevation,
//! and applies only to the current user, which is the right scope for something
//! that lives in the tray.
//!
//! # Portable means the path can change
//!
//! Lumen is a drop-anywhere exe: it gets moved to another folder, or another
//! drive, far more often than an installed program does. A stale entry would
//! silently stop working, or worse, start whatever now sits at the old path. So
//! the stored command is compared against the running exe on every launch and
//! rewritten when it differs — see [`sync`].

use anyhow::{Context, Result};
use windows::{
    Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    },
    core::{PCWSTR, w},
};

/// The per-user Run key. Values here are launched once, at sign-in.
const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
/// Our value name inside it.
const VALUE: PCWSTR = w!("Lumen");

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The command Windows should run: the current exe, quoted.
///
/// Quoted because a path with a space in it — `C:\Program Files\…`, or any
/// folder someone happens to name with one — is otherwise parsed as a command
/// plus arguments, and silently launches nothing.
fn command() -> Result<String> {
    let exe = std::env::current_exe().context("cannot locate the running executable")?;
    Ok(format!("\"{}\"", exe.display()))
}

fn open(access: u32) -> Result<HKEY> {
    let mut key = HKEY::default();
    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            None,
            windows::Win32::System::Registry::REG_SAM_FLAGS(access),
            &mut key,
        )
        .ok()
        .context("could not open the Run key")?;
    }
    Ok(key)
}

/// What is registered today, if anything.
pub fn current() -> Option<String> {
    unsafe {
        let key = open(KEY_READ.0).ok()?;
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut size = 0u32;

        // First call sizes the buffer; a missing value fails here, which is the
        // ordinary "autostart is off" answer.
        let probe = RegQueryValueExW(key, VALUE, None, Some(&mut kind), None, Some(&mut size));
        if probe.is_err() || size == 0 {
            let _ = RegCloseKey(key);
            return None;
        }

        let mut buf = vec![0u8; size as usize];
        let read = RegQueryValueExW(
            key,
            VALUE,
            None,
            Some(&mut kind),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        read.ok().ok()?;

        // REG_SZ is UTF-16 with a trailing NUL that is not part of the string.
        let wide: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        Some(String::from_utf16_lossy(&wide))
    }
}

pub fn is_enabled() -> bool {
    current().is_some()
}

/// Register or remove the entry.
pub fn set(enabled: bool) -> Result<()> {
    unsafe {
        let key = open(KEY_WRITE.0 | KEY_READ.0)?;
        let result = if enabled {
            let value = wide(&command()?);
            let bytes = std::slice::from_raw_parts(
                value.as_ptr().cast::<u8>(),
                value.len() * std::mem::size_of::<u16>(),
            );
            RegSetValueExW(key, VALUE, None, REG_SZ, Some(bytes))
                .ok()
                .context("could not write the Run entry")
        } else {
            // Deleting something already absent is success, not failure.
            let _ = RegDeleteValueW(key, VALUE);
            Ok(())
        };
        let _ = RegCloseKey(key);
        result?;
    }
    tracing::info!("start with Windows: {}", if enabled { "on" } else { "off" });
    Ok(())
}

/// Reconcile the registry with the config, and with where the exe now lives.
///
/// Called at every startup. The path check is the point: a portable app gets
/// moved, and an entry pointing at the old location either does nothing or
/// launches whatever took its place.
pub fn sync(want_enabled: bool) {
    let existing = current();
    let expected = match command() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("autostart: {e:#}");
            return;
        }
    };

    match (want_enabled, existing) {
        (true, Some(found)) if found == expected => {}
        (true, Some(found)) => {
            tracing::info!("autostart path moved ({found} -> {expected}); rewriting");
            if let Err(e) = set(true) {
                tracing::warn!("autostart: {e:#}");
            }
        }
        (true, None) => {
            if let Err(e) = set(true) {
                tracing::warn!("autostart: {e:#}");
            }
        }
        (false, Some(_)) => {
            if let Err(e) = set(false) {
                tracing::warn!("autostart: {e:#}");
            }
        }
        (false, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path with a space is the common case on Windows, and an unquoted
    /// command silently launches nothing.
    #[test]
    fn command_is_quoted() {
        let c = command().expect("the test binary has a path");
        assert!(c.starts_with('"') && c.ends_with('"'), "not quoted: {c}");
        assert!(c.len() > 2);
    }

    /// Reading, writing and clearing round-trip against the real registry.
    ///
    /// Ignored on purpose: this writes to `HKCU\...\Run`, which is real,
    /// shared, machine-global state. On a developer's machine that is exactly
    /// the point — it is the only way to prove the write actually lands. On a
    /// CI runner it proves something else entirely: it passed on three runs
    /// and failed on the fourth, a *documentation-only* commit, because
    /// whether a hosted runner permits that write is not a property of this
    /// code. A test that fails for reasons unrelated to the change under test
    /// teaches people to ignore the red mark.
    ///
    /// Run it by hand after touching this module:
    ///   cargo test --lib autostart -- --ignored --nocapture
    #[test]
    #[ignore = "writes to the real HKCU Run key"]
    fn set_and_clear_round_trip() {
        let restore = current();

        set(true).expect("writing the Run entry must succeed");
        assert!(is_enabled());
        assert_eq!(current().as_deref(), Some(command().unwrap().as_str()));

        set(false).expect("clearing must succeed");
        assert!(!is_enabled(), "entry survived deletion");

        // Clearing twice is not an error.
        set(false).expect("clearing an absent entry is not a failure");

        if let Some(previous) = restore {
            let value = wide(&previous);
            unsafe {
                let key = open(KEY_WRITE.0 | KEY_READ.0).unwrap();
                let bytes = std::slice::from_raw_parts(
                    value.as_ptr().cast::<u8>(),
                    value.len() * std::mem::size_of::<u16>(),
                );
                let _ = RegSetValueExW(key, VALUE, None, REG_SZ, Some(bytes));
                let _ = RegCloseKey(key);
            }
        }
    }

    /// Reading the state must work anywhere, including where writing does not.
    ///
    /// This is what CI can honestly assert: a machine that forbids the write
    /// still has to answer "is it enabled" without panicking, and the answer
    /// has to agree with what is actually in the key.
    #[test]
    fn reading_the_state_never_panics() {
        let stored = current();
        assert_eq!(is_enabled(), stored.is_some());
    }
}
