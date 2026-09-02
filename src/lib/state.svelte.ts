// The renderer's entire state. Svelte 5 runes — no store subscriptions, no
// virtual DOM, and no derived value recomputed outside of what actually reads it.

import { host, on, type RuntimeInfo } from "./bridge";
import { shareCard } from "./sharecard";
import { DURATION } from "./motion";
import type {
  Accent,
  AppConfig,
  IslandState,
  NowPlaying,
  SessionSummary,
  VolumeState,
} from "./types";

const FALLBACK_ACCENT: Accent = { base: "#7a8cff", fg: "#0b0d14", glow: "#2a3052" };

class Island {
  now = $state<NowPlaying | null>(null);
  state = $state<IslandState>("hidden");
  info = $state<RuntimeInfo | null>(null);
  config = $state<AppConfig | null>(null);
  volume = $state<VolumeState>({ scalar: 0, muted: false });
  sessions = $state<SessionSummary[]>([]);

  /**
   * Lay the capsule out right-to-left, because it is docked against the right
   * edge of the screen. The window shrinks toward the edge it is pinned to, so
   * the contents have to sit against that same edge — otherwise every collapse
   * drags the artwork sideways across the screen. Set by the host, which is the
   * only side that knows where the window actually is.
   */
  mirrored = $state(false);

  /** Only worth offering a switcher when there is somewhere to switch to. */
  canSwitch = $derived(this.sessions.length > 1);

  /**
   * `performance.now()` of the last volume change, so the capsule can surface a
   * readout briefly and then get out of the way. Scrolling the wheel is a
   * deliberate act — it deserves feedback — but the volume is not what the
   * island is *for*, so the readout is transient rather than permanent.
   */
  volumeTouchedAt = $state(0);

  /**
   * What the transient readout should show.
   *
   * Two different levels can drive it — the system master, and one
   * application's own — and they must never be confused for each other. A
   * per-app change leaves the master exactly where it was, so reading
   * `volume` there would show a number that did not move. `label` names the
   * application, or is null when the master moved.
   */
  volumeHud = $state<{ label: string | null; scalar: number; muted: boolean } | null>(null);

  /** Duration the host is using for the transition currently in flight. */
  transitionMs = $state<number>(DURATION.expand);

  /**
   * `performance.now()` when the current timeline sample arrived. The host sends
   * a position, never a tick — everything between samples is interpolated here,
   * so playback progress costs zero IPC.
   */
  #sampledAt = $state(0);

  expanded = $derived(this.state === "expanded");
  visible = $derived(this.state !== "hidden");
  playing = $derived(this.now?.state === "playing");
  accent = $derived(this.now?.accent ?? FALLBACK_ACCENT);

  /**
   * Where a seek asked playback to go, and when it was asked.
   *
   * Browsers republish a timeline of `position ≈ 0` for a while after a seek —
   * the old sample is gone and the new one has not settled — so trusting what
   * arrives makes the counter drop to 0:00 and start climbing while the video
   * plays on from the new position. Pausing forced a fresh, correct sample,
   * which is why pause/play appeared to "fix" it.
   *
   * So the requested position is held as the local truth and only given up when
   * a sample arrives that agrees with it.
   */
  // Plain fields, not `$state`: nothing renders these directly, and the only
  // reader is `positionAt`, which is already re-evaluated on every animation
  // frame from the `clock` rune.
  #seekTo: number | null = null;
  #seekAt = 0;

  /** How long to prefer the requested position over what the source reports. */
  static #SEEK_SETTLE_MS = 6000;
  /** How far a sample may sit from the expected position and still be believed. */
  static #SEEK_TOLERANCE_S = 3;

  /** Record a seek the moment it is requested, before anything is republished. */
  noteSeek(position: number): void {
    this.#seekTo = position;
    this.#seekAt = performance.now();
    this.#sampledAt = this.#seekAt;
  }

