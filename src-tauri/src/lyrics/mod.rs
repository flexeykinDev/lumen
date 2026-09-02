//! Time-synced lyrics, from LRCLIB.
//!
//! # Privacy
//!
//! This is the only part of Lumen that talks to the network, and it sends the
//! artist, title, album and duration of whatever is playing to `lrclib.net`.
//! That is a fair trade for lyrics and a poor one to make on someone's behalf,
//! so it is **off by default** and the README says plainly what leaves the
//! machine.
//!
//! # No HTTP crate
//!
//! WinHTTP is already on the machine and uses the system certificate store, so
//! there is no TLS stack to ship. `reqwest` and friends would add megabytes
//! against a 10 MB budget for one GET request.
//!
//! # Fetch once per track
//!
//! The whole timed lyric is handed to the renderer in a single event when the
//! track changes, and the renderer picks the current line from the clock it is
//! already interpolating. That keeps playback free of IPC — the same rule the
//! progress bar follows — and means one HTTP request per track, never per line.

use std::sync::{
    Arc, Mutex,
    mpsc::{self, Sender},
};

use serde::Serialize;

use crate::media::NowPlaying;

mod http;
mod lrc;

pub use lrc::{Line, parse};

/// Lyrics for one track, as handed to the renderer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// The session the lyric belongs to, so a late arrival for a track that has
    /// already been skipped can be discarded rather than shown over the new one.
    pub session_id: String,
    pub revision: u64,
    /// Timed lines, ascending. Empty when only unsynced lyrics exist.
    pub lines: Vec<Line>,
    /// True when the source had lyrics but no timings — worth saying, because
    /// "no lyrics" and "lyrics we cannot follow" look identical otherwise.
    pub plain_only: bool,
}

#[derive(Debug)]
enum Cmd {
    Fetch(Box<NowPlaying>),
    Shutdown,
}

pub struct LyricsService {
    tx: Sender<Cmd>,
}

impl LyricsService {
    /// Start the fetcher. `on_lyrics` is called on the worker thread whenever a
    /// track's lyrics arrive.
    pub fn start(on_lyrics: impl Fn(Lyrics) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();

        let _ = std::thread::Builder::new().name("lumen-lyrics".into()).spawn(move || {
            // One entry, not a map: only the current track is ever displayed,
            // and an unbounded cache of every track ever played is a leak with
            // no user-visible benefit.
            let mut last: Option<(String, u64)> = None;

            while let Ok(cmd) = rx.recv() {
                let np = match cmd {
                    Cmd::Shutdown => break,
                    Cmd::Fetch(np) => np,
                };

                let key = (np.session_id.clone(), np.revision);
                if last.as_ref() == Some(&key) {
                    continue;
                }
                last = Some(key);

                match fetch(&np) {
                    Ok(Some(lyrics)) => on_lyrics(lyrics),
                    Ok(None) => {
                        tracing::debug!("no lyrics for {} - {}", np.artist, np.title);
                        // Still reported, so the UI can stop waiting.
                        on_lyrics(Lyrics {
                            session_id: np.session_id.clone(),
                            revision: np.revision,
                            lines: Vec::new(),
                            plain_only: false,
                        });
                    }
                    Err(e) => tracing::debug!("lyrics lookup failed: {e:#}"),
                }
            }
        });

        Self { tx }
    }

    /// Ask for lyrics for a track. Cheap to call repeatedly — the worker skips
    /// anything it has already looked up.
    pub fn request(&self, np: &NowPlaying) {
        if np.title.trim().is_empty() {
            return;
        }
        let _ = self.tx.send(Cmd::Fetch(Box::new(np.clone())));
    }
}

impl Drop for LyricsService {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

/// Ask LRCLIB for one track.
fn fetch(np: &NowPlaying) -> anyhow::Result<Option<Lyrics>> {
    let query = format!(
        "/api/get?artist_name={}&track_name={}&album_name={}&duration={}",
        http::encode(&np.artist),
        http::encode(&np.title),
        http::encode(&np.album),
        np.timeline.duration_sec.max(0.0).round() as i64
    );

    let Some(body) = http::get("lrclib.net", &query)? else {
        // 404 is the ordinary "we do not have this track" answer.
        return Ok(None);
    };

    let value: serde_json::Value = serde_json::from_slice(&body)?;
    let synced = value.get("syncedLyrics").and_then(|v| v.as_str()).unwrap_or_default();
    let plain = value.get("plainLyrics").and_then(|v| v.as_str()).unwrap_or_default();

    let lines = parse(synced);
    if lines.is_empty() && plain.trim().is_empty() {
        return Ok(None);
    }

    tracing::info!(
        "lyrics for {} - {}: {} timed line(s)",
        np.artist,
        np.title,
        lines.len()
    );
    Ok(Some(Lyrics {
        session_id: np.session_id.clone(),
        revision: np.revision,
        plain_only: lines.is_empty(),
        lines,
    }))
}

/// Shared handle so the composition root can hold one and the media pump can
/// reach it.
pub type Shared = Arc<Mutex<Option<LyricsService>>>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Hits the real service, so it is not part of the normal run — it needs a
    /// network and it depends on someone else's database still holding this
    /// track. Run deliberately:
    ///
    /// ```text
    /// cargo test -- --ignored lyrics
    /// ```
    #[test]
    #[ignore = "requires network access to lrclib.net"]
    fn live_lookup_returns_timed_lines() {
        let query = format!(
            "/api/get?artist_name={}&track_name={}&album_name={}&duration=200",
            http::encode("The Weeknd"),
            http::encode("Blinding Lights"),
            http::encode("After Hours"),
        );
        let body = http::get("lrclib.net", &query)
            .expect("the request itself must succeed")
            .expect("lrclib should know this track");

        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("the response must be JSON");
        let synced = value.get("syncedLyrics").and_then(|v| v.as_str()).unwrap_or_default();
        let lines = parse(synced);
        assert!(!lines.is_empty(), "expected timed lyrics, got {} bytes", body.len());
        assert!(lines[0].at_sec >= 0.0);
    }

    /// A track that cannot exist must come back as "not found", not as an error
    /// — that distinction is what stops the UI reporting a failure for every
    /// obscure song.
    #[test]
    #[ignore = "requires network access to lrclib.net"]
    fn live_lookup_of_a_nonexistent_track_is_not_an_error() {
        let query = format!(
            "/api/get?artist_name={}&track_name={}&album_name=&duration=1",
            http::encode("zzzz nonexistent artist qqq"),
            http::encode("zzzz nonexistent track qqq"),
        );
        assert!(matches!(http::get("lrclib.net", &query), Ok(None)));
    }
}
