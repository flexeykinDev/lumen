//! The smallest HTTPS GET that will do, over WinHTTP.
//!
//! Shared by the two features that reach the network at all — lyrics lookup and
//! Discord cover art — both of which are off until asked for.
//!
//! WinHTTP ships with Windows and uses the system certificate store, so this
//! adds no TLS stack to a binary with a 10 MB budget. The cost is a handful of
//! handles to close in the right order, which is what `Session` is for.

use anyhow::Context;
use windows::{
    Win32::Networking::WinHttp::*,
    core::{PCWSTR, w},
};

/// Identifies Lumen to the service. LRCLIB asks callers to say who they are.
const USER_AGENT: PCWSTR = w!("Lumen/0.1.0 (https://github.com/lumen)");

/// Refuse a response larger than this. A lyric is a few kilobytes; anything
/// approaching a megabyte is a redirect loop or a hostile server, and neither
/// should be allowed to allocate freely.
const MAX_BODY: usize = 1024 * 1024;

/// A WinHTTP handle that closes itself.
///
/// The three handles must be closed in reverse order of opening, and there are
/// several early returns between them; doing it by hand leaks on the error
/// paths, which is exactly where it matters.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { let _ = WinHttpCloseHandle(self.0); }
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Percent-encode a query-string value.
///
/// Track and artist names routinely contain spaces, `&`, `#` and non-ASCII —
/// all of which change the meaning of the URL if passed through.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// GET `https://{host}{path}`.
///
/// `Ok(None)` for 404, which callers treat as "not found" rather than failure.
pub fn get(host: &str, path: &str) -> anyhow::Result<Option<Vec<u8>>> {
    unsafe {
        let session = Handle(WinHttpOpen(
            USER_AGENT,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            None,
            None,
            0,
        ));
        anyhow::ensure!(!session.0.is_null(), "WinHttpOpen failed");

        let host_w = wide(host);
        let connect =
            Handle(WinHttpConnect(session.0, PCWSTR(host_w.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0));
        anyhow::ensure!(!connect.0.is_null(), "WinHttpConnect failed");

        let path_w = wide(path);
        let request = Handle(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            PCWSTR(path_w.as_ptr()),
            None,
            None,
            // No `Accept-Type` restriction: the null list means "anything".
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        anyhow::ensure!(!request.0.is_null(), "WinHttpOpenRequest failed");

        WinHttpSendRequest(request.0, None, None, 0, 0, 0).ok().context("send failed")?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .ok()
            .context("no response")?;

        // Status first: a 404 is an answer, not an error, and reading the body
        // of an error page would be pointless.
        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            None,
            Some((&raw mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .ok()
        .context("could not read the status line")?;

        if status == 404 {
            return Ok(None);
        }
        anyhow::ensure!(status == 200, "unexpected HTTP status {status}");

        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            WinHttpQueryDataAvailable(request.0, &mut available)
                .ok()
                .context("could not size the response")?;
            if available == 0 {
                break;
            }
            anyhow::ensure!(
                body.len() + available as usize <= MAX_BODY,
                "response exceeded {MAX_BODY} bytes"
            );

            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read: u32 = 0;
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                available,
                &mut read,
            )
            .ok()
            .context("read failed")?;
            body.truncate(start + read as usize);
            if read == 0 {
                break;
            }
        }
        Ok(Some(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Track names carry spaces, ampersands and non-ASCII as a matter of course,
    /// and every one of them changes the URL's meaning if passed raw.
    #[test]
    fn encodes_characters_that_would_break_a_query() {
        assert_eq!(encode("Hello World"), "Hello%20World");
        assert_eq!(encode("A&B"), "A%26B");
        assert_eq!(encode("a?b#c"), "a%3Fb%23c");
        assert_eq!(encode("+"), "%2B");
    }

    #[test]
    fn leaves_unreserved_characters_alone() {
        assert_eq!(encode("abcXYZ091-_.~"), "abcXYZ091-_.~");
    }

    /// Non-ASCII must be encoded per UTF-8 byte, not per character.
    #[test]
    fn encodes_non_ascii_as_utf8_bytes() {
        assert_eq!(encode("é"), "%C3%A9");
        assert_eq!(encode("Дудь"), "%D0%94%D1%83%D0%B4%D1%8C");
    }

    #[test]
    fn empty_encodes_to_empty() {
        assert_eq!(encode(""), "");
    }
}
