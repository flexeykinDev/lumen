//! Deciding whether the thing playing is a *song*.
//!
//! Looking up lyrics for a gameplay video or a ninety-minute interview does not
//! politely fail — the databases match on artist and title, so a video called
//! "ПЯТЁРКА СМОТРИТ" finds *some* song and confidently scrolls its words over
//! unrelated audio. That is worse than showing nothing.
//!
//! # What is actually knowable
//!
//! SMTC gives an application id, a title, an artist, an album and a duration.
//! It does **not** say which website a browser is on, so "is this YouTube Music
//! or YouTube" cannot be answered directly — only inferred from the metadata the
//! page chose to publish. So:
//!
//! - a dedicated music player is trusted outright;
//! - a browser has to look like music before anything is looked up;
//! - anything else is trusted, because a media player playing something is the
//!   ordinary case and a false negative here costs only a missing lyric.
//!
//! The bias is deliberate. A missed lyric is a shrug; a confident lyric over the
//! wrong audio looks broken.

use crate::media::NowPlaying;

/// Applications that only ever play music. Matched case-insensitively against
/// the friendly source name, as a substring, so "Yandex Music" catches its
/// several editions.
const MUSIC_APPS: &[&str] = &[
    "spotify",
    "yandex music",
    "яндекс музыка",
    "apple music",
    "itunes",
    "tidal",
    "deezer",
    "qobuz",
    "amazon music",
    "youtube music",
    "foobar2000",
    "aimp",
    "musicbee",
    "winamp",
    "groove",
    "vk музыка",
    "vk music",
    "zvuk",
];

/// Browsers, where the same application plays songs and everything else.
const BROWSERS: &[&str] =
    &["firefox", "chrome", "chromium", "edge", "opera", "brave", "vivaldi", "zen", "yandex"];

/// Words that appear in the titles of things that are not songs.
///
/// Deliberately drawn from both languages this machine plays: the failure that
/// prompted this was a Russian interview video, and an English-only list would
/// have missed it entirely.
const NOT_MUSIC: &[&str] = &[
    "podcast",
    "подкаст",
    "interview",
    "интервью",
    "обзор",
    "review",
    "stream",
    "стрим",
    "gameplay",
    "прохождение",
    "летсплей",
    "let's play",
    "lets play",
    "vlog",
    "влог",
    "tutorial",
    "туториал",
    "лекция",
    "лекции",
    "смотрит",
    "реакция",
    "reaction",
    "разбор",
    "episode",
    "эпизод",
    "серия",
    "trailer",
    "трейлер",
    "выпуск",
    "новости",
    "news",
];

/// Shortest plausible song. Below this it is a clip, a sting or an advert.
const MIN_SONG_SEC: f64 = 40.0;
/// Longest plausible song. Above this it is a mix, a set, or long-form video.
const MAX_SONG_SEC: f64 = 12.0 * 60.0;

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    let lower = haystack.to_lowercase();
    needles.iter().any(|n| lower.contains(n))
}

/// Whether it is worth looking up lyrics for this.
pub fn is_song(np: &NowPlaying) -> bool {
    // A dedicated music player is playing music. Nothing more to establish.
    if contains_any(&np.source, MUSIC_APPS) {
        return true;
    }

    if !contains_any(&np.source, BROWSERS) {
        // Some other media application. Trusted, because the browser case is
        // the one that actually goes wrong.
        return true;
    }

    // --- a browser, so it has to earn it ------------------------------------

    // Long-form video says so in its title far more often than not.
    if contains_any(&np.title, NOT_MUSIC) || contains_any(&np.artist, NOT_MUSIC) {
        return false;
    }

    // No artist at all is the signature of a page that published nothing but a
    // video title.
    if np.artist.trim().is_empty() {
        return false;
    }

    // Duration is the single strongest signal. An interview runs an hour; a
    // song almost never runs past twelve minutes or under forty seconds.
    //
    // A source reporting no duration is rejected rather than guessed at: with
    // no length there is nothing to estimate timings against either, so a lyric
    // would be unusable even if it were correct.
    let seconds = np.timeline.duration_sec;
    if !(seconds.is_finite() && (MIN_SONG_SEC..=MAX_SONG_SEC).contains(&seconds)) {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{PlaybackState, Timeline};

    fn track(source: &str, title: &str, artist: &str, duration: f64) -> NowPlaying {
        NowPlaying {
            session_id: "s".into(),
            source: source.into(),
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            state: PlaybackState::Playing,
            timeline: Timeline { position_sec: 0.0, duration_sec: duration, updated_at_ms: 0.0 },
            art_data_uri: None,
            accent: None,
            revision: 1,
        }
    }

    #[test]
    fn music_apps_are_always_songs() {
        // Even an odd duration: Spotify is not playing an interview.
        assert!(is_song(&track("Spotify", "Hate.", "ThxSoMch", 134.0)));
        assert!(is_song(&track("Yandex Music", "трек", "артист", 3000.0)));
        assert!(is_song(&track("YouTube Music", "Song", "Artist", 200.0)));
    }

    /// The case that prompted this: a long interview in a browser, matched
    /// against a real song and scrolled confidently over it.
    #[test]
    fn long_form_video_in_a_browser_is_not_a_song() {
        assert!(!is_song(&track(
            "Firefox",
            "Моргенштерн – рехаб, Лиза, новое имя, новая жизнь",
            "вДудь",
            4921.0,
        )));
    }

    #[test]
    fn title_keywords_reject_in_either_language() {
        assert!(!is_song(&track("Firefox", "ПЯТЁРКА СМОТРИТ: ТУПЫЕ МОМЕНТЫ", "Канал", 300.0)));
        assert!(!is_song(&track("Chrome", "Elden Ring gameplay part 3", "Streamer", 300.0)));
        assert!(!is_song(&track("Firefox", "Подкаст о музыке", "Студия", 300.0)));
    }

    /// A music video in a browser is still a song, and should get lyrics.
    #[test]
    fn a_music_video_in_a_browser_is_a_song() {
        assert!(is_song(&track("Firefox", "Blinding Lights", "The Weeknd", 201.0)));
        assert!(is_song(&track("Edge", "ДИНАСТИЯ", "VILLIAN", 190.0)));
    }

    #[test]
    fn browser_durations_outside_song_range_are_rejected() {
        assert!(!is_song(&track("Firefox", "Some Song", "Some Artist", 20.0)), "too short");
        assert!(!is_song(&track("Firefox", "Some Song", "Some Artist", 3600.0)), "too long");
    }

    /// A page that published only a video title, with no artist, is not
    /// something to look words up for.
    #[test]
    fn browser_without_an_artist_is_rejected() {
        assert!(!is_song(&track("Firefox", "some video title", "", 200.0)));
    }

    /// No duration means no timings could be estimated even if the words were
    /// right, so there is nothing to gain by looking.
    #[test]
    fn browser_without_a_duration_is_rejected() {
        assert!(!is_song(&track("Firefox", "Song", "Artist", 0.0)));
        assert!(!is_song(&track("Firefox", "Song", "Artist", f64::NAN)));
    }

    /// A local player is trusted: the browser is the ambiguous case, and a
    /// false negative here only costs a missing lyric.
    #[test]
    fn unknown_media_players_are_trusted() {
        assert!(is_song(&track("foobar2000", "Track", "Artist", 200.0)));
        assert!(is_song(&track("Some Player", "Track", "Artist", 200.0)));
    }
}
