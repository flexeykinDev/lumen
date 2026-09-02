//! Portable configuration.
//!
//! Preferred location is `lumen.config.json` beside the executable, so the app
//! stays a drop-anywhere single file. If that directory is not writable (Program
//! Files, a read-only share, a locked-down USB stick) we fall back to
//! `%APPDATA%\Lumen\` and record which one won so the tray can say so.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Stock Windows 11 flyout corners via DWM. Anti-aliased, ~8 px radius.
    ///
    /// The aliases keep configs written against the earlier region-based
    /// implementation loading; DWM exposes no custom radius, so both former
    /// values land here. See `window::backdrop::set_corners`.
    #[serde(alias = "pill", alias = "native")]
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackdropPref {
    /// Wallpaper-tinted and near-opaque; ignores windows behind it by design.
    Mica,
    /// See-through: samples whatever is actually behind the window.
    Acrylic,
    /// Acrylic where available, then Mica, then a plain CSS panel.
    /// Acrylic is preferred because a floating capsule should be glass — see
    /// `window::backdrop::apply`.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonitorPick {
    Primary,
    /// The monitor the cursor is currently on, re-evaluated on every reveal.
    Cursor,
}

/// Where on the chosen monitor the island sits.
///
/// Every mode pins one edge of the capsule; the capsule grows away from it, so
/// the pinned edge never moves during an expand. Bottom docks grow upward, top
/// docks grow downward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DockMode {
    /// Centred above the taskbar. The default, and the reason the app exists.
    TaskbarCenter,
    BottomLeft,
    BottomRight,
    TopLeft,
    TopRight,
    /// Dropped somewhere that is not near an anchor. The position lives in
    /// `freeX`/`freeY`.
    Free,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotkeys {
    pub play_pause: String,
    pub next: String,
    pub previous: String,
    /// Switch which media source the island follows. Empty string disables it.
    pub cycle_session: String,
}

impl Default for Hotkeys {
    /// F5 / F6 / F7 laid out left-to-right as previous / play-pause / next, so
    /// the keys sit in the same spatial order as the on-screen buttons. The
    /// earlier mapping put play-pause on F5 and previous on F6, which meant the
    /// physical layout ran backwards against the UI.
    fn default() -> Self {
        Self {
            previous: "F5".into(),
            play_pause: "F6".into(),
            next: "F7".into(),
            // Ctrl+F6 sits on the play/pause key: same hand position, and the
            // modifier keeps it clear of the bare function keys other apps use.
            cycle_session: "Ctrl+F6".into(),
        }
    }
}

/// Which button closes the app under a taskbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskbarCloseButton {
    /// Off. Taskbar clicks behave exactly as Windows intends.
    None,
    /// Middle-click. Its default (launch another instance) is rarely wanted, so
    /// this is the least disruptive button to repurpose.
    Middle,
    /// Right-click. Note this replaces the jump list, which is a genuinely
    /// useful menu — choose it deliberately.
    Right,
}

/// Global mouse behaviours, all individually switchable.
///
/// These install a system-wide low-level hook, so each one is separately
/// disable-able: a user who wants the taskbar wheel but not middle-click (or
/// neither) should not have to accept the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mouse {
    /// Install the hook at all. `false` means no system-wide hook is created.
    pub enabled: bool,
    /// Wheel over the Windows taskbar changes volume.
    pub taskbar_wheel_volume: bool,
    /// Wheel over a taskbar *button* changes that application's own volume
    /// rather than the system master.
    ///
    /// This is not a convenience: the master slider is applied at the audio
    /// endpoint, after anything that captures an application's sound for a
    /// stream or a recording has already taken it. Turning the master down makes
    /// a stream no quieter for its viewers; turning the application down does.
    /// See `audio::session`.
    ///
    /// Empty stretches of the bar still move the master, so the original
    /// behaviour is never lost — it just moves to where there is no application
    /// to be more specific about.
    pub taskbar_wheel_per_app: bool,
    /// Keep the taskbar wheel working where the bar *would* be when a
    /// full-screen window is covering it.
    ///
    /// The window covering the bar is then the volume target, which is what
    /// makes this usable during a game: the taskbar is exactly what a borderless
    /// full-screen title hides, and a game is when reaching for the mixer is
    /// least welcome.
    pub taskbar_wheel_over_fullscreen: bool,
    /// Middle-click on the capsule hides it. Only ever acts on Lumen's own
    /// window; middle-click everywhere else is passed through untouched.
    pub middle_click_hides: bool,
    /// Alt + middle-click on the capsule quits Lumen.
    pub alt_middle_quits: bool,
    /// Close the application belonging to a taskbar button.
    ///
    /// Defaults to middle-click rather than right-click: right-click opens the
    /// jump list, which is worth keeping, whereas middle-click's default of
    /// launching a second instance rarely is. Set to `"right"` to swap, or
    /// `"none"` to disable.
    pub taskbar_close_button: TaskbarCloseButton,
}

