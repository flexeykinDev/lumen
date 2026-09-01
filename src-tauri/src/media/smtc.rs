//! `GlobalSystemMediaTransportControlsSessionManager` bridge.
//!
//! # Threading
//!
//! Every WinRT call in this file happens on one dedicated thread (`lumen-smtc`)
//! living in the multi-threaded apartment. WinRT event handlers fire on arbitrary
//! system threads, and calling a blocking `Async::join()` from inside one of them
//! deadlocks — so handlers do nothing but push a [`Msg`] onto a channel, and the
//! worker thread performs all the actual reads.
//!
//! # Idle cost
//!
//! There is no polling anywhere. The worker blocks on `recv()` and wakes only
//! when Windows raises an event or the UI issues a transport command. See
//! ARCHITECTURE.md §3.

use std::{
    sync::{
        Arc, RwLock,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
use tokio::sync::broadcast;
use windows::{
    Foundation::TypedEventHandler,
    Media::Control::{
        CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs, SessionsChangedEventArgs,
        TimelinePropertiesChangedEventArgs,
    },
    Storage::Streams::{DataReader, IRandomAccessStreamReference, InputStreamOptions},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};
use windows_future::IAsyncOperation;

use super::{
    MediaBackend, MediaEvent, NowPlaying, PlaybackState, SessionSummary, Timeline, TransportCmd,
    model::pretty_source,
};
use crate::{color, util::block_on_timeout};

/// Cap on any single WinRT async call. Generous enough for a cold session to
/// hand over a 5 MB cover, short enough that a hung player cannot wedge us.
const WINRT_TIMEOUT: Duration = Duration::from_secs(4);

/// Block on a WinRT operation, turning both failure and a hang into an error.
fn await_op<T: windows::core::RuntimeType>(op: IAsyncOperation<T>) -> anyhow::Result<T> {
    match block_on_timeout(op, WINRT_TIMEOUT) {
        Some(result) => result.map_err(Into::into),
        None => Err(anyhow!("WinRT call timed out after {WINRT_TIMEOUT:?}")),
    }
}

/// Coalescing window. SMTC fires several change events for one logical update
/// (properties, then playback, then timeline); batching them into a single
/// refresh avoids three redundant thumbnail decodes per track change.
const COALESCE: Duration = Duration::from_millis(90);

/// Album art larger than this is almost certainly not album art. Guards against
/// a hostile or broken session handing us a 200 MB stream.
const MAX_ART_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Debug)]
enum Msg {
    /// Something about the current session changed; re-read everything.
    Refresh,
    /// The manager swapped the current session, or the session list changed.
    SessionsChanged,
    Control(TransportCmd),
    Focus(String),
    Cycle,
    Shutdown,
}

pub struct SmtcBackend {
    tx: Sender<Msg>,
    events: broadcast::Sender<MediaEvent>,
    latest: Arc<RwLock<Option<NowPlaying>>>,
    sessions: Arc<RwLock<Vec<SessionSummary>>>,
}

impl SmtcBackend {
    pub fn start() -> anyhow::Result<Arc<Self>> {
        let (tx, rx) = mpsc::channel::<Msg>();
        // Capacity 64: a subscriber that stalls longer than 64 events has bigger
        // problems, and `broadcast` lets it recover by skipping to the latest.
        let (events, _) = broadcast::channel(64);
        let latest = Arc::new(RwLock::new(None));
        let sessions = Arc::new(RwLock::new(Vec::new()));

        let backend = Arc::new(Self {
            tx: tx.clone(),
            events: events.clone(),
            latest: Arc::clone(&latest),
            sessions: Arc::clone(&sessions),
        });

        let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<()>>();

        std::thread::Builder::new()
            .name("lumen-smtc".into())
            .spawn(move || {
                // MTA: we never touch UI objects here, and MTA lets WinRT deliver
                // events on its own pool instead of needing a message pump.
                let apartment = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
                if let Err(e) = apartment {
                    let _ = ready_tx.send(Err(anyhow!("RoInitialize failed: {e}")));
                    return;
                }

                match Worker::new(tx, events, latest, sessions) {
                    Ok(mut worker) => {
                        let _ = ready_tx.send(Ok(()));
                        worker.run(rx);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }

                unsafe { RoUninitialize() };
            })
            .context("failed to spawn the SMTC thread")?;

        // Surface a failed manager handshake at startup rather than as silence.
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .context("SMTC thread did not report readiness")?
            .context("SMTC session manager unavailable")?;

        Ok(backend)
    }
}

impl MediaBackend for SmtcBackend {
    fn subscribe(&self) -> broadcast::Receiver<MediaEvent> {
        self.events.subscribe()
    }

    fn snapshot(&self) -> Option<NowPlaying> {
        self.latest.read().ok().and_then(|g| g.clone())
    }

