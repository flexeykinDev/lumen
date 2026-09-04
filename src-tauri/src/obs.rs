//! Now-playing, as files a streaming tool can read.
//!
//! OBS cannot reliably capture the capsule itself: it is a layered, topmost
//! window with a DWM backdrop, and window capture of those ranges from a black
//! rectangle to nothing at all depending on the capture method. Rather than
//! fight that, this writes what is playing to plain files OBS already knows how
//! to read — a Text (GDI+) source pointed at a `.txt`, an Image source pointed
//! at the cover, or a Browser source pointed at the HTML.
//!
//! Written on a track change and nowhere else, so a stream that is not running
//! costs nothing. Off by default; see `config::Obs`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::media::{NowPlaying, PlaybackState};

/// Where the files go when no folder is configured.
const DEFAULT_DIR: &str = "obs";

/// Resolve the output directory, creating it if need be.
pub fn folder(configured: &str, config_path: Option<&Path>) -> Option<PathBuf> {
    let dir = if configured.trim().is_empty() {
        config_path.and_then(|p| p.parent()).map(|d| d.join(DEFAULT_DIR))?
    } else {
        PathBuf::from(configured.trim())
    };

    match fs::create_dir_all(&dir) {
        Ok(()) => Some(dir),
        Err(e) => {
            tracing::warn!("obs: cannot use {}: {e}", dir.display());
            None
        }
    }
}

/// The text of every file, ready to write.
#[derive(Debug, Default, PartialEq)]
pub struct Files {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `Artist — Title`, for a single Text source.
    pub line: String,
    pub html: String,
}

/// What each file should contain for `np`.
///
/// Split from the writing so the formatting can be tested without touching a
/// disk — which is the half with decisions in it.
pub fn render(np: Option<&NowPlaying>) -> Files {
    let Some(np) = np.filter(|n| n.state != PlaybackState::Stopped) else {
        // Stopped is worth publishing as empty: an overlay still showing a
        // track after the music ended is worse than a blank one.
        return Files::default();
    };

    let title = np.title.trim();
    let artist = np.artist.trim();
    let line = match (title.is_empty(), artist.is_empty()) {
        (false, false) => format!("{artist} — {title}"),
        (false, true) => title.to_owned(),
        _ => String::new(),
    };
    if line.is_empty() {
        return Files::default();
    }

    Files {
        title: title.to_owned(),
        artist: artist.to_owned(),
        album: np.album.trim().to_owned(),
        html: page(artist, title),
        line,
    }
}

/// A Browser-source page that keeps itself up to date.
///
/// Self-contained and offline: it re-reads `nowplaying.txt` beside it once a
/// second. A second is the right rate for a caption whose content only changes
/// when the track does.
fn page(artist: &str, title: &str) -> String {
    PAGE_TEMPLATE.replace("__TITLE__", &escape(title)).replace("__ARTIST__", &escape(artist))
}

const PAGE_TEMPLATE: &str = r####"<!doctype html>
<meta charset="utf-8">
<title>Now playing</title>
<style>
  body { margin: 0; background: transparent; font-family: "Segoe UI", system-ui, sans-serif; }
  #now { display: flex; gap: 14px; align-items: center; padding: 14px 18px; color: #fff;
         text-shadow: 0 2px 8px rgba(0, 0, 0, 0.65); }
  #cover { width: 64px; height: 64px; border-radius: 10px; object-fit: cover;
           box-shadow: 0 4px 18px rgba(0, 0, 0, 0.5); }
  #title { font-size: 20px; font-weight: 650; }
  #artist { font-size: 15px; opacity: 0.8; }
  .empty { display: none; }
</style>
<div id="now">
  <img id="cover" src="cover.jpg" alt="">
  <div>
    <div id="title">__TITLE__</div>
    <div id="artist">__ARTIST__</div>
  </div>
</div>
<script>
  // Plain polling of a local file, because that is all a Browser source can do
  // without a server behind it. One local read a second, and only while the
  // scene using it is live.
  const cover = document.getElementById("cover");
  cover.addEventListener("error", () => (cover.style.display = "none"));

  setInterval(async () => {
    try {
      const text = (await (await fetch("nowplaying.txt?" + Date.now())).text()).trim();
      const parts = text.split(" — ");
      document.getElementById("now").className = text ? "" : "empty";
      document.getElementById("title").textContent = parts[1] ?? text;
      document.getElementById("artist").textContent = parts[1] ? parts[0] : "";
      cover.style.display = "";
      cover.src = "cover.jpg?" + Date.now();
    } catch (e) {
      /* the file is mid-write; the next tick will have it */
    }
  }, 1000);