impl Default for Mouse {
    fn default() -> Self {
        Self {
            enabled: true,
            taskbar_wheel_volume: true,
            taskbar_wheel_per_app: true,
            taskbar_wheel_over_fullscreen: true,
            middle_click_hides: true,
            alt_middle_quits: true,
            taskbar_close_button: TaskbarCloseButton::Middle,
        }
    }
}

/// Discord Rich Presence.
///
/// Off unless an `applicationId` is supplied, because there is no sensible
/// default: presence is published *as* a Discord application, and the name and
/// artwork people will see belong to whoever created it. A shared id baked in
/// here would show someone else's branding on your profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Discord {
    pub enabled: bool,
    /// Client id from <https://discord.com/developers/applications>. The
    /// application's *name* is what renders as "Listening to …".
    pub application_id: String,
    /// Keep the presence up while paused, rather than clearing it.
    pub show_while_paused: bool,
}

impl Default for Discord {
    fn default() -> Self {
        Self { enabled: true, application_id: String::new(), show_while_paused: false }
    }
}

/// Time-synced lyrics from LRCLIB.
///
/// **Off by default, and deliberately so.** This is the only feature that talks
/// to the network: enabling it sends the artist, title, album and duration of
/// whatever you play to `lrclib.net`. That is a fair trade for lyrics and not
/// one to make on someone's behalf without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lyrics {
    pub enabled: bool,
    /// Fall back to scraping Genius when LRCLIB has no timed lyrics.
    ///
    /// Separate from enabled because it is a different kind of thing: Genius
    /// has no lyrics API, so this reads their web page and will break whenever
    /// that page changes. The timings it produces are guesses.
    pub genius_fallback: bool,
}

impl Default for Lyrics {
    fn default() -> Self {
        Self { enabled: false, genius_fallback: true }
    }
}

/// Pause playback when the machine is locked.
///
/// Only ever resumes what it paused itself, and only if the session is
/// untouched on return — coming back to music you had deliberately stopped is
/// worse than coming back to silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmartPause {
    pub enabled: bool,
    /// Start it playing again on unlock. Off leaves the pause in place.
    pub resume_on_unlock: bool,
}

impl Default for SmartPause {
    fn default() -> Self {
        Self { enabled: true, resume_on_unlock: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub shape: Shape,
    pub backdrop: BackdropPref,
    pub theme: Theme,
    pub monitor: MonitorPick,
    pub dock: DockMode,
    /// Gap between the island and the edge it is docked against, in *logical*
    /// pixels — scaled by the destination monitor's DPI at placement time, so
    /// the visual gap is identical at 100% and 200%.
    pub taskbar_gap: i32,
    /// Inset from the side of the work area for the corner docking modes, in
    /// logical pixels. Ignored by `taskbar-center`.
    pub edge_margin: i32,
    /// Free position, in *logical* pixels from the work-area top-left of the
    /// chosen monitor. Relative rather than absolute so the island lands
    /// sensibly after a resolution change. Only read when `dock` is `free`.
    pub free_x: i32,
    pub free_y: i32,
    /// How near an anchor a drop has to land to snap to it, in logical pixels.
    /// Zero disables snapping, so a drag always leaves a free position.
    pub snap_threshold: i32,
    /// Briefly expand the panel when a new track starts.
    pub auto_expand_on_track_change: bool,
    /// How long that automatic expansion lasts before collapsing back.
    pub auto_expand_ms: u64,
    /// Keep the pill on screen while paused (false hides it until playback resumes).
    pub show_while_paused: bool,
    /// How much one wheel notch moves the master volume, in scalar units
    /// (0.02 = 2%, matching the granularity of the Windows volume flyout).
    pub volume_step: f32,
    pub hotkeys: Hotkeys,
    pub mouse: Mouse,
    pub discord: Discord,
    pub smart_pause: SmartPause,
    pub lyrics: Lyrics,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shape: Shape::Round,
            backdrop: BackdropPref::Auto,
            theme: Theme::System,
            monitor: MonitorPick::Primary,
            dock: DockMode::TaskbarCenter,
            taskbar_gap: 10,
            edge_margin: 16,
            free_x: 0,
            free_y: 0,
            snap_threshold: 50,
            auto_expand_on_track_change: true,
            auto_expand_ms: 2600,
            show_while_paused: true,
            volume_step: 0.02,
            hotkeys: Hotkeys::default(),
            mouse: Mouse::default(),
            discord: Discord::default(),
            smart_pause: SmartPause::default(),
            lyrics: Lyrics::default(),
        }
    }
}