    fn control(&self, cmd: TransportCmd) -> anyhow::Result<()> {
        self.tx.send(Msg::Control(cmd)).map_err(|_| anyhow!("SMTC thread is gone"))
    }

    fn sessions(&self) -> Vec<SessionSummary> {
        self.sessions.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn focus(&self, session_id: &str) -> anyhow::Result<()> {
        self.tx
            .send(Msg::Focus(session_id.to_owned()))
            .map_err(|_| anyhow!("SMTC thread is gone"))
    }

    fn cycle(&self) -> anyhow::Result<()> {
        self.tx.send(Msg::Cycle).map_err(|_| anyhow!("SMTC thread is gone"))
    }
}

impl Drop for SmtcBackend {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

/// Event registrations we own on the *current* session, so they can be removed
/// when the session is swapped. Leaking these keeps dead sessions alive and
/// eventually delivers events for tracks nobody is playing.
struct SessionHooks {
    session: Session,
    media: i64,
    playback: i64,
    timeline: i64,
}

impl Drop for SessionHooks {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.media);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline);
    }
}

/// A play/pause watch on *every* session, not just the followed one.
///
/// Needed because the choice of which session to follow depends on which one is
/// playing (see `pick_session`), and that can change without the session list
/// changing at all — hitting play in Spotify while Lumen follows a paused
/// browser tab raises no `SessionsChanged`. Without this, Lumen would stay
/// attached to the silent app until something else happened to jog it.
struct PlaybackWatch {
    entries: Vec<(Session, i64)>,
}

impl Drop for PlaybackWatch {
    fn drop(&mut self) {
        for (session, token) in self.entries.drain(..) {
            let _ = session.RemovePlaybackInfoChanged(token);
        }
    }
}

struct Worker {
    manager: Manager,
    /// Handlers on the session currently being followed.
    hooks: Option<SessionHooks>,
    /// Play/pause handlers on every session, for re-selection.
    watch: Option<PlaybackWatch>,
    /// AUMID the user explicitly asked to follow, if any.
    pinned: Option<String>,
    tx: Sender<Msg>,
    events: broadcast::Sender<MediaEvent>,
    latest: Arc<RwLock<Option<NowPlaying>>>,
    sessions: Arc<RwLock<Vec<SessionSummary>>>,
    /// Bumped only when the track identity changes; drives the art crossfade.
    revision: u64,
    /// Process start, so timeline samples carry a monotonic host timestamp.
    started: Instant,
    /// Lumen's own playback clock. See `clock_position`.
    clock: Option<TrackClock>,
    /// A source value that disagreed with the clock, awaiting confirmation.
    resync: Option<(f64, Instant)>,
    /// When the last confirmation re-read was scheduled, to rate-limit them.
    probe_at: Option<Instant>,
    /// Consecutive probes that failed to resolve a disagreement.
    disagreements: u32,
    /// When the source was last forced to republish.
    last_nudge: Option<Instant>,
    /// Set by `clock_position`, acted on by `refresh`, which holds the session.
    want_nudge: bool,
}

/// How far a reported position may sit from Lumen's own clock and still be
/// believed. Wide enough to absorb sampling jitter and coalescing, narrow enough
/// that a post-seek zero never passes.
const CLOCK_TOLERANCE: f64 = 2.5;

/// Minimum spacing between a disagreeing sample and the one that confirms it.
/// Two readings taken milliseconds apart say nothing about whether playback is
/// really there; they only say the source repeated itself.
const RESYNC_MIN_GAP: f64 = 1.5;

/// How long after a disagreement to re-read the source. Slightly longer than
/// `RESYNC_MIN_GAP` so the reading taken then is already old enough to count as
/// confirmation.
const RESYNC_PROBE: Duration = Duration::from_millis(1600);

/// How close two readings must be to count as the source repeating itself
/// rather than reporting progress.
const FROZEN_EPSILON: f64 = 0.25;

/// Repeats of a frozen value before forcing the source to republish. Three is
/// roughly five seconds of a stuck timeline.
///
/// This only ever counts *frozen* readings, so a well-behaved source is never
/// nudged: measured on this machine, Edge and Spotify resync on the first or
/// second re-read and never reach this at all. It is a workaround for one
/// specific fault — Firefox freezing its timeline at 0.0 after an in-page
/// seek — and is gated to look exactly like that fault.
const NUDGE_AFTER: u32 = 3;
/// Minimum spacing between forced republishes.
const NUDGE_COOLDOWN: Duration = Duration::from_secs(10);
/// Pause between the halves of a nudge, and between resume attempts.
const RESUME_GAP: Duration = Duration::from_millis(120);
/// How many times to insist on resuming before forcing it outright.
const RESUME_ATTEMPTS: u32 = 3;

