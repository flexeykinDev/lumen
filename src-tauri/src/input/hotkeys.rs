//! Global media hotkeys.
//!
//! `global-hotkey` is used directly rather than through
//! `tauri-plugin-global-shortcut`: the plugin routes every press through the
//! WebView and back, which means a keypress cannot work while the renderer is
//! busy, and it adds a permission surface we have no use for.
//!
//! # Threading
//!
//! `GlobalHotKeyManager` registers a Win32 hotkey against the calling thread, so
//! it must be constructed on the thread that pumps messages — Tauri's main
//! thread, from inside `setup`. It must also outlive the app, hence the `Arc`
//! stored in managed state.

use std::{
    str::FromStr,
    sync::{Arc, Mutex, RwLock},
};

use anyhow::{Context, anyhow};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};

use crate::{
    config::Hotkeys,
    media::{MediaBackend, TransportCmd},
};

/// `GlobalHotKeyManager` owns a message-only window and is therefore neither
/// `Send` nor `Sync`, but Tauri's managed state requires both.
///
/// SAFETY: every method on the inner manager is reached through exactly two
/// paths, and both run on the main thread — `HotkeyService::start`, called from
/// Tauri's `setup`, and `ipc::set_config`, which hops via
/// `AppHandle::run_on_main_thread` before touching it. Nothing else in the crate
/// holds a reference. If a third caller is ever added it must use the same hop.
struct MainThreadOnly(GlobalHotKeyManager);

unsafe impl Send for MainThreadOnly {}
unsafe impl Sync for MainThreadOnly {}

/// What a binding does. Not every hotkey is a transport command — session
/// cycling changes *which* player the transport keys talk to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HotkeyAction {
    Transport(TransportCmd),
    CycleSession,
}

pub struct HotkeyService {
    manager: MainThreadOnly,
    /// Kept whole (not just the id) so `rebind` can unregister exactly what it
    /// registered, even if the config changed underneath us.
    registered: Mutex<Vec<(HotKey, HotkeyAction)>>,
    /// Read by the dispatch thread on every press; swapped on rebind.
    bindings: Arc<RwLock<Vec<(u32, HotkeyAction)>>>,
}

impl HotkeyService {
    /// Register the configured bindings and start dispatching presses.
    ///
    /// A binding that fails to register (already owned by another app — F5 is
    /// commonly taken) is logged and skipped; the rest still work. Returning an
    /// error here would mean one busy key disables all media control.
    pub fn start(
        keys: &Hotkeys,
        backend: Arc<dyn MediaBackend>,
    ) -> anyhow::Result<Arc<Self>> {
        let manager = GlobalHotKeyManager::new().context("could not create the hotkey manager")?;

        let service = Arc::new(Self {
            manager: MainThreadOnly(manager),
            registered: Mutex::new(Vec::new()),
            bindings: Arc::new(RwLock::new(Vec::new())),
        });

        service.rebind(keys);
        service.pump(backend);
        Ok(service)
    }

    /// Block on the hotkey channel from a dedicated thread.
    ///
    /// This is a blocking `recv`, not a poll — the thread is asleep and costs
    /// nothing until a key is actually pressed.
    fn pump(&self, backend: Arc<dyn MediaBackend>) {
        let bindings = Arc::clone(&self.bindings);
        let _ = std::thread::Builder::new().name("lumen-hotkeys".into()).spawn(move || {
            let rx = GlobalHotKeyEvent::receiver();
            while let Ok(event) = rx.recv() {
                // Logged unconditionally: without it, "hotkey does nothing" is
                // indistinguishable between the event never arriving and the
                // event arriving with an id that matches no binding. Both look
                // like silence, and they have completely different causes.
                tracing::debug!("hotkey event id={} state={:?}", event.id, event.state);

                // Fire on press only; without this every binding acts twice.
                if event.state != HotKeyState::Pressed {
                    continue;
                }
                let known = bindings.read().ok().map(|b| b.clone()).unwrap_or_default();
                let action = known.iter().find(|(id, _)| *id == event.id).map(|(_, a)| *a);
                let Some(action) = action else {
                    tracing::warn!(
                        "hotkey id {} matched no binding (registered: {:?})",
                        event.id,
                        known.iter().map(|(id, a)| (*id, *a)).collect::<Vec<_>>()
                    );
                    continue;
                };

                let result = match action {
                    HotkeyAction::Transport(cmd) => backend.control(cmd),
                    HotkeyAction::CycleSession => backend.cycle(),
                };
                if let Err(e) = result {
                    tracing::warn!("hotkey {action:?} could not be delivered: {e}");
                }
            }
        });
    }

