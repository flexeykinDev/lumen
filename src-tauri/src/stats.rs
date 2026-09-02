//! What you actually listened to.
//!
//! A play counter, kept in `lumen.stats.json` beside the config. Nothing here
//! leaves the machine: there is no account, no sync and no upload, and the file
//! is plain JSON you can read, edit or delete.
//!
//! # What counts as a play
//!
//! Not "a track appeared". Skipping through a playlist would otherwise credit
//! every track you rejected, and the top of the list would be whatever your
//! shuffle happened to touch. A play is counted once the track has been
//! *playing* for [`PLAY_SECONDS`], or for half its length if it is shorter than
//! that — the same rule scrobblers have used for twenty years, because it is
//! the one that matches what people mean.
//!
//! # Time, not timestamps
//!
//! Listening time is accumulated from the wall clock between events while the
//! state is playing, rather than from the source's own position. A browser that
//! reports a frozen or resetting timeline (see `media::smtc`) would otherwise
//! produce nonsense totals, and the whole point of this file is that its numbers
//! can be trusted.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::media::{NowPlaying, PlaybackState};

/// How long a track must play before it counts.
pub const PLAY_SECONDS: f64 = 30.0;

/// The most tracks kept. Well beyond a decade of listening, and a bound is what
/// stops a file that is only ever appended to from growing without limit.
const MAX_TRACKS: usize = 5000;

const FILE_NAME: &str = "lumen.stats.json";

/// One track's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub album: String,
    /// The player it was last heard through.
    #[serde(default)]
    pub source: String,
    pub plays: u32,
    /// Total listening time, in seconds. Counts the parts you actually heard,
    /// so a track played half way twice is a minute short of two plays.
    pub seconds: f64,
    /// Unix seconds, for "first heard" and "last heard".
    pub first_at: u64,
    pub last_at: u64,
}

/// The totals, for the line above the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub plays: u32,
    pub seconds: f64,
    pub tracks: usize,
    pub artists: usize,
    /// When the first play was recorded, or `None` on an empty history.
    pub since: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    tracks: Vec<Track>,
}

/// Identity of a track, for counting purposes.
///
/// Artist and title, case-folded. Not the album: the same song from a single
/// and from an album is the same song, and a source that reports one of them
/// inconsistently would otherwise split its own count in two.
fn key(artist: &str, title: &str) -> (String, String) {
    (artist.trim().to_lowercase(), title.trim().to_lowercase())
}

/// Whether a stretch of listening is long enough to count as a play.
///
/// `duration` of zero means the source did not say (a live stream), in which
/// case only the absolute threshold applies.
pub fn counts_as_play(listened: f64, duration: f64) -> bool {
    if listened <= 0.0 {
        return false;
    }
    let bar = if duration > 0.0 { PLAY_SECONDS.min(duration / 2.0) } else { PLAY_SECONDS };
    listened >= bar
}

/// Sort by plays, then by time, then alphabetically.
///
/// The last key is what keeps the list stable: two tracks with one play each
/// must not swap places every time the window is opened.
fn rank(a: &Track, b: &Track) -> std::cmp::Ordering {
    b.plays
        .cmp(&a.plays)
        .then(b.seconds.total_cmp(&a.seconds))
        .then(a.artist.cmp(&b.artist))
        .then(a.title.cmp(&b.title))
}

