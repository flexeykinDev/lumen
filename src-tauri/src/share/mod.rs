//! Save a rendered share card, and put it on the clipboard.
//!
//! # Why the drawing happens in the renderer
//!
//! The card is composed on a `<canvas>` in the WebView and arrives here as PNG
//! bytes. Drawing it in Rust would mean a font rasteriser and a layout pass —
//! `image` cannot render text — for a result that then has to be kept looking
//! like the capsule by hand, in a second place. The WebView already has the
//! fonts, the accent colour and the artwork decoded.
//!
//! So this module does the two things a WebView cannot: write a file where the
//! user can find it, and put the image on the Windows clipboard.

use std::path::PathBuf;

use anyhow::Context;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock},
        Ole::CF_DIB,
    },
};

/// Where cards are written. Chosen over the desktop or a temp folder because it
/// is where Windows itself puts screenshots, and it survives a reboot.
fn output_dir() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .context("USERPROFILE is not set")?;
    let dir = base.join("Pictures").join("Lumen");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("could not create {}", dir.display()))?;
    Ok(dir)
}

/// Write the card and return where it landed.
///
/// The name carries the track so a folder of these is browsable, and a
/// timestamp so saving the same track twice does not overwrite.
pub fn save(png: &[u8], title: &str, artist: &str) -> anyhow::Result<PathBuf> {
    let dir = output_dir()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let stem = sanitise(&format!("{artist} - {title}"));
    let path = dir.join(format!("{stem} {stamp}.png"));
    std::fs::write(&path, png).with_context(|| format!("could not write {}", path.display()))?;
    Ok(path)
}

/// Make a track title safe to use as a filename.
///
/// Windows rejects `\ / : * ? " < > |`, and silently mangles names ending in a
/// dot or a space. Track titles contain all of these routinely — `AC/DC`, or
/// anything with a `?` — so this is the common case, not a hostile one.
fn sanitise(name: &str) -> String {
    const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let cleaned: String = name
        .chars()
        .map(|c| if FORBIDDEN.contains(&c) || c.is_control() { '-' } else { c })
        .collect();
    // Long titles plus a path plus an extension can exceed MAX_PATH.
    let trimmed: String = cleaned.trim().chars().take(96).collect();
    let trimmed = trimmed.trim_end_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() { "Lumen".to_owned() } else { trimmed }
}

/// Put the card on the clipboard as a device-independent bitmap.
///
/// `CF_DIB` rather than a PNG blob because that is what paste targets actually
/// read — Discord, Word, Paint. The header is a bare `BITMAPINFOHEADER`
/// followed by pixels, with no file header, and the rows run bottom-up.
pub fn copy_to_clipboard(png: &[u8]) -> anyhow::Result<()> {
    let image = image::load_from_memory(png).context("share card is not a readable image")?;
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    anyhow::ensure!(w > 0 && h > 0, "share card has no pixels");

    let header_size = std::mem::size_of::<BITMAPINFOHEADER>();
    let stride = (w as usize) * 4;
    let mut dib = vec![0u8; header_size + stride * h as usize];

    let header = BITMAPINFOHEADER {
        biSize: header_size as u32,
        biWidth: w as i32,
        // Positive height means bottom-up, which is the conventional DIB layout
        // and the one every paste target handles.
        biHeight: h as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: (stride * h as usize) as u32,
        ..Default::default()
    };
    // SAFETY: `header` is plain old data and the destination is large enough.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&raw const header).cast::<u8>(),
            dib.as_mut_ptr(),
            header_size,
        );
    }

    // RGBA top-down to BGRA bottom-up.
    for y in 0..h as usize {
        let src = &rgba.as_raw()[y * stride..(y + 1) * stride];
        let dst_row = h as usize - 1 - y;
        let dst = &mut dib[header_size + dst_row * stride..header_size + (dst_row + 1) * stride];
        for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
    }

    unsafe {
        OpenClipboard(Some(HWND::default())).context("could not open the clipboard")?;
        // From here on the clipboard is open and must be closed on every path,
        // or every other application loses access to it.
        let result = (|| -> anyhow::Result<()> {
            EmptyClipboard().context("could not empty the clipboard")?;

            let handle = GlobalAlloc(GHND, dib.len()).context("clipboard allocation failed")?;
            let ptr = GlobalLock(handle);
            anyhow::ensure!(!ptr.is_null(), "could not lock clipboard memory");
            std::ptr::copy_nonoverlapping(dib.as_ptr(), ptr.cast::<u8>(), dib.len());
            let _ = GlobalUnlock(handle);

            // Ownership of the handle passes to the clipboard on success; on
            // failure it would leak, which is why the result is checked.
            SetClipboardData(CF_DIB.0 as u32, Some(windows::Win32::Foundation::HANDLE(handle.0)))
                .context("could not set clipboard data")?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Track titles routinely contain characters Windows forbids in a filename.
    #[test]
    fn sanitise_strips_characters_windows_rejects() {
        let out = sanitise(r#"AC/DC - Who Made Who? <live>"#);
        for c in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!out.contains(c), "{c:?} survived in {out:?}");
        }
    }

    /// A name ending in a dot or a space is silently mangled by the shell.
    #[test]
    fn sanitise_trims_trailing_dots_and_spaces() {
        assert!(!sanitise("Track name. ").ends_with(['.', ' ']));
        assert_eq!(sanitise("   "), "Lumen");
        assert_eq!(sanitise(""), "Lumen");
    }

    /// Path length is finite; a long title plus a folder plus a timestamp is
    /// exactly how MAX_PATH gets hit.
    #[test]
    fn sanitise_caps_length() {
        assert!(sanitise(&"a".repeat(500)).chars().count() <= 96);
    }

    /// The name is for humans to browse, so ordinary punctuation stays.
    #[test]
    fn sanitise_keeps_readable_punctuation() {
        assert_eq!(sanitise("Death Grips - I've Seen Footage"), "Death Grips - I've Seen Footage");
    }
}
