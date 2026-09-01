//! Lumen — composition root.
//!
//! This is the only file that knows about more than one subsystem. Each of
//! `media`, `window`, `input`, `policy` and `color` is independently testable and
//! never reaches into another; they are wired together exactly once, here.

pub mod appinfo;
pub mod audio;
pub mod color;
pub mod config;
pub mod input;
pub mod ipc;
pub mod media;
pub mod motion;
pub mod policy;
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
    media::{MediaBackend, MediaEvent, smtc::SmtcBackend},
    policy::Policy,
    window::Island,
};

/// The window label the whole app refers to. Matches `tauri.conf.json`.
const ISLAND: &str = "island";

pub fn run() {
    init_tracing();

    // Before anything else claims a hotkey or a screen position.
    let Some(_instance) = single_instance::InstanceGuard::acquire() else {
        tracing::info!("another Lumen is already running; exiting");
        return;
    };

    tune_webview();

    let cfg = Arc::new(ConfigStore::load());
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
                let info = RuntimeInfo::build(island.backdrop_kind(), &cfg);
                let policy = Policy::new(island, Arc::clone(&cfg));

                // Must be constructed on the main thread: `global-hotkey`
                // registers against the calling thread's message queue.
                let hotkeys = HotkeyService::start(&cfg.get().hotkeys, Arc::clone(&media))?;

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

                app.manage(Ctx {
                    media: Arc::clone(&media),
                    policy: Arc::clone(&policy),
                    cfg: Arc::clone(&cfg),
                    hotkeys: Arc::clone(&hotkeys),
                    volume,
                    app: handle.clone(),
                    info,
                });

                build_tray(app, Arc::clone(&policy), Arc::clone(&cfg))?;
                install_mouse_hook(&handle, Arc::clone(&cfg), Arc::clone(&policy));
                pump_media(handle, Arc::clone(&media), Arc::clone(&policy));

                Ok(())
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Lumen application");

    app.run(|_app, event| {
        // The island window is never closed, so an exit request can only come
        // from the tray — let it through untouched.
        if let RunEvent::ExitRequested { .. } = event {
            tracing::info!("shutting down");
        }
    });
}

/// Forward SMTC events to both the visibility policy and the renderer.
///
/// Runs on Tauri's async runtime; `broadcast::Receiver` yields only when Windows
/// actually raises an event, so this task is asleep at idle.
fn pump_media(app: tauri::AppHandle, media: Arc<dyn MediaBackend>, policy: Arc<Policy>) {
    let mut rx = media.subscribe();

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

    // Catch up on anything that arrived between backend start and window ready.
    if let Some(snapshot) = media.snapshot() {
        let _ = app.emit(ipc::EVT_NOW_PLAYING, &snapshot);
        publish_app_volume(&app, &snapshot.source);
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
                        }
                        MediaEvent::TimelineChanged(np) => {
                            let _ = app.emit(ipc::EVT_NOW_PLAYING, np.as_ref());
                        }
                        MediaEvent::Vanished => {
                            let _ = app.emit(ipc::EVT_NOW_PLAYING, Option::<()>::None);
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
            MouseAction::QuitApp => {
                tracing::info!("alt + middle-click on the capsule: exiting");
                app_for_actions.exit(0);
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

fn build_tray(
    app: &tauri::App,
    policy: Arc<Policy>,
    cfg: Arc<ConfigStore>,
) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show now", true, None::<&str>)?;
    let open_cfg = MenuItem::with_id(
        app,
        "config",
        "Open settings file",
        cfg.path().is_some(),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Lumen", true, None::<&str>)?;
    let menu =
        Menu::with_items(app, &[&show, &open_cfg, &PredefinedMenuItem::separator(app)?, &quit])?;

    let tooltip = match cfg.path() {
        Some(p) => format!("Lumen — settings: {}", p.display()),
        None => "Lumen — settings are not persisted (no writable location)".to_owned(),
    };

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::UnknownPath)?;

    TrayIconBuilder::with_id("lumen-tray")
        .icon(icon)
        .menu(&menu)
        .tooltip(&tooltip)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => policy.reveal(),
            "config" => {
                if let Some(path) = cfg.path() {
                    // `/select,` opens Explorer with the file highlighted.
                    let _ = std::process::Command::new("explorer")
                        .arg(format!("/select,{}", path.display()))
                        .spawn();
                }
            }
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