/// Merge one finished listen into a history.
fn record(tracks: &mut Vec<Track>, np: &NowPlaying, listened: f64, now: u64) {
    let (artist, title) = key(&np.artist, &np.title);
    if title.is_empty() {
        return;
    }

    let identity = (artist, title);
    if let Some(existing) = tracks.iter_mut().find(|t| key(&t.artist, &t.title) == identity) {
        existing.plays += 1;
        existing.seconds += listened;
        existing.last_at = now;
        // The display form follows the most recent report: sources correct
        // their own metadata, and the newer spelling is usually the better one.
        existing.artist = np.artist.trim().to_owned();
        existing.title = np.title.trim().to_owned();
        if !np.album.trim().is_empty() {
            existing.album = np.album.trim().to_owned();
        }
        existing.source = np.source.clone();
        return;
    }

    tracks.push(Track {
        artist: np.artist.trim().to_owned(),
        title: np.title.trim().to_owned(),
        album: np.album.trim().to_owned(),
        source: np.source.clone(),
        plays: 1,
        seconds: listened,
        first_at: now,
        last_at: now,
    });

    // Evict the least interesting rather than the oldest: something played once
    // last year is a better candidate than something played fifty times.
    if tracks.len() > MAX_TRACKS {
        tracks.sort_by(rank);
        tracks.truncate(MAX_TRACKS);
    }
}

/// Wall-clock listening, one track at a time.
struct Current {
    track: NowPlaying,
    /// When the current *playing* stretch started, or `None` while paused.
    since: Option<Instant>,
    /// Seconds accumulated across previous stretches of this same track.
    banked: f64,
    /// Already counted, so a long listen does not count twice.
    credited: bool,
}

/// The recorder. One per process, behind the usual lock.
pub struct Stats {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    store: Store,
    current: Option<Current>,
    /// Set when something has changed but has not been written yet.
    dirty: bool,
}

impl Stats {
    /// Load whatever history exists beside the config.
    pub fn load(config_path: Option<&Path>) -> Self {
        let path = config_path.and_then(|p| p.parent()).map(|dir| dir.join(FILE_NAME));

        let store = path
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| match serde_json::from_str::<Store>(raw.trim_start_matches('\u{feff}')) {
                Ok(store) => Some(store),
                Err(e) => {
                    // A corrupt history is not worth failing a launch over, and
                    // it is not worth silently deleting either.
                    tracing::warn!("listening history could not be read ({e}); starting fresh");
                    None
                }
            })
            .unwrap_or_default();

