//! System master volume, via Core Audio's `IAudioEndpointVolume`.
//!
//! # Threading
//!
//! Every call here is COM, and COM must not be touched from a low-level input
//! hook callback (Phase 2's taskbar wheel) — the hook has a ~300 ms budget after
//! which Windows silently unhooks it. So this is an actor: one thread in the
//! multi-threaded apartment owns the endpoint, and callers post deltas to it.
//!
//! # Coalescing
//!
//! One flick of a scroll wheel emits a burst of wheel messages. Applying each
//! one separately would issue a dozen COM round-trips and produce audible
//! stepping, so the worker drains everything queued and applies the *sum* once
//! per wake.

use std::sync::{
    Arc, RwLock,
    mpsc::{self, RecvTimeoutError, Sender},
};
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::Serialize;
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    Media::Audio::{
        EDataFlow, ERole, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator,
        eConsole, eRender,
    },
    System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    },
};

/// How long to gather more wheel input before committing a change.
const COALESCE: Duration = Duration::from_millis(35);

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeState {
    /// 0.0..=1.0, matching the Windows volume slider (which is also scalar).
    pub scalar: f32,
    pub muted: bool,
}

impl Default for VolumeState {
    fn default() -> Self {
        Self { scalar: 0.0, muted: false }
    }
}

/// The result of a per-application change, for the renderer's readout.
///
/// Separate from `VolumeState` because it answers a different question: not "how
/// loud is this machine" but "how loud is *that* program", and the UI has to name
/// which one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVolumeState {
    /// Display name, e.g. "Firefox" — never the executable path.
    pub app: String,
    pub scalar: f32,
    pub muted: bool,
}

/// Which application a per-app command targets.
///
/// `exe` is the full lower-cased image path, which is what actually matches
/// audio sessions; `label` is only ever shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppTarget {
    pub exe: String,
    pub label: String,
}

#[derive(Debug)]
enum Cmd {
    /// Relative change in scalar units; summed across a burst.
    Adjust(f32),
    Set(f32),
    ToggleMute,
    /// Relative change to one application's own volume, not the master.
    AdjustApp(AppTarget, f32),
    ToggleMuteApp(AppTarget),
    /// Report an application's current level without changing it, so the UI
    /// starts out showing the right number instead of the master's.
    PublishByName(String),
    Shutdown,
}

pub struct VolumeControl {
    tx: Sender<Cmd>,
    state: Arc<RwLock<VolumeState>>,
    /// Notified after every applied change so the UI can show a readout.
    on_change: Sink,
    on_app_change: AppSink,
}

impl VolumeControl {
    pub fn start() -> anyhow::Result<Arc<Self>> {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let state = Arc::new(RwLock::new(VolumeState::default()));
        let on_change: Sink = Arc::new(RwLock::new(None));
        let on_app_change: AppSink = Arc::new(RwLock::new(None));

        let me = Arc::new(Self {
            tx,
            state: Arc::clone(&state),
            on_change: Arc::clone(&on_change),
            on_app_change: Arc::clone(&on_app_change),
        });

        let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<()>>();

        std::thread::Builder::new()
            .name("lumen-volume".into())
            .spawn(move || {
                // SAFETY: paired with CoUninitialize at thread exit; this thread
                // owns its apartment for its whole life.
                if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
                    let _ = ready_tx.send(Err(anyhow!("CoInitializeEx failed")));
                    return;
                }

                let mut endpoint = match acquire() {
                    Ok(e) => {
                        let _ = ready_tx.send(Ok(()));
                        e
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        unsafe { CoUninitialize() };
                        return;
                    }
                };

                publish(&endpoint, &state, &on_change);
                run(rx, &mut endpoint, &state, &on_change, &on_app_change);

                unsafe { CoUninitialize() };
            })
            .context("failed to spawn the volume thread")?;

        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .context("volume thread did not report readiness")?
            .context("no default audio endpoint")?;

