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
//! real cover cannot be shown without uploading it to some third party first,
//! which is not something a local music widget should be doing to every track
//! someone plays. `large_image` therefore carries the application's own asset,
//! and `album_art_url` exists for the day a source does give us a URL.
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

use crate::media::{NowPlaying, PlaybackState};

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
    /// Start the presence actor. `app_id` is the Discord application's client id.
    pub fn start(app_id: String) -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();

        let _ = std::thread::Builder::new().name("lumen-presence".into()).spawn(move || {
            let mut conn: Option<Connection> = None;
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

                let payload = activity_payload(state.as_ref());
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
                let _ = c.set_activity(&activity_payload(None));
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
fn activity_payload(now: Option<&NowPlaying>) -> String {
    let Some(np) = now else {
        // Clearing is `activity: null`, not an empty object.
        return json!({ "pid": std::process::id(), "activity": null }).to_string();
    };

    let mut activity = json!({
        // 2 = Listening, which renders as "Listening to <application name>".
        "type": 2,
        "details": trim_field(&np.title, "Unknown track"),
        "state": trim_field(&np.artist, "Unknown artist"),
        "assets": {
            "large_image": LARGE_ASSET,
            "large_text": if np.album.trim().is_empty() { np.source.clone() } else { np.album.clone() },
            "small_text": "Listening via Lumen",
        },
    });

    // Timestamps only while actually playing. Discord animates a running clock
    // from `start`, so leaving them on a paused track shows time advancing on
    // something that is not moving.
    if np.state == PlaybackState::Playing
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

    #[test]
    fn clearing_sends_a_null_activity() {
        let payload = activity_payload(None);
        assert!(payload.contains("\"activity\":null"), "{payload}");
    }

    #[test]
    fn a_playing_track_carries_start_and_end() {
        let payload = activity_payload(Some(&track(PlaybackState::Playing, 30.0, 200.0)));
        assert!(payload.contains("\"timestamps\""), "{payload}");
        assert!(payload.contains("\"end\""), "{payload}");
        assert!(payload.contains("The Law of Recognition"), "{payload}");
        assert!(payload.contains("Listening via Lumen"), "{payload}");
    }

    /// Discord runs a live clock from `start`, so a paused track would show time
    /// advancing on something that is not moving.
    #[test]
    fn a_paused_track_carries_no_timestamps() {
        let payload = activity_payload(Some(&track(PlaybackState::Paused, 30.0, 200.0)));
        assert!(!payload.contains("\"timestamps\""), "{payload}");
    }

    /// A live stream has no end time; the elapsed clock is still meaningful.
    #[test]
    fn an_unknown_duration_still_gets_a_start() {
        let payload = activity_payload(Some(&track(PlaybackState::Playing, 12.0, 0.0)));
        assert!(payload.contains("\"start\""), "{payload}");
        assert!(!payload.contains("\"end\""), "{payload}");
    }

    /// Discord rejects fields shorter than two characters outright.
    #[test]
    fn short_or_empty_fields_fall_back() {
        let mut np = track(PlaybackState::Playing, 0.0, 0.0);
        np.title = String::new();
        np.artist = "x".into();
        let payload = activity_payload(Some(&np));
        assert!(payload.contains("Unknown track"), "{payload}");
        assert!(payload.contains("Unknown artist"), "{payload}");
    }

    #[test]
    fn overlong_fields_are_truncated() {
        let mut np = track(PlaybackState::Playing, 0.0, 0.0);
        np.title = "a".repeat(400);
        let payload = activity_payload(Some(&np));
        assert!(!payload.contains(&"a".repeat(129)), "title was not truncated");
    }
}