        tracing::info!("listening history: {} tracks", store.tracks.len());
        Self { path, inner: Mutex::new(Inner { store, current: None, dirty: false }) }
    }

    /// Feed the recorder whatever the media backend just reported.
    pub fn observe(&self, np: Option<&NowPlaying>) {
        let Ok(mut inner) = self.inner.lock() else { return };
        let now = unix_now();

        // Bank the time spent on whatever was playing until this moment.
        if let Some(current) = inner.current.as_mut()
            && let Some(since) = current.since.take()
        {
            current.banked += since.elapsed().as_secs_f64();
        }

        let same = matches!((inner.current.as_ref(), np), (Some(c), Some(n))
            if key(&c.track.artist, &c.track.title) == key(&n.artist, &n.title));

        if !same {
            // A different track (or none): finish the old one first.
            if let Some(previous) = inner.current.take() {
                Self::finish(&mut inner, previous, now);
            }
            if let Some(np) = np {
                inner.current = Some(Current {
                    track: np.clone(),
                    since: None,
                    banked: 0.0,
                    credited: false,
                });
            }
        }

        // Restart the clock if it is playing now.
        if let (Some(current), Some(np)) = (inner.current.as_mut(), np) {
            current.track = np.clone();
            if np.state == PlaybackState::Playing {
                current.since = Some(Instant::now());
            }
        }

        // Credit as soon as the threshold is crossed rather than at the end:
        // a track left playing when the machine is switched off still counts.
        let ready = inner.current.as_ref().is_some_and(|c| {
            !c.credited && counts_as_play(c.banked, c.track.timeline.duration_sec)
        });
        if ready {
            let (track, banked) = {
                let current = inner.current.as_mut().expect("checked above");
                current.credited = true;
                (current.track.clone(), current.banked)
            };
            record(&mut inner.store.tracks, &track, banked, now);
            inner.dirty = true;
            self.save_locked(&inner);
        }
    }

    /// Close out a track that is no longer playing.
    fn finish(inner: &mut Inner, current: Current, now: u64) {
        if current.credited {
            // Already counted; add the rest of the time it was heard for.
            let identity = key(&current.track.artist, &current.track.title);
            if let Some(existing) =
                inner.store.tracks.iter_mut().find(|t| key(&t.artist, &t.title) == identity)
            {
                let extra = current.banked - existing.seconds.min(current.banked);
                existing.seconds += extra.max(0.0);
                existing.last_at = now;
                inner.dirty = true;
            }
        } else if counts_as_play(current.banked, current.track.timeline.duration_sec) {
            record(&mut inner.store.tracks, &current.track, current.banked, now);
            inner.dirty = true;
        }
    }

    /// The `limit` most played tracks.
    pub fn top(&self, limit: usize) -> Vec<Track> {
        let Ok(inner) = self.inner.lock() else { return Vec::new() };
        let mut tracks = inner.store.tracks.clone();
        tracks.sort_by(rank);
        tracks.truncate(limit);
        tracks
    }

    /// The `limit` most played artists, as a name and a play count.
    pub fn top_artists(&self, limit: usize) -> Vec<(String, u32, f64)> {
        let Ok(inner) = self.inner.lock() else { return Vec::new() };
        let mut totals: HashMap<String, (String, u32, f64)> = HashMap::new();
        for track in &inner.store.tracks {
            if track.artist.trim().is_empty() {
                continue;
            }
            let entry = totals
                .entry(track.artist.trim().to_lowercase())
                .or_insert_with(|| (track.artist.trim().to_owned(), 0, 0.0));
            entry.1 += track.plays;
            entry.2 += track.seconds;
        }
        let mut artists: Vec<_> = totals.into_values().collect();
        artists.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.total_cmp(&a.2)).then(a.0.cmp(&b.0)));
        artists.truncate(limit);
        artists
    }

    pub fn summary(&self) -> Summary {
        let Ok(inner) = self.inner.lock() else {
            return Summary { plays: 0, seconds: 0.0, tracks: 0, artists: 0, since: None };
        };
        let tracks = &inner.store.tracks;
        let artists: std::collections::HashSet<String> =
            tracks.iter().map(|t| t.artist.trim().to_lowercase()).filter(|a| !a.is_empty()).collect();

        Summary {
            plays: tracks.iter().map(|t| t.plays).sum(),
            seconds: tracks.iter().map(|t| t.seconds).sum(),
            tracks: tracks.len(),
            artists: artists.len(),
            since: tracks.iter().map(|t| t.first_at).min(),
        }
    }

    /// Forget everything, including the file.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.store.tracks.clear();
            inner.current = None;
            inner.dirty = true;
            self.save_locked(&inner);
        }
    }

    /// Write the history out. Called when a play is credited, not on a timer.
    fn save_locked(&self, inner: &Inner) {
        let Some(path) = self.path.as_deref() else { return };
        let Ok(json) = serde_json::to_string_pretty(&inner.store) else { return };
        // Write-then-rename, so a crash mid-write cannot truncate the history.
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }

    /// Flush whatever the current track has earned. For shutdown.
    pub fn flush(&self) {
        let Ok(mut inner) = self.inner.lock() else { return };
        let now = unix_now();
        if let Some(mut current) = inner.current.take() {
            if let Some(since) = current.since.take() {
                current.banked += since.elapsed().as_secs_f64();
            }
            Self::finish(&mut inner, current, now);
        }
        if inner.dirty {
            self.save_locked(&inner);
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Timeline;

    fn track(artist: &str, title: &str, duration: f64) -> NowPlaying {
        NowPlaying {
            session_id: "test".into(),
            source: "Spotify".into(),
            title: title.into(),
            artist: artist.into(),
            album: "An Album".into(),
            state: PlaybackState::Playing,
            timeline: Timeline { position_sec: 0.0, duration_sec: duration, updated_at_ms: 0.0 },
            art_data_uri: None,
            accent: None,
            revision: 1,
        }
    }

    #[test]
    fn a_skipped_track_does_not_count() {
        // The whole reason for a threshold: three seconds of a song you skipped
        // must not outrank one you listened to.
        assert!(!counts_as_play(3.0, 210.0));
        assert!(!counts_as_play(29.9, 210.0));
        assert!(counts_as_play(30.0, 210.0));
    }

    #[test]
    fn a_short_track_counts_at_half_its_length() {
        // A 40-second interlude can never reach 30 seconds and half of it is a
        // fair listen, so the bar drops to 20.
        assert!(counts_as_play(20.0, 40.0));
        assert!(!counts_as_play(19.0, 40.0));
    }

    #[test]
    fn a_stream_with_no_duration_uses_the_absolute_bar() {
        assert!(!counts_as_play(20.0, 0.0));
        assert!(counts_as_play(31.0, 0.0));
    }

    #[test]
    fn nothing_counts_as_nothing() {
        assert!(!counts_as_play(0.0, 200.0));
        assert!(!counts_as_play(-5.0, 200.0));
    }

    #[test]
    fn the_same_song_merges_regardless_of_case_and_spacing() {
        let mut tracks = Vec::new();
        record(&mut tracks, &track("KYSLINGO", "The Law", 200.0), 60.0, 100);
        record(&mut tracks, &track("  kyslingo ", " the law  ", 200.0), 40.0, 200);

        assert_eq!(tracks.len(), 1, "the same song was counted twice: {tracks:?}");
        assert_eq!(tracks[0].plays, 2);
        assert_eq!(tracks[0].seconds, 100.0);
        assert_eq!(tracks[0].first_at, 100);
        assert_eq!(tracks[0].last_at, 200);
        // The display form follows the newest report, trimmed.
        assert_eq!(tracks[0].artist, "kyslingo");
    }

    #[test]
    fn a_track_with_no_title_is_not_recorded() {
        // Sources publish empty metadata during a transition; counting those
        // would produce a top entry called nothing.
        let mut tracks = Vec::new();
        record(&mut tracks, &track("Someone", "   ", 200.0), 60.0, 1);
        assert!(tracks.is_empty());
    }

    #[test]
    fn ranking_puts_the_most_played_first_and_is_stable() {
        let mut tracks = vec![
            Track { plays: 2, seconds: 100.0, ..sample("B", "b") },
            Track { plays: 9, seconds: 10.0, ..sample("C", "c") },
            Track { plays: 2, seconds: 400.0, ..sample("A", "a") },
        ];
        tracks.sort_by(rank);

        assert_eq!(tracks[0].title, "c", "most plays wins");
        assert_eq!(tracks[1].title, "a", "equal plays: more time wins");
        assert_eq!(tracks[2].title, "b");

        // Sorting again must not shuffle equal entries.
        let before = tracks.clone();
        tracks.sort_by(rank);
        assert_eq!(before, tracks);
    }

    #[test]
    fn the_history_is_bounded_and_keeps_the_most_played() {
        let mut tracks: Vec<Track> = (0..MAX_TRACKS)
            .map(|i| Track { plays: 5, ..sample("Artist", &format!("song {i}")) })
            .collect();
        // One more, played once: it is the entry that should be dropped.
        record(&mut tracks, &track("Newcomer", "one hit", 200.0), 60.0, 1);

        assert_eq!(tracks.len(), MAX_TRACKS);
        assert!(
            !tracks.iter().any(|t| t.title == "one hit"),
            "the least played entry should have been evicted"
        );
    }

    fn sample(artist: &str, title: &str) -> Track {
        Track {
            artist: artist.into(),
            title: title.into(),
            album: String::new(),
            source: "Spotify".into(),
            plays: 1,
            seconds: 1.0,
            first_at: 0,
            last_at: 0,
        }
    }
}