/// Lumen's own view of where playback is, carried between samples.
struct TrackClock {
    /// Position that was trusted at `at`.
    position: f64,
    at: Instant,
}

impl Worker {
    fn new(
        tx: Sender<Msg>,
        events: broadcast::Sender<MediaEvent>,
        latest: Arc<RwLock<Option<NowPlaying>>>,
        sessions: Arc<RwLock<Vec<SessionSummary>>>,
    ) -> anyhow::Result<Self> {
        let manager = await_op(Manager::RequestAsync().context("RequestAsync call failed")?)
            .context("SMTC manager handshake failed")?;

        let mut worker = Self {
            manager,
            hooks: None,
            watch: None,
            pinned: None,
            tx,
            events,
            latest,
            sessions,
            revision: 0,
            started: Instant::now(),
            clock: None,
            resync: None,
            probe_at: None,
            disagreements: 0,
            last_nudge: None,
            want_nudge: false,
        };

        worker.hook_manager()?;
        worker.rewatch_all();
        worker.rebind_session();
        worker.refresh();
        Ok(worker)
    }

    fn hook_manager(&mut self) -> anyhow::Result<()> {
        let tx = self.tx.clone();
        self.manager
            .CurrentSessionChanged(&TypedEventHandler::<Manager, CurrentSessionChangedEventArgs>::new(
                move |_, _| {
                    let _ = tx.send(Msg::SessionsChanged);
                    Ok(())
                },
            ))
            .context("CurrentSessionChanged registration failed")?;

        let tx = self.tx.clone();
        self.manager
            .SessionsChanged(&TypedEventHandler::<Manager, SessionsChangedEventArgs>::new(
                move |_, _| {
                    let _ = tx.send(Msg::SessionsChanged);
                    Ok(())
                },
            ))
            .context("SessionsChanged registration failed")?;

        Ok(())
    }

    fn run(&mut self, rx: mpsc::Receiver<Msg>) {
        loop {
            // Block indefinitely — this is what keeps idle CPU at zero.
            let Ok(first) = rx.recv() else { return };

            let mut refresh = false;
            let mut resession = false;
            let mut msg = Some(first);

            // Drain everything that arrives within the coalescing window so one
            // logical change produces one refresh.
            let deadline = Instant::now() + COALESCE;
            loop {
                match msg.take() {
                    Some(Msg::Shutdown) => return,
                    Some(Msg::Refresh) => refresh = true,
                    Some(Msg::SessionsChanged) => resession = true,
                    Some(Msg::Control(cmd)) => {
                        if let Err(e) = self.dispatch(cmd) {
                            tracing::warn!("transport command {cmd:?} failed: {e}");
                        }
                        // Windows will raise its own change events; no forced refresh.
                    }
                    Some(Msg::Focus(id)) => {
                        self.pinned = Some(id);
                        resession = true;
                    }
                    Some(Msg::Cycle) => {
                        self.cycle_session();
                        resession = true;
                    }
                    None => {}
                }

                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok(next) => msg = Some(next),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }

            if resession {
                // Order matters: refresh the watch first so a session that
                // appeared in this batch is already being observed, then choose.
                self.rewatch_all();
                self.rebind_session();
                self.publish_sessions();
            }
            if resession || refresh {
                self.refresh();
            }
        }
    }

    /// Watch play/pause on every session, so a source starting playback in the
    /// background can trigger a re-selection.
    fn rewatch_all(&mut self) {
        // Dropping the previous watch unregisters every handler in it.
        self.watch = None;

        let Ok(list) = self.manager.GetSessions() else { return };
        let mut entries = Vec::new();
        for session in list {
            let tx = self.tx.clone();
            let handler = TypedEventHandler::<Session, PlaybackInfoChangedEventArgs>::new(
                move |_, _| {
                    // Re-evaluate *which* session to follow, not just refresh
                    // the current one's state.
                    let _ = tx.send(Msg::SessionsChanged);
                    Ok(())
                },
            );
            if let Ok(token) = session.PlaybackInfoChanged(&handler) {
                entries.push((session, token));
            }
        }
        self.watch = Some(PlaybackWatch { entries });
    }