const FILE_NAME: &str = "lumen.config.json";

/// Where the config actually ended up, for diagnostics in the tray tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Beside the executable — fully portable.
    Portable,
    /// `%APPDATA%\Lumen\` — the exe directory was not writable.
    Roaming,
    /// Neither location worked; settings live in memory only for this run.
    Ephemeral,
}

pub struct ConfigStore {
    inner: RwLock<Config>,
    path: Option<PathBuf>,
    origin: Origin,
}

impl ConfigStore {
    /// Resolve a location, load whatever is there, and repair anything unusable.
    pub fn load() -> Self {
        let (path, origin) = resolve_path();

        let cfg = path
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| match serde_json::from_str::<Config>(strip_bom(&raw)) {
                Ok(c) => Some(c),
                Err(e) => {
                    // A malformed file must never block startup. Keep the bad copy
                    // so the user can recover hand-edits, then start clean.
                    tracing::warn!("config parse failed ({e}); falling back to defaults");
                    if let Some(p) = path.as_deref() {
                        let _ = fs::rename(p, p.with_extension("json.bak"));
                    }
                    None
                }
            })
            .unwrap_or_default();

        let store = Self { inner: RwLock::new(cfg), path, origin };
        // Materialise defaults on first run so the file is discoverable and editable.
        let _ = store.save();
        store
    }

    pub fn get(&self) -> Config {
        self.inner.read().expect("config lock poisoned").clone()
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Replace the whole config and persist. Returns the stored value.
    pub fn set(&self, next: Config) -> Config {
        {
            let mut guard = self.inner.write().expect("config lock poisoned");
            *guard = next;
        }
        if let Err(e) = self.save() {
            tracing::warn!("config save failed: {e}");
        }
        self.get()
    }

    pub fn update(&self, f: impl FnOnce(&mut Config)) -> Config {
        {
            let mut guard = self.inner.write().expect("config lock poisoned");
            f(&mut guard);
        }
        if let Err(e) = self.save() {
            tracing::warn!("config save failed: {e}");
        }
        self.get()
    }

    fn save(&self) -> anyhow::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let json = serde_json::to_string_pretty(&self.get())?;
        // Write-then-rename so a crash mid-write cannot leave a truncated config.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Prefer the exe directory; prove it is writable rather than assuming.
fn resolve_path() -> (Option<PathBuf>, Origin) {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && is_writable(dir)
    {
        return (Some(dir.join(FILE_NAME)), Origin::Portable);
    }

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let dir = PathBuf::from(appdata).join("Lumen");
        if fs::create_dir_all(&dir).is_ok() && is_writable(&dir) {
            return (Some(dir.join(FILE_NAME)), Origin::Roaming);
        }
    }

    tracing::warn!("no writable config location; settings will not persist");
    (None, Origin::Ephemeral)
}

/// Drop a leading UTF-8 byte-order mark.
///
/// `serde_json` rejects a BOM as invalid input, and on Windows a BOM is the
/// *normal* result of editing a file by hand: Notepad writes one, and so does
/// PowerShell's `Set-Content -Encoding utf8`. Without this, hand-editing
/// `lumen.config.json` silently reverts every setting to its default — the file
/// gets renamed to `.bak` and the user is left wondering why nothing they typed
/// took effect. (Found exactly that way: a test harness wrote the config from
/// PowerShell and every configured run silently used defaults instead.)
fn strip_bom(raw: &str) -> &str {
    raw.strip_prefix('\u{feff}').unwrap_or(raw)
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".lumen-write-probe");
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_written_with_a_utf8_bom() {
        // Exactly what Notepad and `Set-Content -Encoding utf8` produce.
        let raw = "\u{feff}{\"showWhilePaused\":false,\"volumeStep\":0.05}";
        let cfg: Config =
            serde_json::from_str(strip_bom(raw)).expect("a BOM must not break the config");
        assert!(!cfg.show_while_paused);
        assert_eq!(cfg.volume_step, 0.05);
    }

    #[test]
    fn parses_config_without_a_bom() {
        let cfg: Config = serde_json::from_str(strip_bom("{\"taskbarGap\":42}")).unwrap();
        assert_eq!(cfg.taskbar_gap, 42);
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        // Forward compatibility: a config from a newer build must still load.
        let cfg: Config = serde_json::from_str(strip_bom("{\"somethingNew\":1}")).unwrap();
        assert_eq!(cfg.taskbar_gap, Config::default().taskbar_gap);
    }

    #[test]
    fn legacy_shape_values_still_load() {
        for legacy in ["\"pill\"", "\"native\""] {
            let json = format!("{{\"shape\":{legacy}}}");
            let cfg: Config = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("legacy shape {legacy} should load: {e}"));
            assert_eq!(cfg.shape, Shape::Round);
        }
    }
}
