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

/// Interface language.
///
/// Covers the settings window and the tray menu; there is no other text in the
/// product. `Auto` reads the system language, which is right for almost
/// everyone — the explicit values are for a machine whose Windows is in one
/// language and whose owner is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Auto,
    En,
    Ru,
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
    /// Volume up and down for the playing application — the keyboard version of
    /// the taskbar wheel, for keyboards without media keys or where Windows'
    /// own volume keys move the master instead of the app.
    pub volume_up: String,
    pub volume_down: String,
    /// Cycle the player's repeat mode: off, whole list, one track.
    pub repeat: String,
    /// Put the capsule away, or bring it back.
    pub toggle_visible: String,
    /// Hold the full panel open instead of letting it collapse.
    pub toggle_pinned: String,
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
            // Unbound by default. These are additions to a keyboard that
            // already has media keys, and every default is a key taken away
            // from something else the user may already have bound.
            volume_up: String::new(),
            volume_down: String::new(),
            repeat: String::new(),
            toggle_visible: String::new(),
            toggle_pinned: String::new(),
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
/// Presence is published *as* a Discord application, so it needs an id. The
/// default is Lumen's own: the application is called Lumen and its artwork is
/// Lumen's, so "Listening to Lumen" is accurate for everyone rather than
/// borrowed branding — which is the only reason this used to be blank.
///
/// Anyone who wants their profile to say something else creates their own
/// application and pastes its id over this one.
/// Which activity type the presence is published as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    /// "Listening to …", with the progress bar. No buttons.
    Listening,
    /// "Playing …", which is the only type whose buttons Discord draws.
    Playing,
}

impl ActivityKind {
    /// The number Discord's RPC expects.
    pub fn code(self) -> u8 {
        match self {
            Self::Playing => 0,
            Self::Listening => 2,
        }
    }

    /// Whether Discord will render buttons for this type.
    pub fn shows_buttons(self) -> bool {
        matches!(self, Self::Playing)
    }
}

/// Lumen's own Discord application. Not a secret: a client id is public by
/// design — it is what every Rich Presence integration puts on the wire, and it
/// grants nothing on its own.
pub const DEFAULT_APPLICATION_ID: &str = "1544611850946224128";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Discord {
    pub enabled: bool,
    /// How the presence is announced, and — the part that matters in practice —
    /// whether the buttons render.
    ///
    /// Discord draws presence buttons for a *Playing* activity. On a Listening
    /// activity the client shows the progress bar and the album art but drops
    /// the buttons, so the two cannot both be had at once. Listening reads
    /// better for music and is the default; anyone who wants the buttons can
    /// take the trade.
    pub activity: ActivityKind,
    /// Client id from <https://discord.com/developers/applications>. The
    /// application's *name* is what renders as "Listening to …".
    pub application_id: String,
    /// Keep the presence up while paused, rather than clearing it.
    pub show_while_paused: bool,
    /// The second line: who is playing. Off publishes the title alone.
    pub show_artist: bool,
    /// Album name as the hover text on the large image.
    pub show_album: bool,
    /// Name the player ("Listening via Lumen · Spotify") rather than Lumen alone.
    pub show_source: bool,
    /// The elapsed/remaining clock. Discord animates this itself, so it keeps
    /// running even between our updates.
    pub show_timestamps: bool,
    /// Look the cover up online and show it as the presence image.
    ///
    /// Off by default because it is a network call, like [`Lyrics`]: Discord
    /// renders an image by URL or by asset key, and SMTC hands the cover over as
    /// *bytes*. Turning this on sends the artist and title to Apple's public
    /// iTunes Search endpoint to find a URL for the same artwork.
    pub album_art: bool,
    /// Sources that are never published — by the name shown on the capsule,
    /// e.g. `"Firefox"`. A blocklist rather than an allowlist so a newly
    /// installed player does not silently go missing from your profile.
    pub hidden_sources: Vec<String>,
    /// Up to two link buttons under the presence.
    ///
    /// Note Discord does not draw these on your *own* profile — only other
    /// people see them, which makes them look broken if you don't know.
    pub buttons: Vec<PresenceButton>,
}