    /// Attach per-session handlers to whichever session we should be following.
    ///
    /// Idempotent: if the choice has not changed, the existing hooks are left
    /// alone. That matters because the all-session playback watch feeds back
    /// into here, and tearing down and rebuilding handlers on every event would
    /// churn constantly while music plays.
    fn rebind_session(&mut self) {
        let picked = self.pick_session();
        let picked_id =
            picked.as_ref().and_then(|s| s.SourceAppUserModelId().ok()).map(|h| h.to_string());
        let current_id = self
            .hooks
            .as_ref()
            .and_then(|h| h.session.SourceAppUserModelId().ok())
            .map(|h| h.to_string());

        if picked_id.is_some() && picked_id == current_id {
            return;
        }

        // Dropping the old hooks unregisters them.
        self.hooks = None;

        let Some(session) = picked else { return };

        // Each of the three events carries a different args type, so a shared
        // closure would be pinned to whichever one instantiated it first. Three
        // one-line bodies are cheaper than the generic machinery to avoid them.
        macro_rules! poke {
            ($args:ty) => {{
                let tx = self.tx.clone();
                TypedEventHandler::<Session, $args>::new(move |_, _| {
                    let _ = tx.send(Msg::Refresh);
                    Ok(())
                })
            }};
        }

        let media = session.MediaPropertiesChanged(&poke!(MediaPropertiesChangedEventArgs));
        let playback = session.PlaybackInfoChanged(&poke!(PlaybackInfoChangedEventArgs));
        let timeline =
            session.TimelinePropertiesChanged(&poke!(TimelinePropertiesChangedEventArgs));

        match (media, playback, timeline) {
            (Ok(media), Ok(playback), Ok(timeline)) => {
                // Logged at info: "which session do we actually hold" is the
                // first question whenever transport control appears to do
                // nothing, and without it the happy path is entirely silent.
                tracing::info!(
                    "following session: {}",
                    session.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default()
                );
                self.hooks = Some(SessionHooks { session, media, playback, timeline });
            }
            _ => tracing::warn!("could not fully hook the current SMTC session"),
        }
    }

    /// Choose the session to follow.
    ///
    /// **Not** simply `GetCurrentSession()`. Windows' "current session" means
    /// the app that most recently *interacted* with SMTC, which is not the same
    /// as the app that is *playing*: a browser tab that was paused an hour ago
    /// routinely holds that title while a music player is actually producing
    /// sound. Following it verbatim sends every transport command to the wrong
    /// app — where it is accepted, and does nothing audible.
    ///
    /// Priority: an explicit pin, then Windows' choice *if it is playing*, then
    /// any playing session, then Windows' choice as a last resort.
    fn pick_session(&mut self) -> Option<Session> {
        let sessions: Vec<Session> = self
            .manager
            .GetSessions()
            .map(|list| list.into_iter().collect())
            .unwrap_or_default();

        let aumid = |s: &Session| s.SourceAppUserModelId().ok().map(|h| h.to_string());
        let is_playing = |s: &Session| {
            matches!(
                s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()),
                Ok(Status::Playing)
            )
        };

        // 1. An explicit pin from the session switcher wins outright.
        if let Some(id) = self.pinned.clone() {
            if let Some(s) = sessions.iter().find(|s| aumid(s).as_deref() == Some(id.as_str())) {
                return Some(s.clone());
            }
            // The pinned source closed; fall back to automatic selection.
            self.pinned = None;
        }

        let current = self.manager.GetCurrentSession().ok();

        // 2. Windows' choice, when it is actually playing.
        if let Some(c) = current.as_ref()
            && is_playing(c)
        {
            return Some(c.clone());
        }

        // 3. Any session that is playing.
        if let Some(s) = sessions.iter().find(|s| is_playing(s)) {
            return Some(s.clone());
        }

