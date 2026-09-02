//! The renderer-facing surface: commands in, events out.
//!
//! Deliberately small. The renderer draws and reports hover; it makes no
//! decisions about visibility, position, or media state.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{
    audio::{VolumeControl, VolumeState},
    config::{Config, ConfigStore, Origin},
    input::HotkeyService,
    media::{MediaBackend, NowPlaying, SessionSummary, TransportCmd},
    policy::Policy,
    window::{BackdropKind, IslandState, Placement},
};

pub const EVT_NOW_PLAYING: &str = "lumen://now-playing";
pub const EVT_TRANSITION: &str = "lumen://transition";
pub const EVT_CONFIG: &str = "lumen://config";
pub const EVT_VOLUME: &str = "lumen://volume";
/// A single application's volume moved. Distinct from `EVT_VOLUME`, which is
/// always the system master.
pub const EVT_APP_VOLUME: &str = "lumen://app-volume";
pub const EVT_SESSIONS: &str = "lumen://sessions";
/// Which way round the capsule's contents belong. Changes when the island is
/// docked to, or dragged to, the other half of the screen.
pub const EVT_PLACEMENT: &str = "lumen://placement";
/// Ask the renderer to compose a share card. Sent by the tray, because only the
/// renderer can draw one.
pub const EVT_SHARE_REQUEST: &str = "lumen://share-request";
/// Timed lyrics for one track, sent once when they arrive. The renderer picks
/// the current line from the clock it already interpolates, so playback itself
/// costs no IPC.
pub const EVT_LYRICS: &str = "lumen://lyrics";
/// Spectrum bands, ~20 times a second, and only while the bars are on screen.
/// The one event in Lumen that fires during steady playback — see spectrum.
pub const EVT_SPECTRUM: &str = "lumen://spectrum";

/// Everything the renderer needs once, at boot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    /// Which system backdrop actually landed — the CSS differs for each.
    pub backdrop: BackdropKind,
    /// Where settings are stored, or `null` if nothing was writable.
    pub config_path: Option<String>,
    /// True when settings live beside the exe (fully portable).
    pub portable: bool,
    pub version: &'static str,
}

pub struct Ctx {
    pub media: Arc<dyn MediaBackend>,
    pub policy: Arc<Policy>,
    pub cfg: Arc<ConfigStore>,
    pub hotkeys: Arc<HotkeyService>,
    /// `None` when no audio endpoint exists (no sound card, all devices
    /// disabled). Volume commands then no-op instead of failing the app.
    pub volume: Option<Arc<VolumeControl>>,
    pub app: AppHandle,
    pub info: RuntimeInfo,
    /// The audio capture, alive only while the bars are on screen. `None` the
    /// rest of the time — the thread does not exist and the endpoint is closed.
    pub spectrum: std::sync::Mutex<Option<crate::spectrum::Spectrum>>,
}

#[tauri::command]
pub fn runtime_info(ctx: State<'_, Ctx>) -> RuntimeInfo {
    ctx.info.clone()
}

/// Late-join snapshot. The renderer calls this on mount so a WebView reload
/// never leaves an empty capsule waiting for the next SMTC event.
#[tauri::command]
pub fn now_playing(ctx: State<'_, Ctx>) -> Option<NowPlaying> {
    ctx.media.snapshot()
}

#[tauri::command]
pub fn sessions(ctx: State<'_, Ctx>) -> Vec<SessionSummary> {
    ctx.media.sessions()
}

/// Follow the next media source. No-op when only one is publishing.
#[tauri::command]
pub fn cycle_session(ctx: State<'_, Ctx>) -> Result<(), String> {
    ctx.media.cycle().map_err(|e| e.to_string())
}