    /// Install `keys`, replacing whatever is currently registered.
    ///
    /// A binding that fails to register (already owned by another app — F5 is
    /// commonly taken) is logged and skipped; the rest still work. Failing the
    /// whole call would let one busy key disable all media control.
    pub fn rebind(&self, keys: &Hotkeys) {
        let mut held = self.registered.lock().expect("hotkey lock poisoned");
        for (hotkey, _) in held.drain(..) {
            let _ = self.manager.0.unregister(hotkey);
        }

        let wanted = [
            (keys.play_pause.as_str(), HotkeyAction::Transport(TransportCmd::PlayPause)),
            (keys.next.as_str(), HotkeyAction::Transport(TransportCmd::Next)),
            (keys.previous.as_str(), HotkeyAction::Transport(TransportCmd::Previous)),
            (keys.cycle_session.as_str(), HotkeyAction::CycleSession),
        ];

        // An empty spec is how a binding is switched off in the config, and is
        // not worth a warning.
        let requested = wanted.iter().filter(|(spec, _)| !spec.trim().is_empty()).count();

        for (spec, action) in wanted {
            if spec.trim().is_empty() {
                continue;
            }
            match parse(spec) {
                Ok(hotkey) => match self.manager.0.register(hotkey) {
                    Ok(()) => held.push((hotkey, action)),
                    Err(e) => tracing::warn!("hotkey {spec:?} for {action:?} unavailable: {e}"),
                },
                Err(e) => tracing::warn!("hotkey {spec:?} is not a valid binding: {e}"),
            }
        }

        if let Ok(mut b) = self.bindings.write() {
            *b = held.iter().map(|(hk, action)| (hk.id(), *action)).collect();
        }
        tracing::info!("{} of {requested} hotkeys registered", held.len());
    }
}

/// Accept both the crate's canonical form (`F5`, `Ctrl+KeyA`) and the shorthand
/// a human would actually type into a JSON config (`ctrl+a`, `alt+shift+n`).
fn parse(spec: &str) -> anyhow::Result<HotKey> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(anyhow!("empty binding"));
    }

    // Fast path: already in the canonical form.
    if let Ok(hk) = HotKey::from_str(spec) {
        return Ok(hk);
    }

    let mut mods = Modifiers::empty();
    let mut key: Option<String> = None;

    for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "super" | "win" | "meta" | "cmd" => mods |= Modifiers::META,
            _ => key = Some(part.to_owned()),
        }
    }

    let key = key.ok_or_else(|| anyhow!("{spec:?} has modifiers but no key"))?;

    // `Code` names a physical key: a single letter is `KeyA`, a digit `Digit1`.
    let code_name = if key.len() == 1 {
        let c = key.chars().next().expect("length checked");
        if c.is_ascii_alphabetic() {
            format!("Key{}", c.to_ascii_uppercase())
        } else if c.is_ascii_digit() {
            format!("Digit{c}")
        } else {
            key.clone()
        }
    } else {
        // `f5` -> `F5`, `space` -> `Space`.
        let mut chars = key.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
            None => key.clone(),
        }
    };

    let canonical = if mods.is_empty() {
        code_name
    } else {
        let mut prefix = Vec::new();
        if mods.contains(Modifiers::CONTROL) {
            prefix.push("Control");
        }
        if mods.contains(Modifiers::ALT) {
            prefix.push("Alt");
        }
        if mods.contains(Modifiers::SHIFT) {
            prefix.push("Shift");
        }
        if mods.contains(Modifiers::META) {
            prefix.push("Super");
        }
        format!("{}+{}", prefix.join("+"), code_name)
    };

    HotKey::from_str(&canonical).map_err(|e| anyhow!("{spec:?} -> {canonical:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn accepts_the_defaults() {
        for spec in ["F5", "F6", "F7"] {
            assert!(parse(spec).is_ok(), "{spec} should parse");
        }
    }

    #[test]
    fn accepts_human_shorthand() {
        for spec in ["ctrl+alt+n", "Alt+Shift+K", "win+f8", "ctrl+1"] {
            assert!(parse(spec).is_ok(), "{spec} should parse");
        }
    }

    #[test]
    fn rejects_nonsense() {
        assert!(parse("").is_err());
        assert!(parse("ctrl+").is_err());
    }
}
