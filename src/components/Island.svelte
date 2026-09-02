<script lang="ts">
  import AlbumArt from "./AlbumArt.svelte";
  import Controls from "./Controls.svelte";
  import Marquee from "./Marquee.svelte";
  import Progress from "./Progress.svelte";
  import Volume from "./Volume.svelte";
  import { host } from "../lib/bridge";
  import { EASE_CSS } from "../lib/motion";
  import { island } from "../lib/state.svelte";

  // The host animates the *window rectangle* to the capsule size, so the stage
  // simply fills whatever the window currently is. It must not carry its own
  // width/height animation: that would run a second, competing interpolation
  // against the one the host is already applying to the window.
  //
  // Sizes still live in src-tauri/src/window/island.rs; nothing here needs to
  // know them.
  const now = $derived(island.now);
  const accent = $derived(island.accent);
  const title = $derived(now?.title?.trim() || "Nothing playing");
  const artist = $derived(now?.artist?.trim() || "");

  // Lyric follow.
  //
  // A timer rather than requestAnimationFrame: lines change every few seconds,
  // so 60 fps would be three hundred wasted wakeups per line, and there is
  // already a rAF loop driving the progress bar. 200 ms is well inside what
  // reads as "in time" and costs almost nothing.
  //
  // Gated on all three of expanded, playing and having lyrics, so a collapsed or
  // paused capsule runs no timer at all — the same rule the marquee follows.
  const LYRIC_TICK_MS = 200;
  let lyricClock = $state(performance.now());
  $effect(() => {
    if (!island.expanded || !island.playing || island.lyrics.length === 0) return;
    const id = setInterval(() => (lyricClock = performance.now()), LYRIC_TICK_MS);
    return () => clearInterval(id);
  });
  const lyric = $derived(island.expanded ? island.lyricAt(lyricClock) : null);

  // One element that scales between the two layouts rather than two elements
  // crossfading, so the cover physically travels between states.
  //
  // Mirrored, the slot is pinned to the right edge and its origin flips with it,
  // so the same offsets simply run the other way.
  const ART_BASE = 96;
  const artTransform = $derived.by(() => {
    const dir = island.mirrored ? -1 : 1;
    return island.expanded
      ? `translate(${16 * dir}px, -26px) scale(1)`
      : `translate(${9 * dir}px, -6px) scale(${32 / ART_BASE})`;
  });

  function hover(hovering: boolean) {
    // The host owns visibility; the renderer only reports what the pointer did.
    host.setHover(hovering).catch(() => {});
  }

  /**
   * Wheel over the capsule adjusts system volume.
   *
   * Worth noting what this deliberately is *not*: a WH_MOUSE_LL hook. The
   * WebView receives wheel events over its own window natively, so the island
   * needs no system-wide hook, swallows no input, and cannot be silently
   * unhooked by Windows for exceeding the low-level hook timeout. Only the
   * taskbar-scroll feature needs the real hook.
   */
  function onWheel(event: WheelEvent) {
    event.preventDefault();
    const step = island.config?.volumeStep ?? 0.02;

    // Modifiers change the grain, matching how the rest of Windows behaves:
    //   Ctrl  — coarse, for crossing the range in a few notches
    //   Shift — fine, for landing on an exact level
    // 5x a 2% step is 10%, so Ctrl gives ten notches end to end.
    const grain = event.ctrlKey ? 5 : event.shiftKey ? 0.25 : 1;

    // deltaY is positive scrolling *down*, which should mean quieter.
    const direction = event.deltaY > 0 ? -1 : 1;
    // The app the island is showing, not the system master. Scrolling here to
    // quieten Spotify used to move the endpoint volume instead, which is
    // downstream of anything Discord captures — so the stream stayed loud.
    host.volumeAdjustMedia(step * grain * direction).catch(() => {});
  }

  // A transient readout: scrolling deserves feedback, but the volume is not what
  // the island is for, so it yields back to the track after a beat. Re-runs on
  // every change, and the cleanup cancels the previous timer, so a fast spin
  // holds the readout open instead of flickering it.
  let showVolume = $state(false);
  $effect(() => {
    if (!island.volumeTouchedAt) return;
    showVolume = true;
    const id = setTimeout(() => (showVolume = false), 1400);
    return () => clearTimeout(id);
  });

  // The readout follows whichever level last moved — the system master, or one
  // application's own. Falling back to the master keeps the very first frame
  // sane, before anything has been touched.
  const hud = $derived(island.volumeHud ?? { label: null, ...island.volume });
  const volumePct = $derived(Math.round(hud.scalar * 100));
  const volumeMuted = $derived(hud.muted || hud.scalar <= 0.0001);

  // Three different volumes can move, and telling them apart matters: lowering
  // the system master does nothing for a stream, while lowering an app's own
  // volume does. So the readout always names its target rather than only doing
  // so for the per-app case.
  const systemLabel = navigator.language?.toLowerCase().startsWith("ru")
    ? "Система"
    : "System";
  const hudLabel = $derived(hud.label ?? systemLabel);

  // --- drag to dock ---------------------------------------------------------
  //
  // Screen coordinates throughout. `clientX/Y` are relative to the window, and
  // the window is moving under the pointer during a drag, so a delta computed
  // from them would feed back into itself and the capsule would run away.
  // `screenX/Y` are absolute and immune to that.
  //
  // Same shape as the seek scrub: a plain variable written per event, one state
  // write per frame from rAF, and no CSS transition in the way.

  let dragging = $state(false);
  let dragPointer = { x: 0, y: 0 };
  let dragOrigin = { x: 0, y: 0 };
  let dragStart = { x: 0, y: 0 };
  let dragRaf = 0;
  let dragMoved = false;

  /** Movement below this is a click, not a drag. */
  const DRAG_SLOP = 4;

  // Velocity is measured over a short trailing window rather than the last two
  // events. A single pair is dominated by whatever jitter happened in the final
  // millisecond — often a near-zero delta, which reads as "stopped" even when
  // the hand was clearly still moving. Averaging over ~80 ms of samples gives
  // the speed of the gesture instead of the speed of its last twitch.
  const VELOCITY_WINDOW_MS = 80;
  const VELOCITY_SAMPLES = 5;
  let samples: { x: number; y: number; t: number }[] = [];

  function recordSample(x: number, y: number) {
    samples.push({ x, y, t: performance.now() });
    if (samples.length > VELOCITY_SAMPLES) samples.shift();
  }

  /** Release velocity in px/ms, from the trailing sample window. */
  function releaseVelocity(): { vx: number; vy: number } {
    const now = performance.now();
    const recent = samples.filter((s) => now - s.t <= VELOCITY_WINDOW_MS);
    if (recent.length < 2) return { vx: 0, vy: 0 };

    const first = recent.at(0);
    const last = recent.at(-1);
    if (!first || !last) return { vx: 0, vy: 0 };

    const dt = last.t - first.t;
    // A zero span would divide by zero; a stale one is not a release gesture.
    if (dt <= 0) return { vx: 0, vy: 0 };

    return { vx: (last.x - first.x) / dt, vy: (last.y - first.y) / dt };
  }

  async function onDragPointerDown(event: PointerEvent) {
    // Left button only, and never when the press started on a control.
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button, .hit, .volume")) return;

    try {
      // Claim placement first, then read the origin: reading it before the
      // host stops auto-placing risks capturing a position that is already
      // being animated somewhere else.
      await host.dragStart();
      const [ox, oy] = await host.islandOrigin();
      dragOrigin = { x: ox, y: oy };
    } catch {
      return;
    }

    dragStart = { x: event.screenX, y: event.screenY };
    dragPointer = { x: event.screenX, y: event.screenY };
    samples = [];
    recordSample(event.screenX, event.screenY);
    dragMoved = false;
    dragging = true;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function pumpDrag() {
    if (!dragging) {
      dragRaf = 0;
      return;
    }
    const x = dragOrigin.x + (dragPointer.x - dragStart.x);
    const y = dragOrigin.y + (dragPointer.y - dragStart.y);
    host.dragTo(x, y).catch(() => {});
    dragRaf = requestAnimationFrame(pumpDrag);
  }

  function onDragPointerMove(event: PointerEvent) {
    if (!dragging) return;
    dragPointer = { x: event.screenX, y: event.screenY };
    recordSample(event.screenX, event.screenY);

    if (!dragMoved) {
      const dx = Math.abs(dragPointer.x - dragStart.x);
      const dy = Math.abs(dragPointer.y - dragStart.y);
      if (dx < DRAG_SLOP && dy < DRAG_SLOP) return;
      dragMoved = true;
    }
    if (!dragRaf) dragRaf = requestAnimationFrame(pumpDrag);
  }

  function onDragPointerUp(event: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    if (dragRaf) cancelAnimationFrame(dragRaf);
    dragRaf = 0;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);

    // A press that never moved is a click, so the placement is left alone — but
    // the host still has to be told the gesture is over. `dragStart` was already
    // sent on pointerdown, and returning here without a matching end left the
    // host in drag mode permanently: automatic placement disabled, and every
    // state change deferred, so the capsule froze at whatever size it had. One
    // stray click on the capsule was enough to do it.
    if (!dragMoved) {
      host.dragCancel().catch(() => {});
      return;
    }

    const x = dragOrigin.x + (dragPointer.x - dragStart.x);
    const y = dragOrigin.y + (dragPointer.y - dragStart.y);
    const { vx, vy } = releaseVelocity();
    host
      .dragEnd(x, y, vx, vy)
      .then((cfg) => (island.config = cfg))
      .catch(() => {});
  }

  /**
   * End a drag that will never receive a pointerup.
   *
   * Two ways that happens: the capsule is hidden out from under the pointer
   * (middle-click, or playback stopping), or capture is taken by another
   * window. Either leaves the host's drag flag set, which disables automatic
   * placement permanently — the island then sits wherever it was and stops
   * responding to docking entirely.
   */
  function abortDrag() {
    if (!dragging) return;
    dragging = false;
    if (dragRaf) cancelAnimationFrame(dragRaf);
    dragRaf = 0;
    samples = [];
    host.dragCancel().catch(() => {});
  }

  // The island being hidden mid-drag is the common case, and it produces no
  // pointer event at all — only a state change.
  $effect(() => {
    if (!island.visible) abortDrag();
  });

  $effect(() => () => {
    if (dragRaf) cancelAnimationFrame(dragRaf);
  });
