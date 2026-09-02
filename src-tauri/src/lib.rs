//! Lumen — composition root.
//!
//! This is the only file that knows about more than one subsystem. Each of
//! `media`, `window`, `input`, `policy` and `color` is independently testable and
//! never reaches into another; they are wired together exactly once, here.

pub mod appinfo;
pub mod autostart;
pub mod audio;
pub mod color;
pub mod config;
pub mod i18n;
pub mod input;
pub mod ipc;
pub mod lyrics;
pub mod media;
pub mod motion;
pub mod policy;
pub mod net;
pub mod presence;
pub mod share;
pub mod smart_pause;
pub mod spectrum;
pub mod stats;
pub mod update;
pub mod single_instance;
pub mod util;
pub mod window;

use std::sync::Arc;

use tauri::{
    Emitter, Manager, RunEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

use crate::{
    config::ConfigStore,
    input::{HotkeyService, MouseAction, MouseHook},
    ipc::{Ctx, RuntimeInfo},
    media::{MediaBackend, MediaEvent, NowPlaying, smtc::SmtcBackend},
    policy::Policy,
    window::Island,
};

/// The window label the whole app refers to. Matches `tauri.conf.json`.
const ISLAND: &str = "island";

/// The settings window. Built on demand — see [`open_settings`].
const SETTINGS: &str = "settings";

pub fn run() {
    init_tracing();

    // Before anything else claims a hotkey or a screen position.
    let Some(_instance) = single_instance::InstanceGuard::acquire() else {
        tracing::info!("another Lumen is already running; exiting");
        return;
    };

    tune_webview();

    let cfg = Arc::new(ConfigStore::load());
    // Reconciled every launch: a portable exe gets moved, and a Run entry
    // pointing at the old path either does nothing or starts whatever took its
    // place. See `autostart::sync`.
    autostart::sync(cfg.get().start_with_windows);
    tracing::info!(
        "config: {} ({:?})",
        cfg.path().map(|p| p.display().to_string()).unwrap_or_else(|| "<memory>".into()),
        cfg.origin()
    );

    // SMTC is a hard dependency: without it there is nothing to show. Fail loudly
    // and early rather than presenting an empty capsule forever.
    let media: Arc<dyn MediaBackend> = match SmtcBackend::start() {
        Ok(backend) => backend,
        Err(e) => {
            tracing::error!("could not reach the Windows media session manager: {e:#}");
            eprintln!("Lumen could not start: {e:#}");
            return;
        }
    };

    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ipc::runtime_info,
            ipc::now_playing,
            ipc::sessions,
            ipc::transport,
            ipc::set_hover,
            ipc::island_state,
            ipc::placement,
            ipc::get_config,
            ipc::set_config,
            ipc::open_settings,
            ipc::restart,
            ipc::check_update,
            ipc::stats_top,
            ipc::stats_artists,
            ipc::stats_summary,
            ipc::stats_clear,
            ipc::open_external,
            ipc::seek,
            ipc::island_origin,
            ipc::drag_start,
            ipc::drag_cancel,
            ipc::drag_to,
            ipc::drag_end,
            ipc::cycle_session,
            ipc::focus_session,
            ipc::volume_state,
            ipc::volume_adjust,
            ipc::volume_adjust_media,
            ipc::volume_toggle_mute_media,
            ipc::volume_set,
            ipc::volume_toggle_mute,
            ipc::share_card,
            ipc::spectrum_enable,
        ])
        .setup({
            let cfg = Arc::clone(&cfg);
            let media = Arc::clone(&media);
            move |app| {
                let handle = app.handle().clone();
                let window = app
                    .get_webview_window(ISLAND)
                    .ok_or("the island window is missing from tauri.conf.json")?;

                // Opt-in renderer inspection: the island has no chrome to right
                // click, so this is the only way in.
                #[cfg(debug_assertions)]
                if std::env::var_os("LUMEN_DEVTOOLS").is_some() {
                    window.open_devtools();
                }

                let island = Island::attach(handle.clone(), window, Arc::clone(&cfg))?;
                apply_zoom(&handle, cfg.get().ui_scale);
                let info = RuntimeInfo::build(island.backdrop_kind(), &cfg);
                let policy = Policy::new(island, Arc::clone(&cfg));

                // A machine with no audio endpoint is unusual but real (RDP
                // sessions, disabled devices). Volume then simply does nothing
                // rather than taking the whole app down with it.
                let volume = match audio::VolumeControl::start() {
                    Ok(v) => {
                        let emitter = handle.clone();
                        v.on_change(move |state| {
                            let _ = emitter.emit(ipc::EVT_VOLUME, state);
                        });

                        // A per-app change has no native indicator — Windows only
                        // draws its flyout for the master level — so the capsule
                        // is the only thing that can say what happened. It is
                        // topmost, which is what makes it readable over the
                        // full-screen window this most often targets.
                        let emitter = handle.clone();
                        let flash = Arc::clone(&policy);
                        v.on_app_change(move |state, deliberate| {
                            let _ = emitter.emit(ipc::EVT_APP_VOLUME, &state);
                            // Only a gesture reveals the capsule. A passive
                            // report at a track change must stay silent.
                            if deliberate {
                                flash.flash(std::time::Duration::from_millis(1600));
                            }
                        });
                        Some(v)
                    }
                    Err(e) => {
                        tracing::warn!("volume control unavailable: {e:#}");
                        None
                    }
                };

                // Must be constructed on the main thread: `global-hotkey`
                // registers against the calling thread's message queue. Built
                // here, after the volume actor, because a binding may need any
                // of the three subsystems and this is the first point where all
                // of them exist.
                let hotkeys = {
                    let media = Arc::clone(&media);
                    let policy = Arc::clone(&policy);
                    let cfg = Arc::clone(&cfg);
                    let volume = volume.clone();
                    HotkeyService::start(&cfg.get().hotkeys, move |action| {
                        use input::HotkeyAction;
                        match action {
                            HotkeyAction::Transport(cmd) => {
                                if let Err(e) = media.control(cmd) {
                                    tracing::warn!("hotkey {cmd:?} failed: {e}");
                                }
                            }
                            HotkeyAction::CycleSession => {
                                if let Err(e) = media.cycle() {
                                    tracing::warn!("hotkey session switch failed: {e}");
                                }
                            }
                            // Same routing as the island wheel: the playing
                            // application's own level, falling back to the
                            // master when it has no audio session.
                            HotkeyAction::Volume(direction) => {
                                let Some(v) = volume.as_ref() else { return };
                                let conf = cfg.get();
                                let delta = conf.volume_step * direction as f32;
                                match media
                                    .snapshot()
                                    .map(|np| np.source)
                                    .and_then(|source| {
                                        audio::session::find_by_display_name(&source)
                                    })
                                    .map(|(exe, label)| audio::volume::AppTarget { exe, label })
                                {
                                    Some(target) => v.adjust_app(target, delta),
                                    None => v.adjust(delta),
                                }
                            }
                            HotkeyAction::ToggleVisible => {
                                if policy.visible() {
                                    policy.conceal();
                                } else {
                                    policy.reveal();
                                }
                            }
                            // Persisted, so "always open" survives a restart
                            // rather than being a mode you have to re-arm.
                            HotkeyAction::TogglePinned => {
                                let next = !policy.pinned();
                                policy.set_pinned(next);
                                cfg.update(|c| c.always_expanded = next);
                                tracing::info!("panel pinned open: {next}");
                            }
                        }
                    })?
                };

                let stats = Arc::new(stats::Stats::load(cfg.path()));

                app.manage(Ctx {
                    media: Arc::clone(&media),
                    policy: Arc::clone(&policy),
                    cfg: Arc::clone(&cfg),
                    hotkeys: Arc::clone(&hotkeys),
                    volume,
                    app: handle.clone(),
                    info,
                    spectrum: std::sync::Mutex::new(None),
                    boost: Some(Arc::new(audio::boost::Supervisor::default())),
                    stats: Arc::clone(&stats),
                });

                // Presence is published *as* a Discord application, so without an
                // id there is nothing to publish as — and no id can be supplied
                // by default without putting someone else's name on the user's
                // profile. Say so once rather than failing silently.
                let discord = cfg.get().discord;
                let presence = if !discord.enabled {
                    None
                } else if discord.application_id.trim().is_empty() {
                    tracing::info!(
                        "Discord presence is on but has no applicationId; \
                         create one at https://discord.com/developers/applications \
                         and put its client id in lumen.config.json"
                    );
                    None
                } else {
                    Some(Arc::new(presence::Presence::start(discord.clone())))
                };

                // Failure is never fatal: a machine or policy that refuses the
                // session registration should still get a working island.
                let pause_conf = cfg.get().smart_pause;
                let smart_pause = if pause_conf.enabled {
                    match smart_pause::SmartPause::start(
                        Arc::clone(&media),
                        pause_conf.resume_on_unlock,
                    ) {
                        Ok(watcher) => Some(watcher),
                        Err(e) => {
                            tracing::warn!("smart pause unavailable: {e:#}");
                            None
                        }
                    }
                } else {
                    None
                };
                if let Some(watcher) = smart_pause {
                    // Held for the process lifetime; dropping it unregisters.
                    app.manage(watcher);
                }

                // The only network feature, so it stays off unless asked for —
                // enabling it sends what you are playing to a third party.
                let lyrics_conf = cfg.get().lyrics;
                let lyrics = if lyrics_conf.enabled {
                    let emitter = handle.clone();
                    Some(Arc::new(lyrics::LyricsService::start(
                        lyrics_conf.genius_fallback,
                        lyrics_conf.estimated_offset_ms as f64 / 1000.0,
                        move |l| {
                        let _ = emitter.emit(ipc::EVT_LYRICS, &l);
                    })))
                } else {
                    None
                };

                // First launch: introduce the features that change how the
                // machine behaves, rather than waiting to be discovered by
                // accident. Once, ever — the answer is in the config.
                if !cfg.get().onboarded {
                    open_settings(&handle);
                }

                // The settings window normally arrives via the tray, which is
                // awkward to reach from a script or a shortcut. Kept in release
                // too: it costs one env lookup and makes the panel reachable
                // without hunting for the tray icon.
                if std::env::var_os("LUMEN_SETTINGS").is_some() {
                    open_settings(&handle);
                }

                build_tray(app, Arc::clone(&policy), Arc::clone(&cfg))?;
                install_mouse_hook(&handle, Arc::clone(&cfg), Arc::clone(&policy));
                pump_media(handle, Arc::clone(&media), Arc::clone(&policy), presence, lyrics);

                Ok(())
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Lumen application");

    app.run(|app, event| {
        // The island window is never closed, so an exit request can only come
        // from the tray — let it through untouched.
        if let RunEvent::ExitRequested { .. } = event {
            tracing::info!("shutting down");
            // Boost holds the playing application muted. Leaving on exit
            // without lifting that would leave someone with a silent Spotify
            // and no idea which app did it.
            if let Some(ctx) = app.try_state::<Ctx>() {
                if let Some(boost) = ctx.boost.as_ref() {
                    boost.stop();
                }
                // Credit whatever the current track has earned before leaving.
                ctx.stats.flush();
            }
        }
    });
}

/// Forward SMTC events to both the visibility policy and the renderer.
///
/// Runs on Tauri's async runtime; `broadcast::Receiver` yields only when Windows
/// actually raises an event, so this task is asleep at idle.
fn pump_media(
    app: tauri::AppHandle,
    media: Arc<dyn MediaBackend>,
    policy: Arc<Policy>,
    presence: Option<Arc<presence::Presence>>,
    lyrics: Option<Arc<lyrics::LyricsService>>,
) {
    let mut rx = media.subscribe();

    /// What Discord should be showing for a given media state.
    ///
    /// Paused is a deliberate choice rather than an oversight: a profile that
    /// still says "listening" hours after the music stopped is worse than one
    /// that says nothing, so a pause clears it unless asked otherwise.
    ///
    /// A hidden source clears the presence rather than freezing it — leaving the
    /// last public track up while something private plays would be the opposite
    /// of what hiding it is for.
    fn for_discord(np: &NowPlaying, cfg: &config::Discord) -> Option<NowPlaying> {
        let worth_showing = np.state == media::PlaybackState::Playing
            || (cfg.show_while_paused && np.state == media::PlaybackState::Paused);
        (worth_showing && cfg.publishes(&np.source)).then(|| np.clone())
    }

    /// Ask the volume actor to report the playing app's own level.
    ///
    /// Without this the expanded volume bar shows the system master until the
    /// first scroll, which is a different number from the one the wheel moves.
    fn publish_app_volume(app: &tauri::AppHandle, source: &str) {
        if let Some(ctx) = app.try_state::<Ctx>()
            && let Some(volume) = ctx.volume.as_ref()
        {
            volume.publish_app_by_name(source);
        }
    }

    let discord = app
        .try_state::<Ctx>()
        .map(|ctx| ctx.cfg.get().discord)
        .unwrap_or_default();

    // Catch up on anything that arrived between backend start and window ready.
    if let Some(snapshot) = media.snapshot() {
        let _ = app.emit(ipc::EVT_NOW_PLAYING, &snapshot);
        publish_app_volume(&app, &snapshot.source);
        apply_boost(&app, Some(&snapshot));
        observe_stats(&app, Some(&snapshot));
        if let Some(p) = presence.as_ref() {
            p.update(for_discord(&snapshot, &discord));
        }
        if let Some(l) = lyrics.as_ref() {
            l.request(&snapshot);
        }
        policy.on_media(&MediaEvent::TrackChanged(Box::new(snapshot)));
    }

    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    policy.on_media(&event);
                    match &event {
                        MediaEvent::TrackChanged(np) | MediaEvent::PlaybackChanged(np) => {
                            let _ = app.emit(ipc::EVT_NOW_PLAYING, np.as_ref());
                            // The source can change under us (a switch to
                            // another player), so re-read rather than assuming
                            // the first reading still applies.
                            publish_app_volume(&app, &np.source);
                            // Follows the source: pausing releases the mute,
                            // and switching player moves the boost with it.
                            apply_boost(&app, Some(np));
                            observe_stats(&app, Some(np));
                            // One lookup per track: the service ignores repeats
                            // for anything it has already fetched.
                            if let Some(l) = lyrics.as_ref() {
                                l.request(np);
                            }
                            if let Some(p) = presence.as_ref() {
                                p.update(for_discord(np, &discord));
                            }
                        }
                        MediaEvent::TimelineChanged(np) => {
                            let _ = app.emit(ipc::EVT_NOW_PLAYING, np.as_ref());
                            // Sent so a seek corrects Discord's running clock.
                            // The actor de-duplicates and rate-limits, so the
                            // steady stream of timeline events costs nothing.
                            if let Some(p) = presence.as_ref() {
                                p.update(for_discord(np, &discord));
                            }
                        }
                        MediaEvent::Vanished => {
                            let _ = app.emit(ipc::EVT_NOW_PLAYING, Option::<()>::None);
                            apply_boost(&app, None);
                            observe_stats(&app, None);
                            if let Some(p) = presence.as_ref() {
                                p.update(None);
                            }
                        }
                        // Forwarded so the renderer knows whether a switcher
                        // affordance is worth showing at all.
                        MediaEvent::SessionsChanged(list) => {
                            let _ = app.emit(ipc::EVT_SESSIONS, list);
                        }
                    }
                }
                // A slow renderer can fall behind a burst; skipping to the
                // newest state is exactly the right recovery for now-playing.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("renderer lagged {n} media events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

/// Install the global mouse hook, if the config wants it.
///
/// Failure is never fatal: a machine or policy that refuses the hook should
/// still get a working island, minus the two mouse conveniences.
fn install_mouse_hook(app: &tauri::AppHandle, cfg: Arc<ConfigStore>, policy: Arc<Policy>) {
    let conf = cfg.get();
    if !conf.mouse.enabled {
        tracing::info!("mouse hook disabled in config");
        return;
    }

    let island_hwnd = policy.island().hwnd();
    let app_for_actions = app.clone();
    let policy_for_actions = Arc::clone(&policy);
    let cfg_for_actions = Arc::clone(&cfg);

    let close_button = match conf.mouse.taskbar_close_button {
        config::TaskbarCloseButton::None => input::mouse_hook::CLOSE_NONE,
        config::TaskbarCloseButton::Middle => input::mouse_hook::CLOSE_MIDDLE,
        config::TaskbarCloseButton::Right => input::mouse_hook::CLOSE_RIGHT,
    };

    let settings = input::mouse_hook::MouseSettings {
        volume_step: conf.volume_step,
        taskbar_wheel: conf.mouse.taskbar_wheel_volume,
        taskbar_wheel_covered: conf.mouse.taskbar_wheel_over_fullscreen,
        middle_click: conf.mouse.middle_click_hides,
        alt_middle_quit: conf.mouse.alt_middle_quits,
        taskbar_close: close_button,
    };

    let hook = MouseHook::start(
        island_hwnd,
        settings,
        move |action| match action {
            // Runs on the hook's *action* thread, never in the callback. That
            // thread holds an STA, which UI Automation requires.
            MouseAction::Volume { delta, x, y, covered } => {
                let Some(ctx) = app_for_actions.try_state::<Ctx>() else { return };
                let Some(volume) = ctx.volume.as_ref() else { return };

                // Which application, if any, this gesture is aimed at.
                //
                //  - over a button: whatever that button represents
                //  - over a covered bar: the window drawing over it
                //  - over empty bar: nothing, so the master moves instead
                let pt = windows::Win32::Foundation::POINT { x, y };
                let target = if !cfg_for_actions.get().mouse.taskbar_wheel_per_app {
                    None
                } else if covered {
                    input::taskbar_target::window_app_at(pt, island_hwnd)
                } else {
                    input::taskbar_target::app_at(pt)
                };

                match target {
                    Some(app) => volume.adjust_app(app, delta),
                    None => volume.adjust(delta),
                }
            }
            MouseAction::HideIsland => {
                tracing::info!("middle-click on the capsule: hiding");
                policy_for_actions.conceal();
            }
            // The escape hatch: out now, with nothing written on the way.
            //
            // `app.exit` runs Tauri's shutdown, which persists window state and
            // gives every subsystem a chance to write. That is the wrong
            // behaviour for a gesture whose whole purpose is "stop, and forget
            // whatever just happened" — so this leaves by the shortest path
            // instead. Settings are saved as they change, so nothing the user
            // deliberately set is lost.
            //
            // The one thing that must still happen is undoing what Lumen did to
            // *other* applications: boost holds the playing app at two percent,
            // and exiting over that would leave someone with a near-silent
            // Spotify and no clue which program did it.
            MouseAction::QuitApp => {
                tracing::info!("alt + middle-click on the capsule: killing this instance");
                if let Some(ctx) = app_for_actions.try_state::<Ctx>()
                    && let Some(boost) = ctx.boost.as_ref()
                {
                    boost.stop();
                }
                std::process::exit(0);
            }
            MouseAction::CloseTaskbarApp(x, y) => {
                use input::taskbar_target::{CloseOutcome, close_at};
                match close_at(windows::Win32::Foundation::POINT { x, y }) {
                    CloseOutcome::Closed { process, windows } => {
                        tracing::info!("closed {process} ({windows} window(s))");
                    }
                    // Logged at info, not debug: when this feature appears to do
                    // nothing, the reason is the whole story.
                    CloseOutcome::Refused(why) => tracing::info!("taskbar close declined: {why}"),
                }
            }
        },
    );

    match hook {
        // Held for the process lifetime; dropping it unhooks.
        Ok(hook) => {
            app.manage(hook);
        }
        Err(e) => tracing::warn!("mouse hook unavailable: {e:#}"),
    }
}

/// Feed the listening history whatever the media backend just reported.
///
/// Called from the same places as the boost: a track change, a play/pause, and
/// the session vanishing are exactly the moments the accumulated time has to be
/// banked or credited.
fn observe_stats(app: &tauri::AppHandle, np: Option<&media::NowPlaying>) {
    if let Some(ctx) = app.try_state::<Ctx>() {
        ctx.stats.observe(np);
    }
}

/// Start, stop or retune the boost for whatever is playing.
///
/// The display name SMTC reports is not a process, so the application has to be
/// resolved through its audio session — the same bridge the taskbar wheel uses
/// to find a per-app volume. Called from the media loop and from a settings
/// change, because either can make the answer different.
pub fn apply_boost(app: &tauri::AppHandle, np: Option<&media::NowPlaying>) {
    let Some(ctx) = app.try_state::<Ctx>() else { return };
    let Some(boost) = ctx.boost.as_ref() else { return };
    let conf = ctx.cfg.get();

    let playing = np.is_some_and(|np| np.state == media::PlaybackState::Playing);
    let exe = np
        .filter(|_| playing && conf.boost.enabled)
        .and_then(|np| audio::session::find_by_display_name(&np.source))
        .map(|(exe, _)| exe);

    boost.apply(exe.as_deref(), playing, conf.boost.enabled, conf.boost.settings());
}

/// Scale what every window draws.
///
/// Separate from the island's own geometry: that decides how big the capsule's
/// *window* is, this decides how big its contents are. Both read `ui_scale`, and
/// they have to move together or the capsule's text stops fitting the capsule.
pub fn apply_zoom(app: &tauri::AppHandle, ui_scale: f32) {
    let factor = f64::from(ui_scale).clamp(0.75, 2.0);
    for (label, window) in app.webview_windows() {
        if let Err(e) = window.set_zoom(factor) {
            tracing::debug!("could not zoom {label}: {e}");
        }
    }
}

/// Show the settings window, creating it the first time.
///
/// Deliberately not declared in `tauri.conf.json`: a window listed there is
/// created at startup even with `visible: false`, and a second WebView2 costs
/// tens of megabytes to sit unread behind a menu item nobody has clicked.
/// Closing it destroys it and gives that back, so this is the whole lifecycle.
pub fn open_settings(app: &tauri::AppHandle) {
    if let Some(existing) = app.get_webview_window(SETTINGS) {
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return;
    }

    let built = tauri::WebviewWindowBuilder::new(
        app,
        SETTINGS,
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("Lumen Settings")
    .inner_size(940.0, 680.0)
    .min_inner_size(760.0, 540.0)
    // Its own chrome, so it can be the same glass object as the capsule. The
    // transparency is what lets the rounded corners actually be round.
    .decorations(false)
    .transparent(true)
    .resizable(true)
    .center()
    .build();

    match built {
        Ok(window) => {
            if let Some(ctx) = app.try_state::<Ctx>() {
                let _ = window.set_zoom(f64::from(ctx.cfg.get().ui_scale).clamp(0.75, 2.0));
            }
            let _ = window.set_focus();
        }
        Err(e) => tracing::warn!("could not open settings: {e}"),
    }
}

fn build_tray(
    app: &tauri::App,
    policy: Arc<Policy>,
    cfg: Arc<ConfigStore>,
) -> tauri::Result<()> {
    let conf = cfg.get();

    // Four items, and every one of them does something the settings window
    // cannot. Toggles that also live in Settings used to sit here too, which
    // meant two places to change lyrics, spectrum, smart pause and autostart —
    // and two places to keep in step. The window owns those now.
    let lang = i18n::Resolved::of(conf.language);
    let text = |key| i18n::tray(lang, key);

    let show = MenuItem::with_id(app, "show", text(i18n::Key::Show), true, None::<&str>)?;
    let settings =
        MenuItem::with_id(app, "settings", text(i18n::Key::Settings), true, None::<&str>)?;
    let share = MenuItem::with_id(app, "share", text(i18n::Key::Share), true, None::<&str>)?;

    let quit = MenuItem::with_id(app, "quit", text(i18n::Key::Quit), true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&show, &share, &settings, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let tooltip = match cfg.path() {
        Some(p) => format!("{} {}", text(i18n::Key::TooltipAt), p.display()),
        None => text(i18n::Key::TooltipNone).to_owned(),
    };

    // A dedicated mark rather than the application icon.
    //
    // The tray renders at 16 px (or 24 at 150% scaling), and the app icon has a
    // house, two chevrons and a landscape in it — roughly 250 pixels to carry
    // four shapes, so at that size it resolves to mush. This is the crescent on
    // its own, which is the one element with a silhouette big enough to survive.
    // Embedded at compile time so the portable exe stays a single file.
    let _ = app.default_window_icon();

    TrayIconBuilder::with_id("lumen-tray")
        .icon(tauri::include_image!("icons/tray.png"))
        .menu(&menu)
        .tooltip(&tooltip)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => policy.reveal(),
            // Only the renderer can draw the card, so the tray just asks.
            "share" => {
                let _ = app.emit(ipc::EVT_SHARE_REQUEST, ());
            }
            "settings" => open_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Trim what WebView2 starts up.
///
/// This must run before the first webview is created. It does not get us to the
/// <20 MB the brief asks for — see ARCHITECTURE.md §3 — but it removes the
/// out-of-process UI helpers, the extra renderer, and the background network
/// traffic that a single always-on capsule has no use for.
fn tune_webview() {
    const ARGS: &str = concat!(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection ",
        "--renderer-process-limit=1 ",
        "--disable-background-networking ",
        "--disable-sync ",
        "--no-pings"
    );
    // Respect an explicit override so the setting stays debuggable.
    if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_none() {
        // SAFETY: single-threaded at this point — called first thing in `run`,
        // before any thread that might read the environment is spawned.
        unsafe { std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", ARGS) };
    }
}

fn init_tracing() {
    // Release builds stay silent unless LUMEN_LOG asks otherwise, so the tray app
    // costs nothing in I/O at idle.
    let default = if cfg!(debug_assertions) { "lumen=debug" } else { "lumen=warn" };
    let filter = std::env::var("LUMEN_LOG").unwrap_or_else(|_| default.to_owned());

    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
        .try_init();
}