/// Follow a specific source by AUMID.
#[tauri::command]
pub fn focus_session(ctx: State<'_, Ctx>, session_id: String) -> Result<(), String> {
    ctx.media.focus(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn transport(ctx: State<'_, Ctx>, action: &str) -> Result<(), String> {
    let cmd = match action {
        "playPause" => TransportCmd::PlayPause,
        "next" => TransportCmd::Next,
        "previous" => TransportCmd::Previous,
        other => return Err(format!("unknown transport action {other:?}")),
    };
    ctx.media.control(cmd).map_err(|e| e.to_string())
}

/// The renderer owns hover detection because only it knows the real hit shape.
#[tauri::command]
pub fn set_hover(ctx: State<'_, Ctx>, hovering: bool) {
    ctx.policy.set_hover(hovering);
}

#[tauri::command]
pub fn island_state(ctx: State<'_, Ctx>) -> IslandState {
    ctx.policy.island().state()
}

/// Late-join read of the layout direction, for the same reason `island_state`
/// exists: the island is usually placed before this WebView is listening, so the
/// event that announced it has already gone.
#[tauri::command]
pub fn placement(ctx: State<'_, Ctx>) -> Placement {
    Placement { mirrored: ctx.policy.island().is_mirrored() }
}

/// Seek to an absolute position, in seconds.
///
/// Not every source supports it — a live stream or a DRM'd track will refuse —
/// and SMTC reports that as a `false` return rather than an error. The renderer
/// finds out because the next timeline event snaps back to the real position.
#[tauri::command]
pub fn seek(ctx: State<'_, Ctx>, position: f64) -> Result<(), String> {
    ctx.media.control(TransportCmd::Seek(position)).map_err(|e| e.to_string())
}

/// Current window origin, so the renderer can compute a drag delta in screen
/// coordinates without guessing where the window is.
#[tauri::command]
pub fn island_origin(ctx: State<'_, Ctx>) -> (i32, i32) {
    ctx.policy.island().origin()
}

/// Begin a manual drag. Suspends automatic placement until `drag_end`.
#[tauri::command]
pub fn drag_start(ctx: State<'_, Ctx>) {
    ctx.policy.island().begin_drag();
}

/// Abandon a drag without placing — pointer capture lost, or the capsule was
/// hidden mid-gesture.
#[tauri::command]
pub fn drag_cancel(ctx: State<'_, Ctx>) {
    ctx.policy.island().cancel_drag(&ctx.cfg.get());
}

/// Move the capsule during a drag. No animation — see `Island::drag_to`.
#[tauri::command]
pub fn drag_to(ctx: State<'_, Ctx>, x: i32, y: i32) {
    ctx.policy.island().drag_to(x, y);
}

/// Finish a drag: snap to the nearest anchor within the threshold, or keep the
/// drop position. The resulting placement is persisted.
/// `vx`/`vy` are the release velocity in px/ms, used for flick prediction and
/// to match the glide's opening speed to the gesture.
#[tauri::command]
pub fn drag_end(ctx: State<'_, Ctx>, x: i32, y: i32, vx: f64, vy: f64) -> Config {
    let conf = ctx.cfg.get();
    let (dock, free_x, free_y) = ctx.policy.island().end_drag(x, y, (vx, vy), &conf);

    let stored = ctx.cfg.update(|c| {
        c.dock = dock;
        c.free_x = free_x;
        c.free_y = free_y;
    });
    let _ = ctx.app.emit(EVT_CONFIG, &stored);
    stored
}

#[tauri::command]
pub fn volume_state(ctx: State<'_, Ctx>) -> VolumeState {
    ctx.volume.as_ref().map(|v| v.state()).unwrap_or_default()
}

/// Nudge the master volume by `delta` in scalar units (0.0..=1.0 full scale).
///
/// Fire-and-forget: the actor coalesces a wheel burst into one COM call and
/// reports the result through `EVT_VOLUME`, so the caller never waits and the
/// UI never has to guess what landed.
#[tauri::command]
pub fn volume_adjust(ctx: State<'_, Ctx>, delta: f32) {
    if let Some(v) = ctx.volume.as_ref() {
        v.adjust(delta);
    }
}

/// The application the island is currently showing, as a volume target.
///
/// `None` when nothing is playing, or when the playing application is rendering
/// no audio through the default endpoint.
fn media_target(ctx: &Ctx) -> Option<crate::audio::volume::AppTarget> {
    let source = ctx.media.snapshot()?.source;
    let (exe, label) = crate::audio::session::find_by_display_name(&source)?;
    Some(crate::audio::volume::AppTarget { exe, label })
}

/// Scrolling on the island moves the volume of *what the island is showing*,
/// not the system master.
///
/// This is the routing that matters for streaming. The master endpoint is the
/// last stage before the speakers, downstream of anything that captures an
/// application's audio — so lowering it quietens the room and leaves a Discord
/// stream exactly as loud as it was. The session volume is upstream of the mix
/// and is the one a capture can see.
///
/// Falls back to the master only when there is no identifiable application,
/// which keeps the wheel from ever feeling dead.
#[tauri::command]
pub fn volume_adjust_media(ctx: State<'_, Ctx>, delta: f32) {
    let Some(v) = ctx.volume.as_ref() else { return };
    match media_target(&ctx) {
        Some(target) => v.adjust_app(target, delta),
        None => v.adjust(delta),
    }
}

#[tauri::command]
pub fn volume_toggle_mute_media(ctx: State<'_, Ctx>) {
    let Some(v) = ctx.volume.as_ref() else { return };
    match media_target(&ctx) {
        Some(target) => v.toggle_mute_app(target),
        None => v.toggle_mute(),
    }
}

#[tauri::command]
pub fn volume_set(ctx: State<'_, Ctx>, scalar: f32) {
    if let Some(v) = ctx.volume.as_ref() {
        v.set(scalar);
    }
}

#[tauri::command]
pub fn volume_toggle_mute(ctx: State<'_, Ctx>) {
    if let Some(v) = ctx.volume.as_ref() {
        v.toggle_mute();
    }
}

/// Where a saved share card ended up, for the renderer to confirm.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCard {
    pub path: String,
    /// False when the image could not be put on the clipboard. The file is
    /// still written, so this is worth reporting rather than failing.
    pub copied: bool,
}

/// Save a card the renderer composed, and copy it to the clipboard.
///
/// The bytes are a PNG from a `<canvas>`; see `share` for why the drawing does
/// not happen on this side.
#[tauri::command]
pub fn share_card(ctx: State<'_, Ctx>, png: Vec<u8>) -> Result<SavedCard, String> {
    let now = ctx.media.snapshot();
    let (title, artist) = now
        .as_ref()
        .map(|n| (n.title.as_str(), n.artist.as_str()))
        .unwrap_or(("Lumen", ""));

    let path = crate::share::save(&png, title, artist).map_err(|e| format!("{e:#}"))?;

    // A clipboard that is momentarily locked by another application is common
    // and not worth losing the card over, so this is reported, not fatal.
    let copied = match crate::share::copy_to_clipboard(&png) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("share card saved but not copied: {e:#}");
            false
        }
    };

    // Deliberately does not open Explorer. The menu item says "copy", and
    // popping a file browser over whatever the user was doing every time they
    // share a track is a much louder response than the action asked for. The
    // path is logged, and the folder is documented.
    tracing::info!("share card saved to {}", path.display());
    Ok(SavedCard { path: path.display().to_string(), copied })
}

