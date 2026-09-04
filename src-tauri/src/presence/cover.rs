//! Finding a *URL* for the cover Discord is going to draw.
//!
//! Discord renders a presence image two ways: an asset key uploaded to the
//! application, or an https URL. SMTC gives us the cover as bytes and nothing
//! else — there is no URL anywhere in the media session — so showing the real
//! artwork means finding the same image somewhere public.
//!
//! Two providers are tried in order, because one of them is not reachable
//! everywhere. Apple's iTunes Search needs no key and covers essentially all
//! commercially released music, but Apple's endpoints are blocked or
//! unreliable from parts of the world — Russia among them — and a blocked
//! lookup there means no artwork at all. Deezer's public search is the
//! fallback: also keyless, better on Russian and CIS catalogues, and reachable
//! where Apple is not.
//!
//! What either costs is a request carrying the artist and title, which is why
//! the whole feature is off until switched on and says so where it is switched
//! on.
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

        let found = lookup(artist, title);

        if self.seen.len() >= MAX_ENTRIES {
            self.seen.clear();
        }
        self.seen.insert(key, found.clone());
        found
    }
}

/// Try each provider until one answers.
///
/// A provider that errors is not a provider that has no match: one is a network
/// or a block, the other is an answer. Both end the same way for the caller, but
/// only the first is worth trying the next provider for — and trying it anyway
/// on a miss would double every lookup for tracks nobody has artwork for.
fn lookup(artist: &str, title: &str) -> Option<String> {
    for (name, provider) in PROVIDERS {
        match provider(artist, title) {
            Ok(Some(url)) => {
                tracing::debug!("cover: {name} matched {artist} — {title}");
                return Some(url);
            }
            Ok(None) => continue,
            Err(e) => {
                // The interesting case: blocked, filtered or simply down. Say
                // so once at debug and move to the next provider.
                tracing::debug!("cover: {name} unreachable for {artist} — {title}: {e}");
                continue;
            }
        }
    }
    None
}

/// In order of coverage, then of reachability.
type Provider = fn(&str, &str) -> anyhow::Result<Option<String>>;
const PROVIDERS: [(&str, Provider); 2] = [("itunes", search_itunes), ("deezer", search_deezer)];

fn search_itunes(artist: &str, title: &str) -> anyhow::Result<Option<String>> {
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

/// Deezer's public search. No key, no account, and reachable from places
/// Apple's endpoints are not.
///
/// `cover_xl` is 1000px; Discord scales it down, and the alternative is asking
/// for `cover_medium` and having it upscaled instead.
fn search_deezer(artist: &str, title: &str) -> anyhow::Result<Option<String>> {
    let term = net::encode(format!("{artist} {title}").trim());
    let path = format!("/search?q={term}&limit=1");
    let Some(body) = net::get("api.deezer.com", &path)? else {
        return Ok(None);
    };

    let json: serde_json::Value = serde_json::from_slice(&body)?;
    let album = json.get("data").and_then(|d| d.get(0)).and_then(|t| t.get("album"));

    Ok(album
        .and_then(|a| a.get("cover_xl").or_else(|| a.get("cover_big")))
        .and_then(|u| u.as_str())
        .map(str::to_owned))
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

    /// Both providers are tried, in a defined order.
    ///
    /// The order is the point: iTunes has the broader catalogue, Deezer is
    /// reachable where iTunes is blocked, and a lookup that stopped at the
    /// first provider is exactly the bug this list exists to fix.
    #[test]
    fn there_is_a_fallback_provider_after_apple() {
        assert_eq!(PROVIDERS.len(), 2);
        assert_eq!(PROVIDERS[0].0, "itunes");
        assert_eq!(PROVIDERS[1].0, "deezer");
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