/// One presence button.
///
/// `url` may carry `{title}`, `{artist}` and `{album}`, each percent-encoded on
/// substitution — that is what turns a fixed link into "find *this* track".
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresenceButton {
    pub enabled: bool,
    /// Discord's limit is 32 characters; longer labels are truncated.
    pub label: String,
    pub url: String,
}

impl Discord {
    /// Whether presence should be published for a given player.
    pub fn publishes(&self, source: &str) -> bool {
        !self.hidden_sources.iter().any(|hidden| hidden.eq_ignore_ascii_case(source.trim()))
    }
}

impl Default for Discord {
    fn default() -> Self {
        Self {
            enabled: true,
            activity: ActivityKind::Listening,
            application_id: DEFAULT_APPLICATION_ID.into(),
            show_while_paused: false,
            show_artist: true,
            show_album: true,
            show_source: true,
            show_timestamps: true,
            album_art: false,
            hidden_sources: Vec::new(),
            buttons: vec![
                PresenceButton {
                    enabled: true,
                    label: "Find this track".into(),
                    url: "https://www.youtube.com/results?search_query={artist}+{title}".into(),
                },
                PresenceButton {
                    enabled: false,
                    label: "Get Lumen".into(),
                    url: "https://github.com/".into(),
                },
            ],
        }
    }
}

/// How Clawd moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Dance {
    /// A different one every track. The default, because a pet that does the
    /// same two poses forever stops being interesting by the third song.
    #[default]
    Random,
    /// Bobs on the spot, claws counter-swinging.
    Bob,
    /// Leans side to side, like something keeping time.
    Sway,
    /// Small hops with a squash on the landing.
    Hop,
    /// Turns on the spot, in four steps.
    Spin,
}

/// What Clawd is wearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Hat {
    #[default]
    None,
    Cap,
    Crown,
    Headphones,
    Antenna,
}

/// Checking whether a newer build has been published.
///
/// On by default, and it is a single request: the version string is a plain
/// text file in the repository, not an API call, so there is no token, no rate
/// limit and nothing to identify the machine beyond the fact that a copy of
/// Lumen exists. It runs once per launch and never downloads anything —
/// finding the release page is left to the person who wants it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Updates {
    pub check: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self { check: true }
    }
}

/// Clawd, the pixel pet.
///
/// An easter egg in the proper sense: there is no switch for it until it has
/// been found. `unlocked` is what the discovery sets, and until then the pet
/// section of the settings window does not exist — a feature you have to be
/// told about in a settings list was never a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pet {
    /// Found by clicking the album art repeatedly. Never set by the UI.
    pub unlocked: bool,
    /// Whether he is on screen. Turned on by the discovery, and a plain switch
    /// afterwards.
    pub enabled: bool,
    /// Height in logical pixels. Bounded by what the capsule row can hold.
    pub size: u32,
    pub dance: Dance,
    /// Shell colour as `#rrggbb`, or `auto` to follow the album art.
    ///
    /// `auto` is the default: the capsule already tints itself from the cover,
    /// and a character who ignores that is the one thing on screen that does
    /// not belong to the track.
    pub color: String,
    pub hat: Hat,
}

impl Default for Pet {
    fn default() -> Self {
        Self {
            unlocked: false,
            enabled: false,
            size: 20,
            dance: Dance::Random,
            color: "auto".into(),
            hat: Hat::None,
        }
    }
}

impl Pet {
    /// Size clamped to what the collapsed row can actually fit.
    pub fn clamped_size(&self) -> u32 {
        self.size.clamp(12, 32)
    }
}

/// Volume boost past 100%, and bass boost.
///
/// **Off by default.** Unlike every other switch here, this one changes how
/// your audio reaches the speakers: Lumen captures the playing application,
/// mutes it, and renders a processed copy. That is the only way past Windows'
/// 100% ceiling from user space — see `audio::boost` — and it has real costs,
/// about 30 ms of added latency and CPU for as long as it runs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Boost {
    pub enabled: bool,
    /// Linear gain. 1.0 is untouched; 3.0 is the practical limit before a
    /// limiter is doing more work than the music.
    pub gain: f32,
    /// Low-shelf lift below 120 Hz, in decibels.
    pub bass_db: f32,
}

impl Default for Boost {
    fn default() -> Self {
        Self { enabled: false, gain: 1.0, bass_db: 0.0 }
    }
}

