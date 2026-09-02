//! Discord Rich Presence.
//!
//! # Transport
//!
//! Discord's local IPC is a named pipe, `\\.\pipe\discord-ipc-N` for N in 0..9 —
//! several exist when more than one Discord build is installed. Frames are a
//! 4-byte little-endian opcode, a 4-byte little-endian length, then JSON.
//!
//! Deliberately no crate for this. The whole protocol is a handshake and one
//! command, and the portable-exe budget is 10 MB; `discord-rich-presence` pulls
//! in a good deal more than 200 lines is worth.
//!
//! # Album art
//!
//! Discord shows an image only by asset key (uploaded to the application) or by
//! URL. SMTC hands over the cover as *bytes* — there is no URL anywhere — so the
//! real cover can only be shown by finding the same artwork somewhere public;
//! see [`cover`]. That is a network request per track, so it is off until asked
//! for, and `large_image` otherwise carries the application's own asset.
//!
//! # Rate limits
//!
//! Discord accepts roughly five activity updates per twenty seconds and silently
//! drops the rest. SMTC, meanwhile, republishes a timeline every second or two.
//! So updates are both de-duplicated by content and floored to one every
//! `MIN_INTERVAL` — without that the presence would spend most of its life
//! rate-limited and showing something stale.

use std::io::{Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::config::Discord;
use crate::media::{NowPlaying, PlaybackState};

pub mod cover;

/// Discord's own floor is ~5 per 20s; this leaves headroom for a burst.
const MIN_INTERVAL: Duration = Duration::from_secs(4);
/// How long to wait before trying the pipe again after a failure.
const RECONNECT_DELAY: Duration = Duration::from_secs(15);

/// Asset key for the image shown beside the track. Uploaded under
/// Rich Presence → Art Assets in the Discord developer portal.
const LARGE_ASSET: &str = "lumen";

/// Frame opcodes. `HANDSHAKE` and `FRAME` are sent; `CLOSE` is how Discord
/// reports a refusal, which is why the opcode has to be read back.
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

#[derive(Debug)]
enum Cmd {
    Update(Box<Option<NowPlaying>>),
    Shutdown,
}

pub struct Presence {
    tx: Sender<Cmd>,
}

impl Presence {
    /// Start the presence actor.
    ///
    /// Takes the whole Discord config rather than just the id: what a presence
    /// shows is entirely a matter of preference, and every one of those switches
    /// is read while building the payload.
    pub fn start(opts: Discord) -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();

        let app_id = opts.application_id.trim().to_owned();
        let _ = std::thread::Builder::new().name("lumen-presence".into()).spawn(move || {
            let mut conn: Option<Connection> = None;
            let mut covers = cover::Covers::new();
            let mut last_sent: Option<Instant> = None;
            let mut last_payload: Option<String> = None;
            // Held when an update arrives inside the rate-limit floor, so the
            // most recent state is sent once the floor lifts rather than lost.
            let mut pending: Option<Option<NowPlaying>> = None;
            let mut next_retry = Instant::now();

            loop {
                // Wait only as long as a deferred update needs; otherwise block.
                let wait = match (&pending, last_sent) {
                    (Some(_), Some(at)) => MIN_INTERVAL.saturating_sub(at.elapsed()),
                    (Some(_), None) => Duration::ZERO,
                    (None, _) => Duration::from_secs(3600),
                };

                match rx.recv_timeout(wait) {
                    Ok(Cmd::Shutdown) => break,
                    Ok(Cmd::Update(np)) => pending = Some(*np),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }

                let Some(state) = pending.clone() else { continue };

                // Still inside the floor: leave it pending and come back.
                if last_sent.is_some_and(|at| at.elapsed() < MIN_INTERVAL) {
                    continue;
                }

                // Cached per track, so this is a network call only on a change
                // and never while a timeline ticks.
                let art = match (opts.album_art, state.as_ref()) {
                    (true, Some(np)) => covers.url_for(&np.artist, &np.title),
                    _ => None,
                };

                let payload = activity_payload(state.as_ref(), &opts, art.as_deref());
                if last_payload.as_deref() == Some(payload.as_str()) {
                    // Nothing a viewer could see has changed.
                    pending = None;
                    continue;
                }

                if conn.is_none() {
                    if Instant::now() < next_retry {
                        continue;
                    }
                    match Connection::open(&app_id) {
                        Ok(c) => {
                            tracing::info!("Discord rich presence connected");
                            conn = Some(c);
                        }
                        Err(e) => {
                            tracing::debug!("Discord unavailable: {e}");
                            next_retry = Instant::now() + RECONNECT_DELAY;
                            continue;
                        }
                    }
                }

                if let Some(c) = conn.as_mut() {
                    if let Err(e) = c.set_activity(&payload) {
                        // Discord closing drops the pipe; reconnect next time.
                        tracing::debug!("Discord presence write failed: {e}");
                        conn = None;
                        last_payload = None;
                        next_retry = Instant::now() + RECONNECT_DELAY;
                        continue;
                    }
                    last_sent = Some(Instant::now());
                    last_payload = Some(payload);
                    pending = None;
                }
            }

            // Leave nothing behind: a stale "listening to" that outlives the app
            // is worse than no presence at all.
            if let Some(c) = conn.as_mut() {
                let _ = c.set_activity(&activity_payload(None, &opts, None));
            }
        });

        Self { tx }
    }

    /// Publish what is playing, or `None` to clear the presence.
    pub fn update(&self, now: Option<NowPlaying>) {
        let _ = self.tx.send(Cmd::Update(Box::new(now)));
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

/// The `args` object of a SET_ACTIVITY command, as a JSON string.
///
/// A string rather than a value so it doubles as the de-duplication key: if the
/// bytes are identical there is nothing to tell Discord.
fn activity_payload(now: Option<&NowPlaying>, opts: &Discord, art: Option<&str>) -> String {
    let Some(np) = now else {
        // Clearing is `activity: null`, not an empty object.
        return json!({ "pid": std::process::id(), "activity": null }).to_string();
    };

    // Discord draws the artwork by URL when it has one, and falls back to the
    // asset uploaded under the application otherwise.
    let large_image = art.unwrap_or(LARGE_ASSET);

    let mut assets = json!({
        "large_image": large_image,
        "small_text": if opts.show_source {
            format!("Listening via Lumen · {}", np.source)
        } else {
            "Listening via Lumen".to_owned()
        },
    });

    // The badge in the corner of the artwork. Only worth drawing when the big
    // image is the album cover — as Lumen's mark on someone else's artwork it
    // says where the presence came from, but stamped on Lumen's own icon it
    // would be the same picture twice.
    //
    // `small_text` alone renders nothing: Discord draws the hover text only if
    // there is a small image to hover over, which is why it was invisible until
    // this was added.
    if art.is_some() {
        assets["small_image"] = json!(LARGE_ASSET);
    }
    if opts.show_album {
        let hover = if np.album.trim().is_empty() { np.source.clone() } else { np.album.clone() };
        assets["large_text"] = json!(hover);
    }

    let mut activity = json!({
        // 2 = Listening ("Listening to <application name>"), 0 = Playing.
        // The choice is the user's because it decides whether buttons render;
        // see `config::ActivityKind`.
        "type": opts.activity.code(),
        "details": trim_field(&np.title, "Unknown track"),
        "assets": assets,
    });

    if opts.show_artist {
        activity["state"] = json!(trim_field(&np.artist, "Unknown artist"));
    }

    // Sent only for an activity type that draws them. Discord accepts the
    // buttons on a Listening activity and then silently shows nothing, which is
    // indistinguishable from a broken URL and sends people hunting for a bug in
    // their own configuration.
    if opts.activity.shows_buttons() {
        let buttons = buttons_for(np, opts);
        if !buttons.is_empty() {
            activity["buttons"] = json!(buttons);
        }
    }

    // Timestamps only while actually playing. Discord animates a running clock
    // from `start`, so leaving them on a paused track shows time advancing on
    // something that is not moving.
    if opts.show_timestamps
        && np.state == PlaybackState::Playing
        && let Some(now_ms) = epoch_millis()
    {
        let position_ms = (np.timeline.position_sec.max(0.0) * 1000.0) as u64;
        let start = now_ms.saturating_sub(position_ms);
        let mut stamps = json!({ "start": start });
        if np.timeline.duration_sec > 0.0 {
            let duration_ms = (np.timeline.duration_sec * 1000.0) as u64;
            stamps["end"] = json!(start + duration_ms);
        }
        activity["timestamps"] = stamps;
    }

    json!({ "pid": std::process::id(), "activity": activity }).to_string()
}

/// The link buttons, resolved against the current track.
///
/// Discord takes at most two and refuses the whole activity over a malformed
/// one, so anything it would reject — an empty label, a non-http URL, a third
/// button — is dropped here rather than costing the presence entirely.
fn buttons_for(np: &NowPlaying, opts: &Discord) -> Vec<serde_json::Value> {
    opts.buttons
        .iter()
        .filter(|b| b.enabled)
        .filter_map(|b| {
            let label: String = b.label.trim().chars().take(32).collect();
            let url = fill(&b.url, np);
            let usable =
                !label.is_empty() && (url.starts_with("https://") || url.starts_with("http://"));
            usable.then(|| json!({ "label": label, "url": url }))
        })
        .take(2)
        .collect()
}

/// Substitute `{title}`, `{artist}` and `{album}` into a button URL.
///
/// Percent-encoded on the way in: these land in a query string, and track names
/// carry spaces, ampersands and non-ASCII as a matter of course. `+` is left
/// alone as the caller wrote it, since that is how a search URL spells a space.
fn fill(template: &str, np: &NowPlaying) -> String {
    template
        .replace("{title}", &crate::net::encode(np.title.trim()))
        .replace("{artist}", &crate::net::encode(np.artist.trim()))
        .replace("{album}", &crate::net::encode(np.album.trim()))
        .trim()
        .to_owned()
}

/// Discord rejects empty strings and requires 2..=128 characters per field.
fn trim_field(value: &str, fallback: &str) -> String {
    let text = value.trim();
    let text = if text.chars().count() < 2 { fallback } else { text };
    text.chars().take(128).collect()
}

fn epoch_millis() -> Option<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

struct Connection {
    pipe: std::fs::File,
}

impl Connection {
    fn open(app_id: &str) -> anyhow::Result<Self> {
        // More than one Discord build can be installed; each takes the next
        // free slot, so the first that answers is the one to talk to.
        let mut last_err = None;
        for n in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{n}");
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(pipe) => {
                    let mut conn = Self { pipe };
                    conn.handshake(app_id)?;
                    return Ok(conn);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow::anyhow!(
            "no Discord IPC pipe answered: {}",
            last_err.map(|e| e.to_string()).unwrap_or_else(|| "none tried".into())
        ))
    }

    fn handshake(&mut self, app_id: &str) -> anyhow::Result<()> {
        let hello = json!({ "v": 1, "client_id": app_id }).to_string();
        self.frame(OP_HANDSHAKE, &hello)?;

        // Discord answers a good id with opcode 1 (FRAME) carrying READY, and a
        // bad one with opcode 2 (CLOSE) carrying `{code, message}` — *not* with
        // an `evt: ERROR` inside a frame. Reading only the body therefore made
        // every rejection look like a success: the connection was announced,
        // and the failure only surfaced one write later as "pipe is closing".
        let (opcode, reply) = self.read_frame()?;
        match opcode {
            OP_CLOSE => anyhow::bail!("Discord refused the connection: {reply}"),
            OP_FRAME if reply.contains(r#""evt":"ERROR""#) => {
                anyhow::bail!("Discord refused the handshake: {reply}")
            }
            OP_FRAME => Ok(()),
            other => anyhow::bail!("unexpected Discord opcode {other}: {reply}"),
        }
    }

    fn set_activity(&mut self, args: &str) -> anyhow::Result<()> {
        let nonce = epoch_millis().unwrap_or(0);
        let cmd =
            format!(r#"{{"cmd":"SET_ACTIVITY","args":{args},"nonce":"{nonce}"}}"#);
        self.frame(OP_FRAME, &cmd)
    }

    fn frame(&mut self, opcode: u32, payload: &str) -> anyhow::Result<()> {
        let bytes = payload.as_bytes();
        let mut out = Vec::with_capacity(8 + bytes.len());
        out.extend_from_slice(&opcode.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
        self.pipe.write_all(&out)?;
        self.pipe.flush()?;
        Ok(())
    }

    /// One frame: its opcode and its JSON body.
    ///
    /// The opcode is returned, not discarded — it is the only place a rejection
    /// is reported.
    fn read_frame(&mut self) -> anyhow::Result<(u32, String)> {
        let mut header = [0u8; 8];
        self.pipe.read_exact(&mut header)?;
        let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        // A malformed length must not turn into a multi-gigabyte allocation.
        anyhow::ensure!(len <= 64 * 1024, "implausible Discord frame length {len}");
        let mut body = vec![0u8; len];
        self.pipe.read_exact(&mut body)?;
        Ok((opcode, String::from_utf8_lossy(&body).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Timeline;

    fn track(state: PlaybackState, position: f64, duration: f64) -> NowPlaying {
        NowPlaying {
            session_id: "Spotify.exe".into(),
            source: "Spotify".into(),
            title: "The Law of Recognition".into(),
            artist: "KYSLINGO".into(),
            album: "Singles".into(),
            state,
            timeline: Timeline { position_sec: position, duration_sec: duration, updated_at_ms: 0.0 },
            art_data_uri: None,
            accent: None,
            revision: 1,
        }
    }

    /// Everything on, no buttons — the baseline the visibility tests vary from.
    fn opts() -> Discord {
        Discord { buttons: Vec::new(), ..Discord::default() }
    }

    fn payload_of(np: &NowPlaying) -> String {
        activity_payload(Some(np), &opts(), None)
    }

    #[test]
    fn clearing_sends_a_null_activity() {
        let payload = activity_payload(None, &opts(), None);
        assert!(payload.contains("\"activity\":null"), "{payload}");
    }

    #[test]
    fn a_playing_track_carries_start_and_end() {
        let payload = payload_of(&track(PlaybackState::Playing, 30.0, 200.0));
        assert!(payload.contains("\"timestamps\""), "{payload}");
        assert!(payload.contains("\"end\""), "{payload}");
        assert!(payload.contains("The Law of Recognition"), "{payload}");
        assert!(payload.contains("Listening via Lumen"), "{payload}");
    }

    /// Discord runs a live clock from `start`, so a paused track would show time
    /// advancing on something that is not moving.
    #[test]
    fn a_paused_track_carries_no_timestamps() {
        let payload = payload_of(&track(PlaybackState::Paused, 30.0, 200.0));
        assert!(!payload.contains("\"timestamps\""), "{payload}");
    }

    /// A live stream has no end time; the elapsed clock is still meaningful.
    #[test]
    fn an_unknown_duration_still_gets_a_start() {
        let payload = payload_of(&track(PlaybackState::Playing, 12.0, 0.0));
        assert!(payload.contains("\"start\""), "{payload}");
        assert!(!payload.contains("\"end\""), "{payload}");
    }

    /// Discord rejects fields shorter than two characters outright.
    #[test]
    fn short_or_empty_fields_fall_back() {
        let mut np = track(PlaybackState::Playing, 0.0, 0.0);
        np.title = String::new();
        np.artist = "x".into();
        let payload = payload_of(&np);
        assert!(payload.contains("Unknown track"), "{payload}");
        assert!(payload.contains("Unknown artist"), "{payload}");
    }

    #[test]
    fn overlong_fields_are_truncated() {
        let mut np = track(PlaybackState::Playing, 0.0, 0.0);
        np.title = "a".repeat(400);
        let payload = payload_of(&np);
        assert!(!payload.contains(&"a".repeat(129)), "title was not truncated");
    }

    #[test]
    fn each_visibility_switch_removes_its_own_field() {
        let np = track(PlaybackState::Playing, 30.0, 200.0);

        let no_artist = Discord { show_artist: false, ..opts() };
        let payload = activity_payload(Some(&np), &no_artist, None);
        assert!(!payload.contains("KYSLINGO"), "{payload}");
        assert!(payload.contains("The Law of Recognition"), "the title must survive: {payload}");

        let no_album = Discord { show_album: false, ..opts() };
        assert!(!activity_payload(Some(&np), &no_album, None).contains("large_text"));

        let no_clock = Discord { show_timestamps: false, ..opts() };
        assert!(!activity_payload(Some(&np), &no_clock, None).contains("timestamps"));

        // The source only ever qualifies the "via Lumen" line, so turning it off
        // must not take the line with it.
        let no_source = Discord { show_source: false, ..opts() };
        let payload = activity_payload(Some(&np), &no_source, None);
        assert!(payload.contains("Listening via Lumen"), "{payload}");
        assert!(!payload.contains("Lumen · Spotify"), "{payload}");
    }

    #[test]
    fn the_source_is_named_when_asked_for() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let payload = activity_payload(Some(&np), &opts(), None);
        assert!(payload.contains("Listening via Lumen · Spotify"), "{payload}");
    }

    /// A URL replaces the application's own asset; without one the asset stands.
    #[test]
    fn cover_art_replaces_the_static_asset() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let url = "https://example.test/512x512bb.jpg";
        let payload = activity_payload(Some(&np), &opts(), Some(url));
        assert!(payload.contains(url), "{payload}");
        // The *large* image must be the cover. `LARGE_ASSET` still appears, as
        // the small badge in its corner, so this checks the field rather than
        // the whole payload.
        let large = format!("\"large_image\":\"{LARGE_ASSET}\"");
        assert!(!payload.contains(&large), "{payload}");

        assert!(activity_payload(Some(&np), &opts(), None).contains(LARGE_ASSET));
    }

    #[test]
    fn the_lumen_badge_rides_on_the_cover_but_not_on_itself() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);

        let with_cover =
            activity_payload(Some(&np), &opts(), Some("https://example.test/cover.jpg"));
        assert!(with_cover.contains("small_image"), "the badge belongs on real artwork");

        // Without a cover the large image is already Lumen's own icon, and a
        // Lumen badge on the Lumen icon is just the picture twice.
        let without = activity_payload(Some(&np), &opts(), None);
        assert!(!without.contains("small_image"), "{without}");
    }

    #[test]
    fn a_button_url_is_filled_in_from_the_track() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let cfg = Discord {
            // Buttons are only sent for a Playing activity; see the test below.
            activity: crate::config::ActivityKind::Playing,
            buttons: vec![crate::config::PresenceButton {
                enabled: true,
                label: "Find this track".into(),
                url: "https://www.youtube.com/results?search_query={artist}+{title}".into(),
            }],
            ..Discord::default()
        };
        let payload = activity_payload(Some(&np), &cfg, None);
        assert!(payload.contains("KYSLINGO+The%20Law%20of%20Recognition"), "{payload}");
        assert!(payload.contains("Find this track"), "{payload}");
    }

    /// Why "the buttons do not work" is usually not a broken button.
    ///
    /// Discord renders presence buttons for a Playing activity and quietly
    /// drops them from a Listening one. Sending them anyway produces a payload
    /// Discord accepts and does not draw, which looks exactly like a bad URL.
    #[test]
    fn buttons_are_sent_only_for_an_activity_that_draws_them() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let with = |activity| Discord {
            activity,
            buttons: vec![crate::config::PresenceButton {
                enabled: true,
                label: "Find this track".into(),
                url: "https://example.test/x".into(),
            }],
            ..Discord::default()
        };

        let listening = activity_payload(Some(&np), &with(crate::config::ActivityKind::Listening), None);
        assert!(listening.contains("\"type\":2"), "{listening}");
        assert!(!listening.contains("buttons"), "{listening}");

        let playing = activity_payload(Some(&np), &with(crate::config::ActivityKind::Playing), None);
        assert!(playing.contains("\"type\":0"), "{playing}");
        assert!(playing.contains("Find this track"), "{playing}");
    }

    /// One bad button would otherwise cost the whole activity.
    #[test]
    fn unusable_buttons_are_dropped_rather_than_sent() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let button = |enabled, label: &str, url: &str| crate::config::PresenceButton {
            enabled,
            label: label.into(),
            url: url.into(),
        };
        let cfg = Discord {
            activity: crate::config::ActivityKind::Playing,
            buttons: vec![
                button(false, "Off", "https://example.test/"),
                button(true, "  ", "https://example.test/"),
                button(true, "No scheme", "example.test"),
                button(true, "Good", "https://example.test/good"),
            ],
            ..Discord::default()
        };
        let payload = activity_payload(Some(&np), &cfg, None);
        assert!(payload.contains("example.test/good"), "{payload}");
        assert!(!payload.contains("\"Off\""), "{payload}");
        assert!(!payload.contains("No scheme"), "{payload}");
    }

    /// Discord accepts two; a third has to go somewhere, and silently is worse.
    #[test]
    fn no_more_than_two_buttons_are_sent() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let cfg = Discord {
            buttons: (0..4)
                .map(|i| crate::config::PresenceButton {
                    enabled: true,
                    label: format!("Button {i}"),
                    url: format!("https://example.test/{i}"),
                })
                .collect(),
            ..Discord::default()
        };
        assert_eq!(buttons_for(&np, &cfg).len(), 2);
    }

    #[test]
    fn labels_are_cut_to_discords_limit() {
        let np = track(PlaybackState::Playing, 0.0, 0.0);
        let cfg = Discord {
            buttons: vec![crate::config::PresenceButton {
                enabled: true,
                label: "L".repeat(80),
                url: "https://example.test/".into(),
            }],
            ..Discord::default()
        };
        let built = buttons_for(&np, &cfg);
        assert_eq!(built[0]["label"].as_str().unwrap().chars().count(), 32);
    }
}