  /** Seconds into the track right now, extrapolated from the last sample. */
  positionAt(clock: number): number {
    const t = this.now?.timeline;
    if (!t) return 0;

    // While a seek is settling, extrapolate from where it was *sent*, not from
    // whatever the source is currently claiming.
    const base = this.#seekPending(clock) ? (this.#seekTo as number) : t.positionSec;
    const drift = this.playing ? Math.max(0, (clock - this.#sampledAt) / 1000) : 0;
    const raw = base + drift;
    return t.durationSec > 0 ? Math.min(raw, t.durationSec) : raw;
  }

  #seekPending(clock: number): boolean {
    return this.#seekTo !== null && clock - this.#seekAt < Island.#SEEK_SETTLE_MS;
  }

  async connect(): Promise<() => void> {
    const unlisten = await Promise.all([
      on.nowPlaying((v) => this.#applyNowPlaying(v)),
      on.transition((v) => {
        this.transitionMs = v.durationMs;
        this.state = v.state;
      }),
      on.config((v) => {
        this.config = v;
      }),
      on.sessions((v) => {
        this.sessions = v;
      }),
      on.placement((v) => {
        this.mirrored = v.mirrored;
      }),
      on.shareRequest(() => {
        // Nothing to share with nothing playing, and a card of "Nothing
        // playing" is worse than no card.
        if (!this.now) return;
        void shareCard(this.now, this.accent.base).catch((e) =>
          console.error("share card failed", e),
        );
      }),
      on.volume((v) => {
        // Only flag a "touch" when the level actually moved. The host also
        // publishes once at startup, and that must not pop the readout open.
        if (v.scalar !== this.volume.scalar || v.muted !== this.volume.muted) {
          this.volumeTouchedAt = performance.now();
          this.volumeHud = { label: null, scalar: v.scalar, muted: v.muted };
        }
        this.volume = v;
      }),
      on.appVolume((v) => {
        // Always a touch: a per-app change is only ever the result of a
        // deliberate gesture, and the level it reports belongs to a different
        // application each time, so there is nothing to compare against.
        this.volumeTouchedAt = performance.now();
        this.volumeHud = { label: v.app, scalar: v.scalar, muted: v.muted };
      }),
    ]);

    // Late join. Two separate races close here, and both are the normal case
    // rather than the exotic one:
    //
    //  - Media: a WebView reload must not leave an empty capsule waiting for the
    //    next SMTC event, which might be minutes away.
    //  - Island state: the host reveals the island as soon as it sees a session,
    //    which routinely happens *before* this WebView is listening. That
    //    transition event is simply lost. Reading the state explicitly is the
    //    only thing that keeps the renderer's layout in step with the window
    //    region the host is actually drawing — and a stale "hidden" here means a
    //    268 px capsule laid out as if it were 84 px.
    //
    // Listeners are subscribed above *before* these reads, so a transition that
    // lands mid-flight is applied after, not dropped.
    const [snapshot, info, config, state, volume, sessions, placement] = await Promise.all([
      host.nowPlaying(),
      host.runtimeInfo(),
      host.getConfig(),
      host.islandState(),
      host.volumeState(),
      host.sessions(),
      host.placement(),
    ]);
    this.info = info;
    this.config = config;
    this.state = state;
    this.volume = volume;
    this.sessions = sessions;
    this.mirrored = placement.mirrored;
    this.#applyNowPlaying(snapshot);

    return () => unlisten.forEach((fn) => fn());
  }

  #applyNowPlaying(v: NowPlaying | null): void {
    const now = performance.now();
    const trackChanged = v?.revision !== this.now?.revision || v?.sessionId !== this.now?.sessionId;

    if (this.#seekTo !== null) {
      if (trackChanged || v === null) {
        // A different track entirely: the seek no longer means anything.
        this.#seekTo = null;
      } else if (this.#seekPending(now)) {
        // Reconcile. Where should playback be by now if the seek landed?
        const expected =
          this.#seekTo + (this.playing ? Math.max(0, (now - this.#seekAt) / 1000) : 0);
        const reported = v?.timeline.positionSec ?? 0;
        // A literal zero is never a plausible answer to "where did that seek
        // land". Browsers publish it for a beat after seeking, and accepting it
        // is what drops the counter to 0:00.
        const reportedZero = reported < 1 && expected > 2;

        if (!reportedZero && Math.abs(reported - expected) <= Island.#SEEK_TOLERANCE_S) {
          // The source agrees; hand authority back to it, and keep the clock
          // running from this sample so nothing jumps.
          this.#seekTo = null;
        } else {
          // Still stale (the 0:00 case). Keep extrapolating from the request,
          // and do not restart the drift clock — that is what would make the
          // counter stutter.
          this.now = v;
          return;
        }
      } else {
        this.#seekTo = null;
      }
    }

    this.now = v;
    this.#sampledAt = now;
  }
}

export const island = new Island();

export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${m}:${String(s).padStart(2, "0")}`;
}