impl Boost {
    /// The settings the engine takes, clamped to what it can do sensibly.
    ///
    /// Clamped here rather than trusted from the file: this config is editable
    /// by hand, and a gain of 50 would be a wall of limiter.
    pub fn settings(&self) -> crate::audio::boost::Settings {
        crate::audio::boost::Settings {
            gain: self.gain.clamp(0.5, 3.0),
            bass_db: self.bass_db.clamp(-12.0, 12.0),
        }
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
    /// Shift *every* line by this many milliseconds, estimated or not.
    ///
    /// Separate from `estimated_offset_ms` because it corrects a different
    /// thing: this is for a lyric file that is simply early or late against
    /// this particular recording — a remaster, a different release, a source
    /// whose timings were made against another master. Applied when the line is
    /// chosen rather than when it is fetched, so the slider moves the words
    /// while the song is playing. Negative shows them earlier.
    pub offset_ms: i64,
    /// Shift every estimated line by this many milliseconds.
    ///
    /// Only affects guessed timings, never a real .lrc. Estimated lines drift
    /// by their nature — the line count from a lyrics page rarely matches what
    /// is actually sung — so a nudge is the only honest correction available.
    /// Negative shows them earlier.
    pub estimated_offset_ms: i64,
}

impl Default for Lyrics {
    fn default() -> Self {
        Self { enabled: false, genius_fallback: true, offset_ms: 0, estimated_offset_ms: 0 }
    }
}

/// The live spectrum behind the expanded panel.
///
/// Unlike everything else here, this one costs CPU while it runs: it captures
/// audio and transforms it twenty times a second. It is therefore gated to
/// expanded *and* playing, and this switch turns it off outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpectrumCfg {
    pub enabled: bool,
}