</script>
"####;

/// Minimal HTML escaping. Track names carry `&`, quotes and angle brackets
/// often enough that this is not theoretical.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Write the set out. Errors are logged, never surfaced: a failed write is not
/// a reason to interrupt anything.
pub fn write(dir: &Path, files: &Files, cover: Option<&[u8]>) {
    let put = |name: &str, body: &str| {
        if let Err(e) = fs::write(dir.join(name), body) {
            tracing::warn!("obs: could not write {name}: {e}");
        }
    };

    put("nowplaying.txt", &files.line);
    put("title.txt", &files.title);
    put("artist.txt", &files.artist);
    put("album.txt", &files.album);
    put("nowplaying.html", &files.html);

    if let Some(bytes) = cover
        && let Err(e) = fs::write(dir.join("cover.jpg"), bytes)
    {
        tracing::warn!("obs: could not write cover.jpg: {e}");
    }
}

/// Pull the bytes back out of the `data:` URL the renderer is handed.
///
/// SMTC gives the artwork as bytes, which the media layer turns into a data URL
/// for the WebView. An Image source in OBS needs a file, so this is the same
/// bytes on their way back out.
pub fn cover_bytes(data_uri: Option<&str>) -> Option<Vec<u8>> {
    use base64::{Engine, engine::general_purpose::STANDARD};

    let uri = data_uri?;
    let payload = uri.split_once("base64,")?.1;
    STANDARD.decode(payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Timeline;

    fn track(artist: &str, title: &str, state: PlaybackState) -> NowPlaying {
        NowPlaying {
            session_id: "s".into(),
            source: "Spotify".into(),
            title: title.into(),
            artist: artist.into(),
            album: "Album".into(),
            state,
            timeline: Timeline { position_sec: 0.0, duration_sec: 100.0, updated_at_ms: 0.0 },
            art_data_uri: None,
            accent: None,
            revision: 1,
        }
    }

    #[test]
    fn the_single_line_is_artist_then_title() {
        let files = render(Some(&track("KYSLINGO", "The Law", PlaybackState::Playing)));
        assert_eq!(files.line, "KYSLINGO — The Law");
        assert_eq!(files.title, "The Law");
        assert_eq!(files.artist, "KYSLINGO");
    }

    #[test]
    fn nothing_playing_writes_nothing_rather_than_the_last_track() {
        // An overlay still showing a song long after the music stopped is worse
        // than an empty one.
        assert_eq!(render(None), Files::default());
        assert_eq!(render(Some(&track("A", "B", PlaybackState::Stopped))), Files::default());
    }

    #[test]
    fn a_paused_track_is_still_what_is_playing() {
        // Paused is not stopped: the viewer is looking at a track that is up.
        let files = render(Some(&track("A", "B", PlaybackState::Paused)));
        assert_eq!(files.line, "A — B");
    }

    #[test]
    fn a_track_with_no_artist_is_just_the_title() {
        let files = render(Some(&track("   ", "Untitled", PlaybackState::Playing)));
        assert_eq!(files.line, "Untitled");
    }

    #[test]
    fn a_track_with_no_title_publishes_nothing() {
        // Sources emit empty metadata mid-transition, and an overlay reading
        // just a dash on someone's stream is not an improvement on nothing.
        assert_eq!(render(Some(&track("Someone", "  ", PlaybackState::Playing))), Files::default());
    }

    #[test]
    fn html_special_characters_cannot_break_the_page() {
        let files = render(Some(&track("AC/DC & \"Friends\"", "<Thunder>", PlaybackState::Playing)));
        assert!(files.html.contains("&lt;Thunder&gt;"), "{}", files.html);
        assert!(files.html.contains("&amp;"), "{}", files.html);
        assert!(!files.html.contains("<Thunder>"));
    }

    #[test]
    fn a_cover_survives_the_round_trip_out_of_its_data_url() {
        let bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0x01, 0x02];
        let uri = format!(
            "data:image/jpeg;base64,{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
        );
        assert_eq!(cover_bytes(Some(&uri)), Some(bytes));
    }

    #[test]
    fn a_missing_or_malformed_cover_is_simply_absent() {
        assert_eq!(cover_bytes(None), None);
        assert_eq!(cover_bytes(Some("not a data url")), None);
        assert_eq!(cover_bytes(Some("data:image/jpeg;base64,!!!not base64!!!")), None);
    }
}