        Ok(me)
    }

    pub fn state(&self) -> VolumeState {
        self.state.read().map(|g| *g).unwrap_or_default()
    }

    /// Register the sink that receives every applied change.
    pub fn on_change(&self, f: impl Fn(VolumeState) + Send + Sync + 'static) {
        if let Ok(mut guard) = self.on_change.write() {
            *guard = Some(Box::new(f));
        }
    }

    pub fn adjust(&self, delta: f32) {
        let _ = self.tx.send(Cmd::Adjust(delta));
    }

    pub fn set(&self, scalar: f32) {
        let _ = self.tx.send(Cmd::Set(scalar.clamp(0.0, 1.0)));
    }

    pub fn toggle_mute(&self) {
        let _ = self.tx.send(Cmd::ToggleMute);
    }

    /// Register the sink for per-application changes.
    ///
    /// Separate from `on_change` on purpose: the master readout and the per-app
    /// readout are different pieces of UI, and a per-app change must not be
    /// mistaken for the system level having moved.
    pub fn on_app_change(&self, f: impl Fn(AppVolumeState, bool) + Send + Sync + 'static) {
        if let Ok(mut guard) = self.on_app_change.write() {
            *guard = Some(Box::new(f));
        }
    }

    /// Nudge one application's own volume, leaving the master untouched.
    pub fn adjust_app(&self, target: AppTarget, delta: f32) {
        let _ = self.tx.send(Cmd::AdjustApp(target, delta));
    }

    pub fn toggle_mute_app(&self, target: AppTarget) {
        let _ = self.tx.send(Cmd::ToggleMuteApp(target));
    }

    /// Publish an application's current level, identified by display name.
    ///
    /// Resolution needs COM, so it happens on the actor thread rather than the
    /// caller's — `pump_media` runs on the async runtime with no apartment.
    pub fn publish_app_by_name(&self, display: impl Into<String>) {
        let _ = self.tx.send(Cmd::PublishByName(display.into()));
    }
}

impl Drop for VolumeControl {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

type Sink = Arc<RwLock<Option<Box<dyn Fn(VolumeState) + Send + Sync>>>>;

/// Append unless it is already there.
///
/// Coalescing a burst means every distinct request survives and every repeat
/// collapses; the lists are two or three entries long, so a linear scan is the
/// right structure.
fn push_unique<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}
type AppSink = Arc<RwLock<Option<Box<dyn Fn(AppVolumeState, bool) + Send + Sync>>>>;

fn acquire() -> anyhow::Result<IAudioEndpointVolume> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("could not create the device enumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .context("no default render endpoint")?;
        device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .context("could not activate IAudioEndpointVolume")
    }
}

fn read(endpoint: &IAudioEndpointVolume) -> windows::core::Result<VolumeState> {
    unsafe {
        Ok(VolumeState {
            scalar: endpoint.GetMasterVolumeLevelScalar()?,
            muted: endpoint.GetMute()?.as_bool(),
        })
    }
}

fn publish(endpoint: &IAudioEndpointVolume, state: &Arc<RwLock<VolumeState>>, sink: &Sink) {
    let Ok(next) = read(endpoint) else { return };
    if let Ok(mut guard) = state.write() {
        *guard = next;
    }
    if let Ok(guard) = sink.read()
        && let Some(f) = guard.as_ref()
    {
        f(next);
    }
}