        // 4. Nothing is playing — Windows' choice is as good as any.
        current
    }

    /// Advance to the next session in the manager's order, wrapping at the end.
    ///
    /// The result is pinned. Without that, the "prefer whatever is playing" rule
    /// in `pick_session` would immediately override the choice the moment any
    /// other source resumed — the user would press the key and watch it snap
    /// straight back.
    fn cycle_session(&mut self) {
        // Only sessions with something to show are worth switching *to*.
        //
        // A `Stopped`/`Closed` session has no track and no artwork, so landing
        // on one makes the island decide there is nothing to display and hide
        // itself — and because cycling pins its choice, it then stays hidden
        // even while another app is playing. With the island gone the badge is
        // gone too, so the only way back is the hotkey. Skipping empty sessions
        // removes the trap entirely.
        let sessions: Vec<String> = self
            .manager
            .GetSessions()
            .map(|list| {
                list.into_iter()
                    .filter(|s| {
                        matches!(
                            s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()),
                            Ok(Status::Playing) | Ok(Status::Paused) | Ok(Status::Changing)
                        )
                    })
                    .filter_map(|s| s.SourceAppUserModelId().ok().map(|h| h.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if sessions.len() < 2 {
            tracing::info!(
                "cycle: only {} switchable session(s), staying put",
                sessions.len()
            );
            return;
        }

        // Start from whatever is currently *followed* rather than from the pin:
        // with no pin set the followed session is the automatic choice, and
        // cycling should move on from what the user can actually see.
        let current = self
            .hooks
            .as_ref()
            .and_then(|h| h.session.SourceAppUserModelId().ok())
            .map(|h| h.to_string());

        let index = current
            .as_ref()
            .and_then(|c| sessions.iter().position(|s| s == c))
            .unwrap_or(0);
        let next = sessions[(index + 1) % sessions.len()].clone();

        tracing::info!("cycle: {} -> {}", current.unwrap_or_else(|| "none".into()), next);
        self.pinned = Some(next);
    }

    fn publish_sessions(&self) {
        let current = self
            .manager
            .GetCurrentSession()
            .ok()
            .and_then(|s| s.SourceAppUserModelId().ok())
            .map(|h| h.to_string());

        let mut out = Vec::new();
        if let Ok(list) = self.manager.GetSessions() {
            for s in list {
                let Ok(aumid) = s.SourceAppUserModelId() else { continue };
                let id = aumid.to_string();
                let is_current = self
                    .pinned
                    .as_deref()
                    .map_or_else(|| current.as_deref() == Some(id.as_str()), |p| p == id);
                out.push(SessionSummary { source: pretty_source(&id), id, is_current });
            }
        }

        if let Ok(mut guard) = self.sessions.write() {
            *guard = out.clone();
        }
        let _ = self.events.send(MediaEvent::SessionsChanged(out));
    }

    /// Lumen's own answer to "where is playback right now".
    ///
    /// # Why this exists
    ///
    /// SMTC is not a reliable narrator. Firefox accepts a seek, moves the
    /// playhead, and then reports `position 0.0, duration 0.0` for several
    /// seconds — measured directly, not inferred. Every attempt to patch that by
    /// pattern-matching the bad values failed on some other source, because
    /// "is this number a lie" is not answerable from the number alone.
    ///
    /// So the model is inverted: **Lumen keeps the clock, and SMTC is a hint.**
    /// The clock advances only while playback is running, which makes it exact
    /// for the common case, and it re-syncs to the source only when the source
    /// proves itself.
    ///
    /// # What counts as proof
    ///
    /// A single sample that disagrees is discarded — that is the post-seek lie.
    /// But a sample that disagrees and is then *confirmed* by a later sample
    /// advancing consistently from it is real: that is what an external seek
    /// (the user dragging YouTube's own bar) looks like, and it is accepted on
    /// the second sighting. Two strikes, not one, is the whole trick — it
    /// separates a transient lie from a new truth without guessing.
    fn clock_position(
        &mut self,
        state: PlaybackState,
        track_changed: bool,
        timeline: Timeline,
    ) -> f64 {
        let playing = state == PlaybackState::Playing;
        let reported = timeline.position_sec;
        let cap = |v: f64| {
            if timeline.duration_sec > 0.0 { v.min(timeline.duration_sec).max(0.0) } else { v.max(0.0) }
        };

        // A new track has no history worth keeping; believe the source outright.
        if track_changed || self.clock.is_none() {
            self.clock = Some(TrackClock { position: reported, at: Instant::now() });
            self.resync = None;
            self.probe_at = None;
            self.disagreements = 0;
            return cap(reported);
        }

        let clock = self.clock.as_mut().expect("checked above");
        let expected = clock.position + if playing { clock.at.elapsed().as_secs_f64() } else { 0.0 };

        if (reported - expected).abs() <= CLOCK_TOLERANCE {
            // Agreement: adopt the source's value and restart the local clock
            // from it, so drift never accumulates.
            clock.position = reported;
            clock.at = Instant::now();
            self.resync = None;
            self.probe_at = None;
            self.disagreements = 0;
            return cap(reported);
        }

        // Disagreement. Is this the second sighting of the same new reality?
        //
        // Two conditions, and the second is the one that matters: the candidate
        // must have *advanced*. A source that is merely stuck republishes the
        // same stale value, and two identical samples 200 ms apart trivially
        // agree with each other — which is how a repeated post-seek `0.0`
        // confirmed itself and threw away a correct position that arrived
        // immediately afterwards. Requiring visible progress means only a
        // genuinely running position can win.
        if let Some((candidate, since)) = self.resync {
            let gap = since.elapsed().as_secs_f64();
            // Judge the *rate*, not the absolute position.
            //
            // Comparing positions with a fixed tolerance only works when the two
            // are already close. Once the clock and the source are far apart —
            // after an external seek, say — a correct, running source stays
            // permanently outside the tolerance and is rejected forever, which
            // stranded the counter at 20s while the video played on at 8:43.
            //
            // How fast the reported position is moving answers the real
            // question directly: about one second per second means the source is
            // playing and telling the truth, whatever the value; about zero
            // means it is stuck.
            let rate = if gap > 0.0 { (reported - candidate) / gap } else { 0.0 };
            let running =
                if playing { (0.5..=1.6).contains(&rate) } else { (reported - candidate).abs() <= CLOCK_TOLERANCE };
            if gap >= RESYNC_MIN_GAP && running {
                tracing::debug!("timeline resynced to {reported:.1}s (was showing {expected:.1}s)");
                clock.position = reported;
                clock.at = Instant::now();
                self.resync = None;
            self.probe_at = None;
            self.disagreements = 0;
                return cap(reported);
            }
        }

        // First sighting: remember it, keep our own clock running, and go back
        // and look again shortly.
        //
        // The re-read is the point. Waiting for the source to volunteer a second
        // sample does not work: YouTube publishes a timeline only on discrete
        // events, so while playing there is no follow-up at all and an external
        // seek is never confirmed. Pausing produces one, which is precisely why
        // pausing appeared to "fix" the sync. Asking again on our own schedule
        // removes the dependency on the source being chatty.
        // Disagreeing and *frozen* are different faults, and only one of them
        // needs the heavy fix.
        //
        // A source that disagrees but keeps moving is simply somewhere else —
        // mid-seek, or seeked in its own player — and the confirmation rule
        // above will adopt it within a couple of seconds. Edge and Spotify
        // behave this way and must never be interfered with.
        //
        // A source repeating the identical value is stuck, and no amount of
        // re-reading will ever change it. Only that case earns a nudge. Keeping
        // the original candidate timestamp matters too: it makes the
        // confirmation window measure real elapsed time rather than restarting
        // on every repeat.
        let frozen = self
            .resync
            .is_some_and(|(candidate, _)| (reported - candidate).abs() <= FROZEN_EPSILON);
        if !frozen {
            self.resync = Some((reported, Instant::now()));
            self.disagreements = 0;
        }
        tracing::debug!(
            "ignoring timeline {reported:.1}s; holding {expected:.1}s{}",
            if frozen { " (source frozen)" } else { "" }
        );

        let due = self.probe_at.is_none_or(|t| t.elapsed().as_secs_f64() >= RESYNC_MIN_GAP);
        if due {
            if frozen {
                self.disagreements += 1;
            }
            self.probe_at = Some(Instant::now());
            let tx = self.tx.clone();
            let _ = std::thread::Builder::new().name("lumen-timeline-probe".into()).spawn(
                move || {
                    std::thread::sleep(RESYNC_PROBE);
                    let _ = tx.send(Msg::Refresh);
                },
            );
        }

        // Re-reading does not help when the source has simply stopped updating.
        // Firefox reports a frozen 0.0 after a seek made in the page itself and
        // never revises it while playing — so no amount of asking again will
        // reveal where the video actually is. A playback state change is the one
        // thing that makes it republish, which is why pausing by hand "fixed"
        // the sync.
        //
        // So after several fruitless probes, force that republish. Deliberately
        // a last resort: it is rate-limited, only ever runs while playing, and
        // verifies the resume, because pausing someone's music and leaving it
        // paused is far worse than a counter that is briefly out of step.
        if playing
            && self.disagreements >= NUDGE_AFTER
            && self.last_nudge.is_none_or(|t| t.elapsed() >= NUDGE_COOLDOWN)
        {
            self.want_nudge = true;
        }

        cap(expected)
    }

    /// Make a stuck source republish its timeline, by pausing and resuming it.
    ///
    /// The resume is *verified*, not assumed. An earlier version fired play
    /// immediately after pause and ignored the result; a source that had not
    /// finished pausing dropped the play and the track simply stayed stopped.
    fn force_republish(&mut self, session: &Session) {
        self.last_nudge = Some(Instant::now());
        self.disagreements = 0;
        tracing::info!("source timeline is stuck; forcing a republish with pause/play");

        let done = (|| -> anyhow::Result<bool> {
            if !await_op(session.TryPauseAsync()?)? {
                return Ok(false);
            }
            std::thread::sleep(RESUME_GAP);
            for attempt in 1..=RESUME_ATTEMPTS {
                if await_op(session.TryPlayAsync()?)?
                    && matches!(
                        session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()),
                        Ok(Status::Playing)
                    )
                {
                    return Ok(true);
                }
                tracing::warn!("resume did not take (attempt {attempt})");
                std::thread::sleep(RESUME_GAP);
            }
            Ok(false)
        })();

        if !matches!(done, Ok(true)) {
            if let Err(e) = &done {
                tracing::warn!("forced republish failed: {e}");
            }
            // Whatever happened, playback must not be left paused by something
            // the user never asked for.
            if matches!(
                session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()),
                Ok(Status::Paused)
            ) {
                tracing::warn!("republish left playback paused; forcing resume");
                let _ = session.TryPlayAsync().map(await_op);
            }
        }
    }

    /// Read the full session state and emit the narrowest event that describes it.
    fn refresh(&mut self) {
        let Some(hooks) = self.hooks.as_ref() else {
            if self.latest.read().ok().and_then(|g| g.clone()).is_some() {
                if let Ok(mut g) = self.latest.write() {
                    *g = None;
                }
                let _ = self.events.send(MediaEvent::Vanished);
            }
            return;
        };
        let session = hooks.session.clone();

        let aumid = session.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        let state = read_state(&session);
        let mut timeline = self.read_timeline(&session);

        let (title, artist, album, art_ref) = match session.TryGetMediaPropertiesAsync() {
            Ok(op) => match await_op(op) {
                Ok(p) => (
                    p.Title().map(|h| h.to_string()).unwrap_or_default(),
                    p.Artist().map(|h| h.to_string()).unwrap_or_default(),
                    p.AlbumTitle().map(|h| h.to_string()).unwrap_or_default(),
                    p.Thumbnail().ok(),
                ),
                Err(e) => {
                    tracing::debug!("media properties unavailable: {e}");
                    return;
                }
            },
            Err(e) => {
                tracing::debug!("TryGetMediaPropertiesAsync failed: {e}");
                return;
            }
        };

        let previous = self.latest.read().ok().and_then(|g| g.clone());
        let identity = (title.as_str(), artist.as_str(), album.as_str());
        let track_changed = previous.as_ref().is_none_or(|p| {
            p.identity() != identity || p.session_id != aumid
        });

        // Browsers routinely drop `EndTime` to zero for a moment after a seek —
        // the timeline is republished before the media element has settled. Read
        // literally that means "no duration", which the UI can only interpret as
        // a live stream; and because nothing republishes a *correct* timeline
        // until the next play/pause, the track stays stuck reading LIVE for the
        // rest of its length.
        //
        // A track cannot lose its duration while remaining the same track, so
        // carry the last known-good value across until the track actually
        // changes.
        if timeline.duration_sec <= 0.0
            && !track_changed
            && let Some(prev) = previous.as_ref()
            && prev.timeline.duration_sec > 0.0
        {
            timeline.duration_sec = prev.timeline.duration_sec;
        }

        timeline.position_sec = self.clock_position(state, track_changed, timeline);

        // Done here rather than inside `clock_position` because it needs the
        // session, and because it must not run while that borrow is live.
        if std::mem::take(&mut self.want_nudge) {
            self.force_republish(&session);
        }

        // Artwork decoding and quantization is the only expensive thing here, so
        // it happens exactly once per track — never on a play/pause or a seek.
        let (art_data_uri, accent) = if track_changed {
            match art_ref.as_ref().and_then(|r| read_thumbnail(r).ok()) {
                Some(bytes) => {
                    let accent = color::extract(&bytes);
                    (Some(to_data_uri(&bytes)), accent)
                }
                None => (None, None),
            }
        } else {
            previous
                .as_ref()
                .map(|p| (p.art_data_uri.clone(), p.accent))
                .unwrap_or((None, None))
        };

        if track_changed {
            self.revision = self.revision.wrapping_add(1);
        }

        let now = NowPlaying {
            session_id: aumid.clone(),
            source: pretty_source(&aumid),
            title,
            artist,
            album,
            state,
            timeline,
            art_data_uri,
            accent,
            revision: self.revision,
        };

        // Sessions that exist but publish nothing (a browser tab that finished)
        // should read as "gone", not as an empty capsule.
        if !now.has_content() && state == PlaybackState::Stopped {
            // Safety net for an explicit pin that has gone empty — the source
            // was closed, or stopped after the user pinned it. Hiding here would
            // strand the island: pinned to a dead source, invisible, with the
            // badge that could switch away from it no longer on screen.
            // Drop the pin and re-pick instead of vanishing.
            if self.pinned.is_some() {
                tracing::info!("pinned session went empty; releasing the pin");
                self.pinned = None;
                self.rebind_session();
                if self.hooks.is_some() {
                    self.refresh();
                    return;
                }
            }

            if previous.is_some() {
                if let Ok(mut g) = self.latest.write() {
                    *g = None;
                }
                let _ = self.events.send(MediaEvent::Vanished);
            }
            return;
        }

        if let Ok(mut g) = self.latest.write() {
            *g = Some(now.clone());
        }

        let event = if track_changed {
            MediaEvent::TrackChanged(Box::new(now))
        } else if previous.as_ref().is_some_and(|p| p.state != state) {
            MediaEvent::PlaybackChanged(Box::new(now))
        } else {
            MediaEvent::TimelineChanged(Box::new(now))
        };
        let _ = self.events.send(event);
    }

    fn read_timeline(&self, session: &Session) -> Timeline {
        let updated_at_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        let Ok(t) = session.GetTimelineProperties() else {
            return Timeline { updated_at_ms, ..Timeline::default() };
        };

        // TimeSpan is 100 ns ticks. Some sources report a non-zero StartTime
        // (chapters, podcast segments); positions are relative to it.
        let ticks = |v: Option<i64>| v.unwrap_or(0) as f64 / 10_000_000.0;
        let start = ticks(t.StartTime().ok().map(|v| v.Duration));
        let end = ticks(t.EndTime().ok().map(|v| v.Duration));
        let pos = ticks(t.Position().ok().map(|v| v.Duration));

        let duration_sec = (end - start).max(0.0);
        let position_sec = (pos - start).clamp(0.0, if duration_sec > 0.0 { duration_sec } else { f64::MAX });

        Timeline { position_sec, duration_sec, updated_at_ms }
    }

    fn dispatch(&mut self, cmd: TransportCmd) -> anyhow::Result<()> {
        let session = self
            .hooks
            .as_ref()
            .map(|h| h.session.clone())
            .ok_or_else(|| anyhow!("no active media session"))?;

        let target =
            session.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        tracing::info!("dispatching {cmd:?} to {target}");

        let ok = match cmd {
            TransportCmd::PlayPause => await_op(session.TryTogglePlayPauseAsync()?)?,
            TransportCmd::Next => await_op(session.TrySkipNextAsync()?)?,
            TransportCmd::Previous => await_op(session.TrySkipPreviousAsync()?)?,
            TransportCmd::Seek(seconds) => {
                // SMTC positions are 100 ns ticks and are absolute from the
                // track's StartTime, which is not always zero (podcast chapters,
                // CUE-split files). `read_timeline` subtracts StartTime on the
                // way out, so it has to be added back on the way in or seeking
                // lands at the wrong place on exactly those sources.
                let start_ticks = session
                    .GetTimelineProperties()
                    .and_then(|t| t.StartTime())
                    .map(|v| v.Duration)
                    .unwrap_or(0);
                let ticks = start_ticks + (seconds.max(0.0) * 10_000_000.0) as i64;
                await_op(session.TryChangePlaybackPositionAsync(ticks)?)?
            }
        };

        // A seek we issued is the truth, immediately. The clock moves to the
        // requested position and starts running from there, so the UI is correct
        // before the source has said anything at all.
        //
        // Applied whether or not SMTC said yes: `TryChangePlaybackPositionAsync`
        // returns false on YouTube in Firefox and the video seeks anyway — the
        // return value describes what the *session* acknowledged, not what the
        // media element did.
        if let TransportCmd::Seek(seconds) = cmd {
            if !ok {
                tracing::debug!("seek to {seconds:.1}s was not acknowledged; tracking it anyway");
            }
            self.clock = Some(TrackClock { position: seconds.max(0.0), at: Instant::now() });
            self.resync = None;
            self.probe_at = None;
            self.disagreements = 0;
        }

        // Logged either way. A silent success is indistinguishable from a
        // command that never ran, which is exactly the hole that made a
        // non-functioning hotkey impossible to diagnose from the log.
        if ok {
            tracing::info!("{target} accepted {cmd:?}");
        } else {
            // Common and benign: many sources refuse Previous on the first track.
            tracing::info!("{target} DECLINED {cmd:?}");
        }
        Ok(())
    }
}

