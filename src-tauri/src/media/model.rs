//! Wire types shared with the renderer. Mirrored by `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

use crate::color::Accent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    /// SMTC reports a transitional state while a session swaps tracks. We keep it
    /// distinct so the UI can hold the previous frame instead of flashing empty.
    Changing,
}

impl PlaybackState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Playing | Self::Changing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub position_sec: f64,
    /// 0 means unknown — live streams report no end time.
    pub duration_sec: f64,
    /// Host monotonic clock (ms since process start) at the moment of sampling.
    /// The renderer interpolates from this instead of receiving ticks.
    pub updated_at_ms: f64,
}

impl Default for Timeline {
    fn default() -> Self {
        Self { position_sec: 0.0, duration_sec: 0.0, updated_at_ms: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub source: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub session_id: String,
    /// Human-readable app name derived from the AUMID, e.g. "Spotify".
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub state: PlaybackState,
    pub timeline: Timeline,
    /// `data:image/png;base64,…`, or None when the session exposes no thumbnail.
    pub art_data_uri: Option<String>,
    pub accent: Option<Accent>,
    /// Bumped on every track identity change; the renderer keys its crossfade on it.
    pub revision: u64,
}

impl NowPlaying {
    /// Identity of the *track*, not of the session or the playback position.
    /// Used to decide whether to re-fetch artwork and re-extract the accent.
    pub fn identity(&self) -> (&str, &str, &str) {
        (self.title.as_str(), self.artist.as_str(), self.album.as_str())
    }

    pub fn has_content(&self) -> bool {
        !self.title.trim().is_empty() || !self.artist.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportCmd {
    PlayPause,
    Next,
    Previous,
    /// Absolute position in seconds from the start of the track.
    Seek(f64),
    /// Step the player's repeat mode: none, whole list, one track, and back.
    CycleRepeat,
}

#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// The set of sessions changed (Phase 2 session switcher listens to this).
    SessionsChanged(Vec<SessionSummary>),
    /// A new track, or the first track from a newly focused session.
    TrackChanged(Box<NowPlaying>),
    /// Play/pause/stop transition on the current track.
    PlaybackChanged(Box<NowPlaying>),
    /// A seek or a periodic timeline refresh.
    TimelineChanged(Box<NowPlaying>),
    /// No session is publishing to SMTC any more.
    Vanished,
}

/// AUMIDs that carry no readable name at all.
///
/// Some desktop apps register an opaque hash as their AUMID rather than a path
/// or a package id. There is nothing to derive a name from, so the well-known
/// ones are mapped by hand and the rest degrade to a neutral label — showing
/// `308046B0AF4A39CB` in the UI is worse than showing nothing.
const OPAQUE_AUMIDS: &[(&str, &str)] = &[
    ("308046B0AF4A39CB", "Firefox"),
    ("6F193CCC56814779", "Firefox"),
    ("E7CF176E110C211B", "Firefox"),
];

/// True when the AUMID is a bare hex blob with no structure to read.
fn is_opaque(s: &str) -> bool {
    s.len() >= 12 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Turn an AUMID into something worth showing a human.
///
/// Packaged apps report `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`; desktop
/// apps report `Spotify.exe` or a full path; a few report an opaque hash. We
/// want a readable name from all four.
pub fn pretty_source(aumid: &str) -> String {
    let s = aumid.trim();
    if s.is_empty() {
        return "Unknown".into();
    }

    if is_opaque(s) {
        let upper = s.to_ascii_uppercase();
        return OPAQUE_AUMIDS
            .iter()
            .find(|(id, _)| *id == upper)
            .map_or_else(|| "Media".to_owned(), |(_, name)| (*name).to_owned());
    }

    // Packaged: the segment after `!` is the app id chosen by the developer.
    let s = s.rsplit('!').next().unwrap_or(s);
    // Desktop: strip any path and the extension.
    let s = s.rsplit(['\\', '/']).next().unwrap_or(s);
    let s = s.strip_suffix(".exe").or_else(|| s.strip_suffix(".EXE")).unwrap_or(s);
    // Packaged family names look like `Publisher.AppName_hash`.
    let s = s.split('_').next().unwrap_or(s);

    // Take the last dot-segment that is actually a name.
    //
    // Simply taking the last segment is wrong: Telegram registers
    // `Telegram.TelegramDesktop.6247bcad5fc8ef719013eb38a24ef630`, whose final
    // segment is a hash — which is how "6247BCADS…" ended up in the UI. Skipping
    // hash-looking trailing segments yields "TelegramDesktop", and the
    // PascalCase split below turns that into "Telegram Desktop".
    //
    // `unwrap_or(s)` matters for the degenerate case where *every* segment looks
    // like a hash (a real app literally named "deadbeefcafe"): better to show
    // the odd name than nothing.
    let s = s.rsplit('.').find(|seg| !seg.is_empty() && !is_opaque(seg)).unwrap_or(s);

    if s.is_empty() {
        return "Unknown".into();
    }

    // Split PascalCase into words so `SpotifyMusic` reads as `Spotify Music`.
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.char_indices() {
        let prev_lower = i > 0 && s[..i].chars().next_back().is_some_and(|c| c.is_lowercase());
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        }
        out.push(ch);
    }

    trim_generic_suffix(out)
}

/// Drop a trailing word that carries no brand meaning.
///
/// `TelegramDesktop` becomes "Telegram Desktop" above, but the badge has room
/// for one word and "Desktop" is not the part that identifies the app. Only
/// genuinely generic words are removed — "Music" stays, because "Zune Music" is
/// the product's actual name and "Zune" alone reads as a dead brand.
pub(crate) fn trim_generic_suffix(name: String) -> String {
    const GENERIC: &[&str] = &["Desktop", "App", "Client", "Player", "UWP", "Beta"];

    if let Some((head, tail)) = name.rsplit_once(' ')
        && !head.is_empty()
        && GENERIC.iter().any(|g| g.eq_ignore_ascii_case(tail))
    {
        return head.to_owned();
    }
    name
}

#[cfg(test)]
mod tests {
    use super::pretty_source;

    #[test]
    fn prettifies_the_three_aumid_shapes() {
        assert_eq!(pretty_source("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(pretty_source("Spotify.exe"), "Spotify");
        assert_eq!(pretty_source(r"C:\Program Files\Foobar2000\foobar2000.exe"), "foobar2000");
        assert_eq!(pretty_source("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Zune Music");
        assert_eq!(pretty_source(""), "Unknown");
    }

    /// Seen live: Firefox publishes a bare hash, and the raw value was reaching
    /// the source badge as "308046B0A…".
    #[test]
    fn maps_opaque_hash_aumids_instead_of_showing_hex() {
        assert_eq!(pretty_source("308046B0AF4A39CB"), "Firefox");
        assert_eq!(pretty_source("308046b0af4a39cb"), "Firefox");
        // Unknown hashes must degrade to a label, never leak hex into the UI.
        assert_eq!(pretty_source("A1B2C3D4E5F60718"), "Media");
    }

    /// Seen live: the badge rendered "6247BCADS…" because the AUMID's final
    /// dot-segment is a hash rather than the app name.
    #[test]
    fn strips_trailing_hash_segments_from_dotted_aumids() {
        // "Desktop" is dropped: the badge has room for the brand, not the
        // edition.
        assert_eq!(
            pretty_source("Telegram.TelegramDesktop.6247bcad5fc8ef719013eb38a24ef630"),
            "Telegram"
        );
        // The publisher-qualified form must still resolve to the app, not the
        // publisher — and "Music" is part of the name here, so it stays.
        assert_eq!(pretty_source("Microsoft.ZuneMusic"), "Zune Music");
        assert_eq!(pretty_source("Some.CoolThingApp"), "Cool Thing");
    }

    #[test]
    fn does_not_mistake_real_names_for_hashes() {
        // Every character here is a hex digit, but it is a real app name.
        assert_eq!(pretty_source("deadbeefcafe.exe"), "deadbeefcafe");
        assert_eq!(pretty_source("Adobe.exe"), "Adobe");
    }
}
