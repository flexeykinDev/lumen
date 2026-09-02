//! Finding a *URL* for the cover Discord is going to draw.
//!
//! Discord renders a presence image two ways: an asset key uploaded to the
//! application, or an https URL. SMTC gives us the cover as bytes and nothing
//! else — there is no URL anywhere in the media session — so showing the real
//! artwork means finding the same image somewhere public.
//!
//! Apple's iTunes Search endpoint is the one that fits: no key, no account, no
//! terms to accept, and it covers essentially all commercially released music.
//! What it costs is a request carrying the artist and title, which is why the
//! whole feature is off until switched on and says so where it is switched on.
//!
//! Lookups are cached, including the failures — a track that has no match must
//! not send a request every time its timeline ticks.

use std::collections::HashMap;

use crate::net;

/// Artwork size to ask for. iTunes serves `100x100` in the response and any
/// size by substitution; 512 is the largest Discord will draw before scaling.
const ART_SIZE: &str = "512x512";

/// Cache ceiling. A long listening session is a few hundred tracks at most, and
/// each entry is two short strings.
const MAX_ENTRIES: usize = 256;

/// Remembers what has already been looked up, misses included.
#[derive(Default)]
pub struct Covers {
    seen: HashMap<String, Option<String>>,
}

impl Covers {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cover URL for a track, or `None` when there is no match.
    ///
    /// Blocking, and deliberately called only when the track identity changes:
    /// the presence actor is a dedicated thread whose only other job is a
    /// four-second rate limit, so a slow search delays nothing that matters.
    pub fn url_for(&mut self, artist: &str, title: &str) -> Option<String> {
        let artist = artist.trim();
        let title = title.trim();
        if title.is_empty() {
            return None;
        }

        let key = format!("{}\u{1}{}", artist.to_lowercase(), title.to_lowercase());
        if let Some(hit) = self.seen.get(&key) {
            return hit.clone();
        }

        let found = match search(artist, title) {
            Ok(url) => url,
            Err(e) => {
                // A failed lookup is cached like a miss. Retrying on every
                // timeline tick would turn one dead network into a flood.
                tracing::debug!("cover lookup failed for {artist} — {title}: {e}");
                None
            }
        };

        if self.seen.len() >= MAX_ENTRIES {
            self.seen.clear();
        }
        self.seen.insert(key, found.clone());
        found
    }
}

fn search(artist: &str, title: &str) -> anyhow::Result<Option<String>> {
    let term = net::encode(format!("{artist} {title}").trim());
    let path = format!("/search?term={term}&entity=song&limit=1&media=music");
    let Some(body) = net::get("itunes.apple.com", &path)? else {
        return Ok(None);
    };

    let json: serde_json::Value = serde_json::from_slice(&body)?;
    let raw = json
        .get("results")
        .and_then(|r| r.get(0))
        .and_then(|r| r.get("artworkUrl100"))
        .and_then(|u| u.as_str());

    Ok(raw.map(upscale))
}

/// Ask for the large artwork instead of the thumbnail the API returns.
///
/// The URL ends `.../100x100bb.jpg`, and the size is a path segment the CDN
/// honours for any value — so this is a substitution, not a resize.
fn upscale(url: &str) -> String {
    url.replace("100x100", ART_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_the_large_artwork() {
        let thumb = "https://is1-ssl.mzstatic.com/image/thumb/abc/100x100bb.jpg";
        assert_eq!(
            upscale(thumb),
            "https://is1-ssl.mzstatic.com/image/thumb/abc/512x512bb.jpg"
        );
    }

    /// A URL that does not carry the size is passed through rather than mangled.
    #[test]
    fn leaves_an_unexpected_url_alone() {
        let odd = "https://example.test/cover.png";
        assert_eq!(upscale(odd), odd);
    }

    /// Nothing to search for is not worth a request.
    #[test]
    fn an_empty_title_never_reaches_the_network() {
        let mut covers = Covers::new();
        assert_eq!(covers.url_for("Someone", "   "), None);
        assert!(covers.seen.is_empty(), "a skipped lookup must not be cached");
    }

    /// Live. Ignored by default: it needs the network and Apple's endpoint.
    #[test]
    #[ignore = "network"]
    fn finds_a_well_known_cover() {
        let mut covers = Covers::new();
        let url = covers.url_for("The Weeknd", "Blinding Lights").expect("a cover");
        assert!(url.starts_with("https://"), "{url}");
        assert!(url.contains(ART_SIZE), "{url}");

        // Second call must be served from the cache, not the network.
        let again = covers.url_for("the weeknd", "BLINDING LIGHTS");
        assert_eq!(again.as_deref(), Some(url.as_str()));
    }
}
