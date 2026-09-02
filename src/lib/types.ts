// Mirrors src-tauri/src/media/model.rs and src-tauri/src/color/mod.rs.
// Keep field names in sync — serde uses camelCase on the wire.

export type PlaybackState = "playing" | "paused" | "stopped" | "changing";

export interface Timeline {
  /** Seconds into the track at the moment `updatedAtMs` was captured. */
  positionSec: number;
  /** Total track length in seconds. 0 means unknown (live stream). */
  durationSec: number;
  /** Host monotonic clock (ms) when this timeline was sampled. */
  updatedAtMs: number;
}

export interface Accent {
  /** Vibrancy-corrected dominant colour, `#rrggbb`. */
  base: string;
  /** Readable foreground against `base`. */
  fg: string;
  /** Desaturated, darkened companion used for the ambient glow. */
  glow: string;
}

export interface NowPlaying {
  sessionId: string;
  /** AUMID-derived app name, e.g. "Spotify". */
  source: string;
  title: string;
  artist: string;
  album: string;
  state: PlaybackState;
  timeline: Timeline;
  /** `data:image/...;base64,...` or null when the session exposes no thumbnail. */
  artDataUri: string | null;
  accent: Accent | null;
  /** Bumped whenever the track identity changes; drives the crossfade key. */
  revision: number;
}

export type IslandState = "hidden" | "collapsed" | "expanded";

/** Emitted once at the start of each transition — the renderer runs the same curve. */
export interface TransitionEvent {
  state: IslandState;
  durationMs: number;
}

/**
 * Which way round the capsule's contents belong.
 *
 * The window grows and shrinks toward whichever edge it is pinned to. Docked on
 * the right that is the right edge, so a left-to-right layout makes the artwork
 * travel sideways on every collapse while the pinned edge stays put. Mirroring
 * puts the contents against the edge that does not move.
 */
export interface PlacementEvent {
  mirrored: boolean;
}

export type DockMode =
  | "taskbar-center"
  | "bottom-left"
  | "bottom-right"
  | "top-left"
  | "top-right"
  | "free";

/** One Discord presence button. `url` may carry `{title}`, `{artist}`, `{album}`. */
export interface PresenceButton {
  enabled: boolean;
  label: string;
  url: string;
}

export interface DiscordConfig {
  enabled: boolean;
  /**
   * `listening` renders a progress bar; `playing` renders buttons. Discord
   * draws buttons on a Playing activity only, so the two are exclusive.
   */
  activity: "listening" | "playing";
  applicationId: string;
  showWhilePaused: boolean;
  showArtist: boolean;
  showAlbum: boolean;
  showSource: boolean;
  showTimestamps: boolean;
  /** Looks the cover up online — the only part of presence that uses the network. */
  albumArt: boolean;
  /** Players never published, by the name shown on the capsule. */
  hiddenSources: string[];
  buttons: PresenceButton[];
}

/** How Clawd moves, what he wears, and whether he exists yet. */
export interface PetConfig {
  /** Set by the discovery gesture, never by a switch. */
  unlocked: boolean;
  enabled: boolean;
  /** Height in logical px, 12–32. */
  size: number;
  dance: "bob" | "sway" | "hop" | "spin";
  /** `#rrggbb`; every other tone is derived from it. */
  color: string;
  hat: "none" | "cap" | "crown" | "headphones" | "antenna";
}

export interface MouseConfig {
  enabled: boolean;
  taskbarWheelVolume: boolean;
  taskbarWheelPerApp: boolean;
  taskbarWheelOverFullscreen: boolean;
  middleClickHides: boolean;
  altMiddleQuits: boolean;
  taskbarCloseButton: "none" | "middle" | "right";
}

/** Mirrors `config::Config`. serde renames to camelCase on the wire. */
export interface AppConfig {
  /** `auto` follows the Windows UI language. */
  language: "auto" | "en" | "ru";
  /** Interface zoom on top of the monitor's DPI scale. 1 = as Windows says. */
  uiScale: number;
  /** Volume past 100% and bass lift, by processing the app's audio. */
  boost: { enabled: boolean; gain: number; bassDb: number };
  /** Clawd, the pixel pet. Hidden until the easter egg is found. */
  pet: PetConfig;
  /** False until the first-run tour has been seen, once, ever. */
  onboarded: boolean;
  shape: "round" | "square";
  backdrop: "mica" | "acrylic" | "auto";
  theme: "dark" | "light" | "system";
  monitor: "primary" | "cursor";
  dock: DockMode;
  /** Gap in px between the island's docked edge and the screen edge. */
  taskbarGap: number;
  /** Inset from the side for the corner docks, in logical px. */
  edgeMargin: number;
  freeX: number;
  freeY: number;
  /** How near an anchor a drop must land to snap. 0 disables snapping. */
  snapThreshold: number;
  autoExpandOnTrackChange: boolean;
  autoExpandMs: number;
  showWhilePaused: boolean;
  /** Scalar units moved per wheel notch; 0.02 = 2%. */
  volumeStep: number;
  hotkeys: {
    playPause: string;
    next: string;
    previous: string;
    cycleSession: string;
    volumeUp: string;
    volumeDown: string;
    repeat: string;
    toggleVisible: string;
    togglePinned: string;
  };
  mouse: MouseConfig;
  discord: DiscordConfig;
  smartPause: { enabled: boolean; resumeOnUnlock: boolean };
  lyrics: {
    enabled: boolean;
    geniusFallback: boolean;
    /** Shifts every line; positive shows them later. */
    offsetMs: number;
    /** An extra shift for guessed timings only. */
    estimatedOffsetMs: number;
  };
  /** The live spectrum. The only feature here that costs CPU while it runs. */
  spectrum: { enabled: boolean };
  /** Hold the panel open instead of collapsing back to the pill. */
  alwaysExpanded: boolean;
  startWithWindows: boolean;
}

export interface VolumeState {
  /** 0..1, matching the Windows volume slider (which is also scalar). */
  scalar: number;
  muted: boolean;
}

/**
 * One application's own volume, as shown in the Windows Volume Mixer.
 *
 * Separate from `VolumeState` because it answers a different question, and
 * because Windows draws no indicator of its own for this — the capsule is the
 * only feedback the gesture gets.
 */
export interface AppVolumeState {
  /** Display name, e.g. "Firefox". */
  app: string;
  scalar: number;
  muted: boolean;
}

/// One lyric line, at the moment it should appear.
export interface LyricLine {
  atSec: number;
  text: string;
}

/// Timed lyrics for one track, delivered once. The renderer picks the current
/// line from the clock it already interpolates, so playback costs no IPC.
export interface LyricsEvent {
  sessionId: string;
  revision: number;
  lines: LyricLine[];
  /// The source had words but no timings, which is worth distinguishing from
  /// having no lyrics at all.
  plainOnly: boolean;
  /// Timings were guessed by spreading plain lines across the track, not read
  /// from a .lrc. The UI shows these with less confidence because they drift.
  estimated: boolean;
}

export interface SessionSummary {
  /** AUMID — opaque, used only as an identity for focusing. */
  id: string;
  /** Human-readable name derived from the AUMID, e.g. "Spotify". */
  source: string;
  isCurrent: boolean;
}

export const EVT = {
  nowPlaying: "lumen://now-playing",
  transition: "lumen://transition",
  config: "lumen://config",
  volume: "lumen://volume",
  appVolume: "lumen://app-volume",
  sessions: "lumen://sessions",
  placement: "lumen://placement",
  shareRequest: "lumen://share-request",
  lyrics: "lumen://lyrics",
  spectrum: "lumen://spectrum",
} as const;