</script>

<div
  class="stage"
  class:expanded={island.expanded}
  class:playing={island.playing}
  data-backdrop={island.info?.backdrop ?? "acrylic"}
  style:--dur="{island.transitionMs}ms"
  style:--ease={EASE_CSS}
  style:--accent={accent.base}
  style:--accent-fg={accent.fg}
  style:--glow={accent.glow}
  role="group"
  aria-label="Now playing"
  class:dragging
  class:mirrored={island.mirrored}
  onpointerenter={() => hover(true)}
  onpointerleave={() => hover(false)}
  onwheel={onWheel}
  onpointerdown={onDragPointerDown}
  onpointermove={onDragPointerMove}
  onpointerup={onDragPointerUp}
  onpointercancel={abortDrag}
  onlostpointercapture={abortDrag}
>
  <div class="veil" aria-hidden="true"></div>
  <div class="tint" aria-hidden="true"></div>

  <div class="art-slot" style:transform={artTransform}>
    <AlbumArt src={now?.artDataUri ?? null} revision={now?.revision ?? 0} accent={accent.base} />
  </div>

  <!-- Collapsed: title and a play indicator, nothing else. -->
  <div class="peek" aria-hidden={island.expanded}>
    <!--
      Scroll only while the capsule is both on screen *and* playing. A marquee is
      a forever-animation: left running behind a parked window, or on a paused
      track, it keeps the compositor awake and burns CPU during what should be
      true idle. A paused long title stays ellipsised — hovering reveals it.
    -->
    <Marquee
      text={title}
      active={island.state === "collapsed" && island.playing && !showVolume}
    />
    <div class="pulse" class:beating={island.playing && island.visible}>
      <i></i><i></i><i></i><i></i>
    </div>
  </div>

  <!-- Volume readout. Overlays the collapsed row while the wheel is in use, so
       the capsule answers the gesture without having to expand. -->
  <div class="vol-hud" class:on={showVolume && !island.expanded} aria-hidden={!showVolume}>
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 9.5h3.2L12 5.4v13.2L7.2 14.5H4z" />
      {#if volumeMuted}
        <path class="stroke" d="M16.5 9.5l4 5M20.5 9.5l-4 5" />
      {:else}
        <path class="stroke" d="M15.6 9.2a4 4 0 0 1 0 5.6" />
      {/if}
    </svg>
    <!-- Named only when the change was app-specific. Without it a per-app
         scroll is indistinguishable from a master one, and the user has no way
         to tell whose volume just moved. -->
    <span class="vol-app" class:system={!hud.label}>{hudLabel}</span>
    <div class="vol-bar">
      <div class="vol-fill" style:transform="scaleX({volumeMuted ? 0 : hud.scalar})"></div>
    </div>
    <span class="vol-pct">{volumePct}</span>
  </div>

  <!-- Expanded: the full panel. -->
  <div class="panel" aria-hidden={!island.expanded} inert={!island.expanded}>
    <!-- Keyed on the session, so switching source replays the entrance rather
         than swapping text in place. The album art already crossfades on its
         own revision; this gives the words the same courtesy. -->
    {#key now?.sessionId}
      <div class="meta swapping">
        <div class="title"><Marquee text={title} active={island.expanded} /></div>
        <!-- The lyric takes the artist's place rather than adding a row. The
             panel is 118px and every row is already spoken for; a third line
             would push the progress bar into the controls. The artist is on the
             share card and in the tooltip, so nothing is actually lost. -->
        <div class="artist" class:lyric={lyric !== null}>{lyric ?? artist ?? "—"}</div>
      </div>
    {/key}

    <div class="rail-row"><Progress /></div>

    <div class="tray">
      <Controls />
      <Volume />
      {#if now?.source}
        {#if island.canSwitch}
          <!-- Only a button when there is somewhere to switch to. A control that
               looks pressable but does nothing is worse than a label. -->
          <button
            type="button"
            class="source switch"
            title="Switch source ({island.sessions.length} playing)"
            aria-label="Switch source, currently {now.source}"
            onclick={() => host.cycleSession().catch(() => {})}
          >
            <span class="source-name">{now.source}</span>
            <svg class="swap" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M7 4.5 3.5 8 7 11.5M3.5 8h12M17 12.5 20.5 16 17 19.5M20.5 16h-12" />
            </svg>
          </button>
        {:else}
          <span class="source">{now.source}</span>
        {/if}
      {/if}
    </div>
  </div>
</div>

<style>
  /* Fills the window exactly. The window *is* the capsule — DWM rounds its
     corners and paints the Acrylic — so there is no shape to draw here and
     nothing to clip. Any size set here would fight the host's animation.
     The radius only has to agree with DWMWCP_ROUND so the inner rim follows the
     same curve as the window edge. */
  .stage {
    position: fixed;
    inset: 0;
    border-radius: 8px;
    overflow: hidden;
    pointer-events: auto;
    color: var(--ink);
    cursor: grab;
  }

  /* Held: the whole capsule is the drag handle, so the cursor says so. */
  .stage.dragging {
    cursor: grabbing;
  }

  /* Controls keep their own cursor — they are not drag surfaces. */
  .stage :global(button),
  .stage :global(.hit) {
    cursor: pointer;
  }

  /* The glass surface. Two things make this read as glass rather than as a grey
     fill: a gradient (light direction) and asymmetric edges (a bright specular
     top, faint sides, and a dark bottom that reads as thickness). A flat wash
     with a uniform 1px outline is what made the old capsule look like plastic. */
  .veil {
    position: absolute;
    inset: 0;
    background: var(--glass);
    box-shadow:
      inset 0 1px 0 0 var(--edge-top),
      inset 0 0 0 1px var(--edge-side),
      inset 0 -1px 0 0 var(--edge-bottom);
    border-radius: inherit;
  }

  /* Album-art colour bleeding through the glass — the whole reason the host
     extracts an accent, and the difference between a capsule that responds to
     what is playing and one that is grey forever.
     It stays low-alpha because this sits *over* the backdrop: a strong value
     here occludes the desktop just as surely as a grey fill would. */
  .tint {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(
        130% 150% at 10% 100%,
        color-mix(in srgb, var(--accent) var(--tint-strength), transparent) 0%,
        transparent 60%
      ),
      linear-gradient(180deg, color-mix(in srgb, var(--accent) 7%, transparent) 0%, transparent 55%);
    border-radius: inherit;
    pointer-events: none;
    /* Colour changes ride the same curve as everything else, so a track change
       washes the new accent across the glass instead of snapping. */
    transition: background var(--dur) var(--ease);
  }

  .art-slot {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 96px;
    height: 96px;
    border-radius: 18px;
    transform-origin: bottom left;
    /* transform only — this is the one thing that must never drop a frame. */
    transition: transform var(--dur) var(--ease);
    will-change: transform;
  }

  /* --- mirrored layout (docked against the right edge) ---
     The window shrinks toward whichever edge it is pinned to. On a right dock
     that is the right edge, so a left-to-right layout sends the artwork sliding
     across the screen on every collapse while the pinned edge sits still. Every
     rule here is the same layout reflected, so the contents stay against the
     edge that does not move and the collapse reads as the capsule closing. */
  .mirrored .art-slot {
    left: auto;
    right: 0;
    transform-origin: bottom right;
  }

  .mirrored .peek,
  .mirrored .vol-hud {
    left: 14px;
    right: 50px;
    flex-direction: row-reverse;
  }

  .mirrored .panel {
    left: 16px;
    right: 126px;
  }

  .mirrored .tray {
    flex-direction: row-reverse;
  }

  /* --- collapsed layer --- */

  /* Bottom-anchored, like the artwork. The window grows upward out of the
     taskbar, so anything pinned to the top would slide away from the capsule's
     lower edge as it expands. A fixed height also stops the contents relaying
     out on every frame of the animation. */
  .peek {
    position: absolute;
    left: 50px;
    right: 14px;
    bottom: 0;
    height: 44px;
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 12.5px;
    font-weight: 550;
    letter-spacing: -0.005em;
    text-shadow: var(--text-shadow);
    opacity: 1;
    transition:
      opacity calc(var(--dur) * 0.4) var(--ease),
      transform var(--dur) var(--ease);
  }

  .peek :global(.viewport) {
    flex: 1;
    min-width: 0;
  }

  .expanded .peek {
    opacity: 0;
    /* Leaves upward as the panel arrives from below — the two layers pass each
       other rather than cross-dissolving in place. */
    transform: translateY(-8px);
    pointer-events: none;
  }

  /* Four bars instead of three, at staggered rest heights: an even row of equal
     bars reads as an icon, an uneven one reads as a level meter. */
  .pulse {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 2.5px;
    height: 13px;
    flex: none;
  }

  /* The glow lives on the *container*, not on the bars.
     A blurred box-shadow attached to an element that is being transformed has to
     be re-rasterised on every frame — it cannot ride the compositor the way a
     bare transform does. Putting the glow on a static parent keeps the four
     animating elements as pure solid-colour transforms, which is the cheapest
     thing a compositor can do, and the halo looks the same. */
  .pulse::after {
    content: "";
    position: absolute;
    inset: -5px -4px -3px;
    background: radial-gradient(
      60% 70% at 50% 70%,
      color-mix(in srgb, var(--accent) 42%, transparent) 0%,
      transparent 72%
    );
    pointer-events: none;
  }

  .pulse i {
    position: relative;
    width: 2.5px;
    /* Full height, scaled down — see `bounce`. */
    height: 13px;
    border-radius: 1.5px;
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--accent) 65%, white) 0%,
      var(--accent) 100%
    );
    transform-origin: bottom;
    transform: scaleY(0.3);
    transition: transform 320ms var(--ease);
  }

  .pulse i:nth-child(1) { transform: scaleY(0.34); }
  .pulse i:nth-child(2) { transform: scaleY(0.62); }
  .pulse i:nth-child(3) { transform: scaleY(0.26); }
  .pulse i:nth-child(4) { transform: scaleY(0.48); }

  /* Only animates while audio is actually playing and the capsule is on screen,
     so a paused or parked island is completely static — no compositor work, no
     wakeups. Paused, the bars settle to their staggered rest heights above. */
  .beating i {
    animation: bounce 1080ms var(--ease) infinite alternate;
  }
  .beating i:nth-child(1) { animation-delay: 0ms; }
  .beating i:nth-child(2) { animation-delay: 160ms; }
  .beating i:nth-child(3) { animation-delay: 330ms; }
  .beating i:nth-child(4) { animation-delay: 90ms; }

  /* `transform`, not `height`. Animating height re-runs layout on every frame
     for the whole flex row; scaleY stays on the compositor and costs the main
     thread nothing — which matters because this is the one animation that runs
     continuously for as long as music plays. */
  @keyframes bounce {
    from {
      transform: scaleY(0.24);
    }
    to {
      transform: scaleY(1);
    }
  }

  /* --- transient volume readout (collapsed) --- */

  .vol-hud {
    position: absolute;
    left: 50px;
    right: 14px;
    bottom: 0;
    height: 44px;
    display: flex;
    align-items: center;
    gap: 9px;
    opacity: 0;
    transform: translateY(6px);
    pointer-events: none;
    color: var(--ink);
    transition:
      opacity 180ms var(--ease),
      transform 260ms var(--ease);
  }

  .vol-hud.on {
    opacity: 1;
    transform: none;
  }

  /* The title steps aside rather than being covered, so the two never overlap
     mid-fade. */
  .peek {
    transition:
      opacity calc(var(--dur) * 0.4) var(--ease),
      transform var(--dur) var(--ease);
  }

  /* `.peek` precedes `.vol-hud` in the DOM, so a sibling combinator cannot
     reach it; `:has()` on the shared parent can. */
  .stage:has(.vol-hud.on) .peek {
    opacity: 0;
    transform: translateY(-6px);
  }

  .vol-hud svg {
    width: 15px;
    height: 15px;
    fill: currentColor;
    flex: none;
  }

  .vol-hud .stroke {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.9;
    stroke-linecap: round;
  }

  /* Capped rather than allowed to push the bar out of the capsule: a long window
     title must shorten, never squeeze the level readout to nothing. */
  .vol-app {
    font-size: 10px;
    font-weight: 640;
    letter-spacing: -0.005em;
    color: var(--ink-dim);
    max-width: 96px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: none;
    text-shadow: var(--text-shadow);
  }

  /* The master reads as a category rather than a name, so it is set apart from
     an application's own label instead of impersonating one. */
  .vol-app.system {
    font-weight: 560;
    opacity: 0.72;
    letter-spacing: 0.01em;
  }

  .vol-bar {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 4px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.3);
    box-shadow: inset 0 1px 1px rgba(0, 0, 0, 0.32);
    overflow: hidden;
  }

  .vol-fill {
    position: absolute;
    inset: 0;
    transform-origin: left center;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--accent) 55%, white),
      var(--accent)
    );
    box-shadow: 0 0 10px -1px color-mix(in srgb, var(--accent) 75%, transparent);
    transition: transform 140ms var(--ease);
  }

  .vol-pct {
    font-size: 11px;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    min-width: 22px;
    text-align: right;
    flex: none;
    text-shadow: var(--text-shadow);
  }

  /* --- expanded layer --- */

  .panel {
    position: absolute;
    left: 126px;
    right: 16px;
    /* Bottom-anchored with a fixed height, for the same reason as `.peek`:
       expanded height (148) minus the 16/14 insets. */
    bottom: 14px;
    height: 118px;
    display: flex;
    flex-direction: column;
    opacity: 0;
    transform: translateY(10px);
    pointer-events: none;
    transition:
      opacity calc(var(--dur) * 0.5) var(--ease) calc(var(--dur) * 0.18),
      transform var(--dur) var(--ease);
  }

  .expanded .panel {
    opacity: 1;
    transform: none;
    pointer-events: auto;
  }

  /* Staggered arrival. The rows settle in reading order rather than all at once,
     which is what makes the expansion feel composed instead of abrupt. The
     delays are fractions of the host's duration, so they stay in step if the
     transition length ever changes. */
  .meta,
  .rail-row,
  .tray {
    opacity: 0;
    transform: translateY(6px);
    transition:
      opacity calc(var(--dur) * 0.45) var(--ease),
      transform calc(var(--dur) * 0.6) var(--ease);
  }
  .expanded .meta {
    opacity: 1;
    transform: none;
    transition-delay: calc(var(--dur) * 0.16);
  }
  .expanded .rail-row {
    opacity: 1;
    transform: none;
    transition-delay: calc(var(--dur) * 0.26);
  }
  .expanded .tray {
    opacity: 1;
    transform: none;
    transition-delay: calc(var(--dur) * 0.34);
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title {
    font-size: 14.5px;
    font-weight: 640;
    letter-spacing: -0.012em;
    line-height: 1.25;
    text-shadow: var(--text-shadow);
  }

  .artist {
    font-size: 11.5px;
    font-weight: 480;
    color: var(--ink-dim);
    line-height: 1.35;
    letter-spacing: -0.003em;
    text-shadow: var(--text-shadow);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* A lyric is the line worth reading, so it is brighter than the artist name it
     replaces. Same size and position, so swapping between them never reflows. */
  .artist.lyric {
    color: var(--ink);
    font-weight: 560;
  }

  .rail-row {
    margin-top: 12px;
  }

  .tray {
    margin-top: auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  /* Tinted with the accent rather than plain white, so the badge belongs to the
     same palette as the rest of the capsule.
     No `max-width` and no ellipsis: a hard cap is what produced "SPOTI…". The
     badge is content-sized and never shrinks (`flex: none`), and the tighter
     type below means real source names fit outright. Genuinely long ones are
     handled by the host, which shortens the AUMID to a single word. */
  /* Replays whenever the `{#key}` block is recreated — i.e. on a source switch.
     Transform + opacity only, so it stays on the compositor. */
  .meta.swapping {
    animation: source-swap 320ms var(--ease) both;
  }

  @keyframes source-swap {
    from {
      opacity: 0;
      transform: translateX(-8px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }

  .source {
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.055em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--accent) 45%, white);
    padding: 3.5px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 22%, transparent);
    white-space: nowrap;
    flex: none;
    transition:
      color var(--dur) var(--ease),
      background var(--dur) var(--ease),
      box-shadow 180ms var(--ease),
      transform 200ms var(--ease-snap);
  }

  /* Interactive variant: reads as a control, not a label. */
  .source.switch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
  }

  .source.switch:hover {
    color: color-mix(in srgb, var(--accent) 25%, white);
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 42%, transparent);
    transform: scale(1.04);
  }

  .source.switch:active {
    transform: scale(0.94);
    transition-duration: 50ms;
  }

  .source.switch:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, white);
    outline-offset: 2px;
  }

  /* Roomier now that the host trims editions like "Desktop": real brand names
     ("Spotify", "Telegram", "Firefox") fit whole at this width. */
  .source-name {
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 84px;
  }

  .swap {
    width: 9px;
    height: 9px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.2;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.75;
  }

  /* Controls hug the left, the volume group takes the slack, the badge hugs the
     right. `min-width: 0` on the middle child is what lets it actually shrink
     instead of forcing the badge off the end. */
  .tray {
    gap: 8px;
  }
</style>