fn run(
    rx: mpsc::Receiver<Cmd>,
    endpoint: &mut IAudioEndpointVolume,
    state: &Arc<RwLock<VolumeState>>,
    sink: &Sink,
    app_sink: &AppSink,
) {
    loop {
        // Blocks indefinitely — this is what keeps idle cost at zero.
        let Ok(first) = rx.recv() else { return };

        let mut delta = 0.0_f32;
        let mut absolute: Option<f32> = None;
        let mut toggle = false;
        // Per-application work, accumulated *per target* rather than summed into
        // one number: a burst almost always names the same application, but if
        // the pointer crosses onto another button mid-flick both must still land.
        let mut apps: Vec<(AppTarget, f32)> = Vec::new();
        let mut app_toggles: Vec<AppTarget> = Vec::new();
        let mut publishes: Vec<String> = Vec::new();
        let mut msg = Some(first);

        // Drain the burst produced by one wheel flick.
        let deadline = std::time::Instant::now() + COALESCE;
        loop {
            match msg.take() {
                Some(Cmd::Shutdown) => return,
                Some(Cmd::Adjust(d)) => delta += d,
                Some(Cmd::Set(v)) => absolute = Some(v),
                Some(Cmd::ToggleMute) => toggle = !toggle,
                Some(Cmd::AdjustApp(target, d)) => {
                    match apps.iter_mut().find(|(t, _)| *t == target) {
                        Some((_, sum)) => *sum += d,
                        None => apps.push((target, d)),
                    }
                }
                Some(Cmd::ToggleMuteApp(target)) => push_unique(&mut app_toggles, target),
                // Deduplicated on the way in: a burst of identical requests
                // should collapse into one publish, not one per message.
                Some(Cmd::PublishByName(name)) => push_unique(&mut publishes, name),
                None => {}
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(next) => msg = Some(next),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        for name in publishes {
            if let Some((exe, label)) = super::session::find_by_display_name(&name)
                && let Some(v) = super::session::read(&exe)
            {
                // `false`: nobody asked for this, so it must not reveal the
                // capsule the way a deliberate scroll does.
                notify_app(&AppTarget { exe, label }, v, false, app_sink);
            }
        }

        for (target, sum) in apps {
            apply_app(&target, sum, app_sink);
        }
        for target in app_toggles {
            match super::session::toggle_mute(&target.exe) {
                Ok(v) => notify_app(&target, v, true, app_sink),
                Err(e) => tracing::debug!("per-app mute for {}: {e:#}", target.label),
            }
        }

        // Nothing asked for a master change — skip the COM round-trip entirely
        // rather than re-publishing an unchanged level.
        if delta == 0.0 && absolute.is_none() && !toggle {
            continue;
        }

        if apply(endpoint, delta, absolute, toggle).is_err() {
            // The default device changed under us (headphones plugged in, a
            // device disabled). The old endpoint is dead; re-acquire and retry
            // once rather than going silent for the rest of the session.
            match acquire() {
                Ok(fresh) => {
                    *endpoint = fresh;
                    let _ = apply(endpoint, delta, absolute, toggle);
                }
                Err(e) => {
                    tracing::warn!("lost the audio endpoint and could not re-acquire: {e}");
                    continue;
                }
            }
        }

        publish(endpoint, state, sink);
    }
}

/// Apply one accumulated per-application change and report it.
///
/// A failure here is ordinary rather than exceptional: an application with no
/// live audio session has nothing to adjust. That is logged, not surfaced, and
/// deliberately does *not* fall back to moving the master — silently changing a
/// different volume from the one that was asked for is worse than doing nothing.
fn apply_app(target: &AppTarget, delta: f32, sink: &AppSink) {
    if delta == 0.0 {
        return;
    }
    match super::session::adjust(&target.exe, delta, None) {
        Ok(v) => {
            tracing::debug!(
                "{} volume -> {:.0}% across {} session(s)",
                target.label,
                v.scalar * 100.0,
                v.sessions
            );
            notify_app(target, v, true, sink);
        }
        Err(e) => tracing::debug!("per-app volume for {}: {e:#}", target.label),
    }
}

/// `deliberate` distinguishes a scroll from a passive report: the first
/// deserves a visible readout, the second must stay silent or every track change
/// would pop the capsule open.
fn notify_app(target: &AppTarget, v: super::session::AppVolume, deliberate: bool, sink: &AppSink) {
    if let Ok(guard) = sink.read()
        && let Some(f) = guard.as_ref()
    {
        f(
            AppVolumeState { app: target.label.clone(), scalar: v.scalar, muted: v.muted },
            deliberate,
        );
    }
}

/// The volume change one `APPCOMMAND_VOLUME_UP/DOWN` produces. Fixed by
/// Windows, not by us.
const OSD_STEP: f32 = 0.02;

fn apply(
    endpoint: &IAudioEndpointVolume,
    delta: f32,
    absolute: Option<f32>,
    toggle: bool,
) -> windows::core::Result<()> {
    unsafe {
        if toggle {
            let muted = endpoint.GetMute()?.as_bool();
            endpoint.SetMute(!muted, std::ptr::null())?;
        }

        if let Some(v) = absolute {
            endpoint.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null())?;
            settle_mute(endpoint, v)?;
            return Ok(());
        }

        if delta == 0.0 {
            return Ok(());
        }

        // Show the *native* Windows volume OSD.
        //
        // `SetMasterVolumeLevelScalar` changes the level silently — no flyout,
        // no feedback — which makes a taskbar scroll feel like nothing
        // happened. Posting APPCOMMAND_VOLUME_UP/DOWN to the shell makes
        // Windows draw its own indicator, but that command always moves by
        // exactly 2%.
        //
        // So for any step of 2% or more: apply the excess directly, then let a
        // single AppCommand supply the final 2% *and* the OSD. The total is
        // exactly the requested delta and the flyout appears. Steps below 2%
        // (the Shift fine grain) cannot be expressed this way and stay silent —
        // which is the right trade for a deliberate fine adjustment.
        let osd = delta.abs() >= OSD_STEP && post_volume_app_command(delta > 0.0);
        let manual = if osd { delta - delta.signum() * OSD_STEP } else { delta };

        if manual.abs() > f32::EPSILON {
            let next = (endpoint.GetMasterVolumeLevelScalar()? + manual).clamp(0.0, 1.0);
            endpoint.SetMasterVolumeLevelScalar(next, std::ptr::null())?;
            settle_mute(endpoint, next)?;
        } else if delta > 0.0 && endpoint.GetMute()?.as_bool() {
            // Raising from muted should unmute even when the AppCommand did all
            // the work; otherwise the level climbs in silence.
            endpoint.SetMute(false, std::ptr::null())?;
        }
    }
    Ok(())
}

/// Match Windows' own behaviour of muting at zero and unmuting above it.
///
/// Windows displays the level rounded to the nearest percent, so anything under
/// 0.5% reads as "0%" — and it mutes there. Without this the slider can sit at a
/// displayed 0% while still technically unmuted.
unsafe fn settle_mute(endpoint: &IAudioEndpointVolume, level: f32) -> windows::core::Result<()> {
    unsafe {
        let should_mute = level < 0.005;
        if endpoint.GetMute()?.as_bool() != should_mute {
            endpoint.SetMute(should_mute, std::ptr::null())?;
        }
    }
    Ok(())
}

/// Ask the shell to perform one volume step, which makes it show the OSD.
///
/// The message has to reach the taskbar's task-switcher child; the shell hook
/// message is what Windows itself uses for media/volume keys. Returns false if
/// the shell windows are not where we expect (Explorer restarting, a replacement
/// shell), in which case the caller falls back to adjusting silently.
fn post_volume_app_command(up: bool) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, PostMessageW, RegisterWindowMessageW,
    };
    use windows::core::w;

    // APPCOMMAND_VOLUME_DOWN = 9, APPCOMMAND_VOLUME_UP = 10; HSHELL_APPCOMMAND = 12.
    const HSHELL_APPCOMMAND: usize = 12;
    let command: i32 = if up { 10 } else { 9 };

    unsafe {
        let shell_hook_msg = RegisterWindowMessageW(w!("SHELLHOOK"));
        if shell_hook_msg == 0 {
            return false;
        }

        let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), None) else { return false };
        let Ok(rebar) = FindWindowExW(Some(tray), None, w!("ReBarWindow32"), None) else {
            return false;
        };
        let Ok(tasksw) = FindWindowExW(Some(rebar), None, w!("MSTaskSwWClass"), None) else {
            return false;
        };

        PostMessageW(
            Some(tasksw),
            shell_hook_msg,
            WPARAM(HSHELL_APPCOMMAND),
            LPARAM((command << 16) as isize),
        )
        .is_ok()
    }
}