fn read_state(session: &Session) -> PlaybackState {
    match session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()) {
        Ok(Status::Playing) => PlaybackState::Playing,
        Ok(Status::Paused) => PlaybackState::Paused,
        Ok(Status::Changing) | Ok(Status::Opened) => PlaybackState::Changing,
        _ => PlaybackState::Stopped,
    }
}

/// Pull the thumbnail into memory. Blocking, and only ever called on the worker.
fn read_thumbnail(reference: &IRandomAccessStreamReference) -> anyhow::Result<Vec<u8>> {
    let stream = await_op(reference.OpenReadAsync()?)?;
    let size = stream.Size()?;

    if size == 0 {
        return Err(anyhow!("empty thumbnail stream"));
    }
    if size > MAX_ART_BYTES {
        return Err(anyhow!("thumbnail of {size} bytes exceeds the {MAX_ART_BYTES} byte cap"));
    }

    let reader = DataReader::CreateDataReader(&stream)?;
    reader.SetInputStreamOptions(InputStreamOptions::None)?;

    let len = size as u32;
    let loaded = await_op(reader.LoadAsync(len)?)?;
    let mut buf = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut buf)?;
    Ok(buf)
}

/// Sniff the container so the renderer gets a correct MIME type. SMTC hands back
/// whatever the source embedded — usually JPEG, sometimes PNG, occasionally BMP.
fn to_data_uri(bytes: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};

    let mime = match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', ..] => "image/gif",
        [b'B', b'M', ..] => "image/bmp",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "image/jpeg",
    };

    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}