/// Turn the audio capture on or off.
///
/// Driven from the renderer because it is the only side that knows precisely
/// when the bars are on screen — the capture exists solely to feed them, and
/// running it a moment longer than that is pure waste. The host stops it too
/// whenever media vanishes, so a stalled WebView cannot leave it running.
#[tauri::command]
pub fn spectrum_enable(ctx: State<'_, Ctx>, on: bool) {
    let mut guard = match ctx.spectrum.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };

    if !on {
        // Dropping it stops the thread and releases the endpoint.
        if guard.take().is_some() {
            tracing::debug!("spectrum: capture stopped");
        }
        return;
    }
    if guard.is_some() || !ctx.cfg.get().spectrum.enabled {
        return;
    }

    let emitter = ctx.app.clone();
    tracing::debug!("spectrum: capture started");
    *guard = Some(crate::spectrum::Spectrum::start(move |bands| {
        let _ = emitter.emit(EVT_SPECTRUM, bands);
    }));
}

#[tauri::command]
pub fn get_config(ctx: State<'_, Ctx>) -> Config {
    ctx.cfg.get()
}

/// Persist a whole config and re-apply everything that can change live.
///
/// Position, shape, theme and hotkeys take effect immediately. `backdrop` is the
/// one exception — it is applied once at window creation and needs a restart,
/// which the README states.
///
/// Phase 1 has no settings UI; this exists so editing `lumen.config.json` by
/// hand is not the only path, and so Phase 3's settings panel has its surface
/// already defined.
#[tauri::command]
pub fn set_config(ctx: State<'_, Ctx>, config: Config) -> Config {
    let stored = ctx.cfg.set(config);
    ctx.policy.island().apply_config(&stored);

    // `global-hotkey` registers against the message queue of the thread that
    // created the manager, so rebinding has to hop back to the main thread.
    let hotkeys = Arc::clone(&ctx.hotkeys);
    let keys = stored.hotkeys.clone();
    if let Err(e) = ctx.app.run_on_main_thread(move || hotkeys.rebind(&keys)) {
        tracing::warn!("could not rebind hotkeys: {e}");
    }

    let _ = ctx.app.emit(EVT_CONFIG, &stored);
    stored
}

impl RuntimeInfo {
    pub fn build(backdrop: BackdropKind, cfg: &ConfigStore) -> Self {
        Self {
            backdrop,
            config_path: cfg.path().map(|p| p.display().to_string()),
            portable: cfg.origin() == Origin::Portable,
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}
