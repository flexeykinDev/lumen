// Typed wrapper over the Tauri IPC surface. Nothing else in the renderer calls
// `invoke` or `listen` directly, so the host contract lives in exactly one file.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  EVT,
  type AppConfig,
  type AppVolumeState,
  type IslandState,
  type NowPlaying,
  type PlacementEvent,
  type SessionSummary,
  type TransitionEvent,
  type VolumeState,
} from "./types";

export type BackdropKind = "mica" | "acrylic" | "none";

export interface RuntimeInfo {
  backdrop: BackdropKind;
  configPath: string | null;
  portable: boolean;
  version: string;
}

export type TransportAction = "playPause" | "next" | "previous";

export const host = {
  runtimeInfo: () => invoke<RuntimeInfo>("runtime_info"),
  nowPlaying: () => invoke<NowPlaying | null>("now_playing"),
  /// The host drives visibility, and its first transition fires before this
  /// WebView can listen. Without an explicit read at startup the renderer stays
  /// stuck on its initial guess. See `Island.connect`.
  islandState: () => invoke<IslandState>("island_state"),
  /// Same late-join problem as `islandState`: the island is placed before this
  /// WebView is listening, so the announcing event has already been and gone.
  placement: () => invoke<PlacementEvent>("placement"),
  transport: (action: TransportAction) => invoke<void>("transport", { action }),
  /// Absolute position in seconds. Sources may refuse; the next timeline event
  /// is the authority on where playback actually landed.
  seek: (position: number) => invoke<void>("seek", { position }),
  setHover: (hovering: boolean) => invoke<void>("set_hover", { hovering }),
  getConfig: () => invoke<AppConfig>("get_config"),
  setConfig: (config: AppConfig) => invoke<AppConfig>("set_config", { config }),

  islandOrigin: () => invoke<[number, number]>("island_origin"),
  /// Suspends automatic placement for the duration of the gesture — otherwise a
  /// media event mid-drag re-docks the window under the pointer.
  dragStart: () => invoke<void>("drag_start"),
  /// Abandon a drag that will get no pointerup — the capsule was hidden, or
  /// capture was lost. Without this the host stays in drag mode forever.
  dragCancel: () => invoke<void>("drag_cancel"),
  /// Screen coordinates of the window's top-left. No animation — the capsule
  /// must sit exactly under the pointer while dragging.
  dragTo: (x: number, y: number) => invoke<void>("drag_to", { x, y }),
  /// Snaps to the nearest anchor within the threshold and persists the result.
  /// `vx`/`vy` are px/ms at release: they widen the catchment in the direction
  /// of a flick, and set the glide's opening speed so it continues the gesture
  /// instead of restarting it.
  dragEnd: (x: number, y: number, vx: number, vy: number) =>
    invoke<AppConfig>("drag_end", { x, y, vx, vy }),

  sessions: () => invoke<SessionSummary[]>("sessions"),
  /// Follow the next media source; no-op when only one is publishing.
  cycleSession: () => invoke<void>("cycle_session"),
  focusSession: (sessionId: string) => invoke<void>("focus_session", { sessionId }),

  volumeState: () => invoke<VolumeState>("volume_state"),
  /// Fire-and-forget. The host coalesces a wheel burst into one COM call and
  /// reports the result on EVT.volume, so we never await a round-trip per notch.
  volumeAdjust: (delta: number) => invoke<void>("volume_adjust", { delta }),
  /// Moves the volume of whatever the island is showing, falling back to the
  /// master when no application can be identified. This is what the island's own
  /// wheel uses — see `volume_adjust_media` for why it is not the master.
  volumeAdjustMedia: (delta: number) => invoke<void>("volume_adjust_media", { delta }),
  volumeToggleMuteMedia: () => invoke<void>("volume_toggle_mute_media"),
  volumeSet: (scalar: number) => invoke<void>("volume_set", { scalar }),
  volumeToggleMute: () => invoke<void>("volume_toggle_mute"),
  /// Hands a canvas-rendered PNG to the host, which writes it and copies it.
  shareCard: (png: number[]) => invoke<{ path: string; copied: boolean }>("share_card", { png }),
};

export const on = {
  nowPlaying: (fn: (v: NowPlaying | null) => void): Promise<UnlistenFn> =>
    listen<NowPlaying | null>(EVT.nowPlaying, (e) => fn(e.payload)),
  transition: (fn: (v: TransitionEvent) => void): Promise<UnlistenFn> =>
    listen<TransitionEvent>(EVT.transition, (e) => fn(e.payload)),
  config: (fn: (v: AppConfig) => void): Promise<UnlistenFn> =>
    listen<AppConfig>(EVT.config, (e) => fn(e.payload)),
  volume: (fn: (v: VolumeState) => void): Promise<UnlistenFn> =>
    listen<VolumeState>(EVT.volume, (e) => fn(e.payload)),
  /// Fires only for taskbar-button scrolling, which targets one application
  /// rather than the system master.
  appVolume: (fn: (v: AppVolumeState) => void): Promise<UnlistenFn> =>
    listen<AppVolumeState>(EVT.appVolume, (e) => fn(e.payload)),
  sessions: (fn: (v: SessionSummary[]) => void): Promise<UnlistenFn> =>
    listen<SessionSummary[]>(EVT.sessions, (e) => fn(e.payload)),
  placement: (fn: (v: PlacementEvent) => void): Promise<UnlistenFn> =>
    listen<PlacementEvent>(EVT.placement, (e) => fn(e.payload)),
  /// The tray asking for a share card. Only this side can draw one.
  shareRequest: (fn: () => void): Promise<UnlistenFn> =>
    listen(EVT.shareRequest, () => fn()),
};
