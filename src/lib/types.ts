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

export interface AppConfig {
  shape: "pill" | "native";
  backdrop: "mica" | "acrylic" | "auto";
  theme: "dark" | "light" | "system";
  monitor: "primary" | "cursor";
  /** Gap in px between the island's bottom edge and the taskbar's top edge. */
  taskbarGap: number;
  autoExpandOnTrackChange: boolean;
  autoExpandMs: number;
  showWhilePaused: boolean;
  /** Scalar units moved per wheel notch; 0.02 = 2%. */
  volumeStep: number;
  hotkeys: { playPause: string; next: string; previous: string };
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
} as const;