impl Default for SpectrumCfg {
    fn default() -> Self {
        Self { enabled: true }
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
    pub language: Language,
    /// Interface zoom, on top of the monitor's own DPI scale.
    ///
    /// Windows' scaling already keeps the capsule the same physical size across
    /// displays. This is for the case that leaves — a 2K or 4K panel running at
    /// 100%, where every correctly-sized interface is also a tiny one.
    pub ui_scale: f32,
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
    pub spectrum: SpectrumCfg,
    pub boost: Boost,
    pub pet: Pet,
    pub updates: Updates,
    /// Whether the first-run tour has been shown.
    ///
    /// Lives in the config rather than in the registry so that a portable copy
    /// carries its own answer: the same exe on a USB stick introduces itself
    /// once per machine it is set up on, not once per launch.
    pub onboarded: bool,
    /// Hold the panel open rather than collapsing it back to the pill.
    ///
    /// The capsule still hides when there is nothing playing: this decides the
    /// state it settles into while it is up, not whether it is up.
    pub always_expanded: bool,
    /// Launch when Windows starts.
    ///
    /// Mirrored into HKCU\...\CurrentVersion\Run and reconciled at every
    /// launch, because a portable exe moves and a stale entry silently stops
    /// working. The registry is the source of truth; this is the intent.
    pub start_with_windows: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: Language::Auto,
            ui_scale: 1.0,
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
            spectrum: SpectrumCfg::default(),
            boost: Boost::default(),
            pet: Pet::default(),
            updates: Updates::default(),
            always_expanded: false,
            onboarded: false,
            start_with_windows: false,
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
    fn a_config_from_an_older_build_gains_every_new_field() {
        // Forward compatibility in the direction that actually happens: someone
        // updates the exe and their existing file is missing everything added
        // since. Every one of these must come back as its default rather than
        // failing the parse and silently resetting the whole file.
        let old = r#"{"shape":"round","taskbarGap":12,"showWhilePaused":false}"#;
        let cfg: Config = serde_json::from_str(old).expect("an old config must still load");

        assert_eq!(cfg.taskbar_gap, 12, "existing values must survive");
        assert!(!cfg.show_while_paused);
        assert_eq!(cfg.language, Language::Auto);
        assert_eq!(cfg.ui_scale, 1.0);
        assert!(!cfg.boost.enabled);
        assert!(!cfg.pet.enabled);
        assert!(!cfg.onboarded);
        assert_eq!(cfg.discord.activity, ActivityKind::Listening);
        assert_eq!(cfg.lyrics.offset_ms, 0);
        assert!(cfg.hotkeys.repeat.is_empty());
    }

    #[test]
    fn presence_works_out_of_the_box() {
        // The default is Lumen's own application, so a fresh install shows
        // "Listening to Lumen" without anyone visiting the developer portal.
        let discord = Discord::default();
        assert!(discord.enabled);
        assert_eq!(discord.application_id, DEFAULT_APPLICATION_ID);
        assert!(
            discord.application_id.chars().all(|c| c.is_ascii_digit()),
            "a Discord client id is a bare snowflake; anything else is rejected at the handshake"
        );
    }

    #[test]
    fn the_pet_starts_locked_and_silent() {
        // The whole point of the easter egg: a fresh install must not mention
        // him, show him, or leave a switch lying around that does.
        let pet = Pet::default();
        assert!(!pet.unlocked);
        assert!(!pet.enabled);
        // A different dance per track is the default: two poses forever is
        // what made the first version boring by the third song.
        assert_eq!(pet.dance, Dance::Random);
        assert_eq!(pet.color, "auto", "he should take the album's colour unless told otherwise");
        assert_eq!(pet.hat, Hat::None);
    }

    #[test]
    fn a_hand_edited_pet_size_is_clamped_to_the_row() {
        // The config is editable by hand, and a 400px crab in a 44px capsule is
        // a layout bug rather than a preference.
        let huge = Pet { size: 400, ..Pet::default() };
        assert_eq!(huge.clamped_size(), 32);
        let tiny = Pet { size: 0, ..Pet::default() };
        assert_eq!(tiny.clamped_size(), 12);
        assert_eq!(Pet::default().clamped_size(), 20);
    }

    #[test]
    fn hand_written_extremes_are_clamped_rather_than_obeyed() {
        // This file is editable by hand, so every number in it is untrusted.
        let wild = r#"{"boost":{"enabled":true,"gain":500.0,"bassDb":-99.0}}"#;
        let cfg: Config = serde_json::from_str(wild).unwrap();
        let settings = cfg.boost.settings();
        assert_eq!(settings.gain, 3.0, "a gain of 500 is a wall of limiter");
        assert_eq!(settings.bass_db, -12.0);
    }

    #[test]
    fn a_config_survives_a_round_trip_through_the_file_format() {
        // Serialising and re-reading must produce the same settings; a field
        // that serialises under one name and deserialises under another looks
        // exactly like a setting that will not save.
        let mut cfg = Config::default();
        cfg.pet.enabled = true;
        cfg.boost.gain = 2.25;
        cfg.lyrics.offset_ms = -750;
        cfg.hotkeys.toggle_pinned = "Ctrl+F8".into();
        cfg.always_expanded = true;
        cfg.ui_scale = 1.35;

        let json = serde_json::to_string(&cfg).expect("serialise");
        let back: Config = serde_json::from_str(&json).expect("deserialise");

        assert!(back.pet.enabled);
        assert_eq!(back.boost.gain, 2.25);
        assert_eq!(back.lyrics.offset_ms, -750);
        assert_eq!(back.hotkeys.toggle_pinned, "Ctrl+F8");
        assert!(back.always_expanded);
        assert_eq!(back.ui_scale, 1.35);
    }

    #[test]
    fn the_wire_format_is_camel_case_as_the_renderer_expects() {
        // The TypeScript side reads these names literally. A rename here with no
        // matching change there is a setting that silently stops working.
        let json = serde_json::to_string(&Config::default()).unwrap();
        for key in [
            "\"uiScale\"",
            "\"alwaysExpanded\"",
            "\"startWithWindows\"",
            "\"bassDb\"",
            "\"offsetMs\"",
            "\"estimatedOffsetMs\"",
            "\"togglePinned\"",
            "\"toggleVisible\"",
            "\"volumeUp\"",
            "\"hiddenSources\"",
        ] {
            assert!(json.contains(key), "{key} missing from {json}");
        }
    }

    #[test]
    fn an_activity_that_draws_buttons_is_the_playing_one() {
        assert!(ActivityKind::Playing.shows_buttons());
        assert!(!ActivityKind::Listening.shows_buttons());
        assert_eq!(ActivityKind::Playing.code(), 0);
        assert_eq!(ActivityKind::Listening.code(), 2);
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
