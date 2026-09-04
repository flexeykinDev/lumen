<script lang="ts">
  import { host } from "../lib/bridge";
  import { formatTime, island } from "../lib/state.svelte";

  // The host never sends ticks — only a position sample and the moment it was
  // taken. Everything between samples is interpolated here, and the rAF loop
  // exists *only* while the panel is open and playing. Collapsed, paused, or
  // hidden, this component runs no timer at all. That is what keeps the app at
  // 0% CPU at idle.

  let clock = $state(performance.now());

  $effect(() => {
    if (!island.expanded || !island.playing) return;

    let frame = 0;
    const tick = () => {
      clock = performance.now();
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  });

  const duration = $derived(island.now?.timeline.durationSec ?? 0);
  // "No duration reported" is NOT the same as "live stream", and we cannot tell
  // the two apart from SMTC. A Telegram video note, a voice message and an
  // internet radio station all report zero here. Labelling that LIVE asserts
  // something we do not know and that is usually false, so the UI now simply
  // declines to state a duration.
  const unknownLength = $derived(duration <= 0);
  const seekable = $derived(duration > 0);

  // --- seeking -------------------------------------------------------------
  //
  //   dragging  the pointer owns the position; the host is ignored entirely
  //   settling  we let go, the host has not caught up, so we hold our value
  //   idle      the host owns the position
  //
  // Without `settling` the thumb snaps back to the old position for the
  // 200-400 ms SMTC takes to round-trip through the source app, then jumps
  // forward again. That double-jump is what makes a seek feel broken.

  // `dragging` = the button is down (drives the visual states: taller rail,
  //              grabbed thumb, brighter fill).
  // `scrubbing` = the pointer has actually MOVED while held, which is the only
  //              state that should suppress the glide.
  //
  // Keeping these separate matters: pressing down is itself a jump to the click
  // position and must glide, and collapsing the two made the press instantly
  // adopt the 50 ms scrub transition — the click-jump never animated at all.
  let dragging = $state(false);
  let scrubbing = $state(false);
  let pendingPos = $state<number | null>(null);
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
  let rail = $state<HTMLDivElement | null>(null);

  // --- scrub loop (deliberately NOT reactive state) -------------------------
  //
  // `pointerX` is a plain variable written on every pointermove and read once
  // per animation frame. Gaming mice report at 500-1000 Hz against a 60-75 Hz
  // display, so assigning reactive state per event queued several renders per
  // frame — work that could never be shown, and the main cause of the stutter.
  //
  // `railBox` is cached at pointerdown. Calling getBoundingClientRect() inside
  // the loop forces a synchronous layout every frame, which is the other half.
  let pointerX = 0;
  let railBox: DOMRect | null = null;
  let scrubRaf = 0;

  function positionFromX(clientX: number): number | null {
    if (!railBox || railBox.width <= 0 || !seekable) return null;
    const ratio = Math.min(1, Math.max(0, (clientX - railBox.left) / railBox.width));
    return ratio * duration;
  }

  /** One state write per displayed frame, no more. */
  function startScrubLoop() {
    if (scrubRaf) return;
    const pump = () => {
      if (!dragging) {
        scrubRaf = 0;
        return;
      }
      const next = positionFromX(pointerX);
      if (next !== null) pendingPos = next;
      scrubRaf = requestAnimationFrame(pump);
    };
    scrubRaf = requestAnimationFrame(pump);
  }

  function stopScrubLoop() {
    if (scrubRaf) cancelAnimationFrame(scrubRaf);
    scrubRaf = 0;
  }

  /**
   * Glide the fill only for *discontinuous* jumps.
   *
   * A blanket `transition` on the fill would be wrong here: this bar is
   * re-rendered every animation frame from the rAF interpolator while music
   * plays, and a transition would chase each of those micro-updates, leaving the
   * fill permanently ~180 ms behind the real position — a bar that visibly lags
   * the music. So the transition is switched on only for a jump the user caused
   * (a click, a keyboard seek) and switched off again for continuous playback
   * and for 1:1 drag tracking.
   */
  let gliding = $state(false);
  let glideTimer: ReturnType<typeof setTimeout> | undefined;

  function glide() {
    gliding = true;
    clearTimeout(glideTimer);
    glideTimer = setTimeout(() => (gliding = false), 200);
  }

  $effect(() => () => {
    clearTimeout(settleTimer);
    clearTimeout(glideTimer);
    stopScrubLoop();
  });

  // Once the host reports a position close to what we asked for, hand control
  // back. Any later event is the source genuinely moving, not our own echo.
  $effect(() => {
    const hostPos = island.now?.timeline.positionSec ?? 0;
    if (!dragging && pendingPos !== null && Math.abs(hostPos - pendingPos) < 1.5) {
      clearTimeout(settleTimer);
      // Handing back to the host is itself a small discontinuity: the source
      // rarely lands on the exact millisecond we asked for. Glide across that
      // correction so the release settles instead of twitching.
      glide();
      pendingPos = null;
    }
  });

  const livePosition = $derived(island.positionAt(clock));
  const position = $derived(pendingPos ?? livePosition);
  const fraction = $derived(duration > 0 ? Math.min(1, Math.max(0, position / duration)) : 0);

  function onPointerDown(event: PointerEvent) {
    if (!seekable || event.button !== 0 || !rail) return;
    // Measure once for the whole gesture.
    railBox = rail.getBoundingClientRect();
    pointerX = event.clientX;
    const next = positionFromX(pointerX);
    if (next === null) return;

    // The initial jump from wherever playback was to where they clicked is the
    // one move that should glide.
    glide();
    dragging = true;
    scrubbing = false;
    pendingPos = next;
    clearTimeout(settleTimer);
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    startReleaseWatch();
    event.preventDefault();
  }

  // --- release watchdog -----------------------------------------------------
  //
  // Dragging the thumb to 0:00 and letting go *outside* the window used to
  // leave the bar showing 0:00 for ever while the track carried on from where
  // it was. The seek was never sent: the `pointerup` landed outside a window
  // that had lost pointer capture, so the handler that commits it never ran,
  // and the scrub loop kept re-asserting the last dragged position.
  //
  // Losing the event is not something this side can prevent, so it asks the
  // host instead: `GetAsyncKeyState` knows whether the button is still down no
  // matter where the pointer went. Polling only exists while a scrub is in
  // progress, and it is four calls a second.

  let releaseWatch: ReturnType<typeof setInterval> | undefined;

  function startReleaseWatch() {
    stopReleaseWatch();
    releaseWatch = setInterval(async () => {
      if (!dragging) {
        stopReleaseWatch();
        return;
      }
      try {
        if (!(await host.pointerPressed())) onPointerUp();
      } catch {
        // The host is the authority; if it cannot answer, keep waiting for a
        // real pointerup rather than committing a seek nobody asked for.
      }
    }, 250);
  }

  function stopReleaseWatch() {
    if (releaseWatch !== undefined) clearInterval(releaseWatch);
    releaseWatch = undefined;
  }

  $effect(() => () => {
    stopReleaseWatch();
    clearTimeout(settleTimer);
  });

  function onPointerMove(event: PointerEvent) {
    if (!dragging) return;
    // Cheapest possible handler: one assignment. Everything else happens on the
    // frame, so a 1000 Hz mouse costs the same as a 125 Hz one.
    pointerX = event.clientX;

    if (!scrubbing) {
      // First real movement promotes the press into a scrub. From here tracking
      // must be 1:1 under the pointer, so the glide is cancelled outright.
      scrubbing = true;
      gliding = false;
      clearTimeout(glideTimer);
    }
    startScrubLoop();
  }

  function onPointerUp(event?: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    scrubbing = false;
    stopScrubLoop();
    stopReleaseWatch();
    if (event) {
      const el = event.currentTarget as HTMLElement | null;
      // `releasePointerCapture` throws if the capture was already lost, which
      // is precisely the case this function exists to survive.
      try {
        el?.releasePointerCapture(event.pointerId);
      } catch {
        /* capture was already gone */
      }
    }

    const target = pendingPos;
    if (target === null) return;
    // Recorded before the round-trip: the source republishes a stale timeline
    // almost immediately, and the optimistic position has to already be in place
    // when it lands.
    island.noteSeek(target);
    // A rejected seek does *not* clear the optimistic position. SMTC returns
    // false for sources that seek anyway, and dropping the override there is
    // what leaves the counter showing the source's stale 0:00. Reconciliation
    // in `state.svelte.ts` decides when to hand authority back.
    host.seek(target).catch(() => {});
    clearTimeout(settleTimer);
    settleTimer = setTimeout(() => (pendingPos = null), 2500);
  }

  /** Arrow keys nudge, Home/End jump. 5s steps, 15s with Shift. */
  function onKeyDown(event: KeyboardEvent) {
    if (!seekable) return;
    const step = event.shiftKey ? 15 : 5;
    let target: number | null = null;
    if (event.key === "ArrowRight") target = position + step;
    else if (event.key === "ArrowLeft") target = position - step;
    else if (event.key === "Home") target = 0;
    else if (event.key === "End") target = duration;
    if (target === null) return;

    event.preventDefault();
    const clamped = Math.min(duration, Math.max(0, target));
    glide();
    pendingPos = clamped;
    island.noteSeek(clamped);
    host.seek(clamped).catch(() => (pendingPos = null));
    clearTimeout(settleTimer);
    settleTimer = setTimeout(() => (pendingPos = null), 2500);
  }

  // The indeterminate sweep went with the LIVE label: it animated a bar back and
  // forth to say "streaming", which was the same unfounded claim in motion form.
  // An unknown-length track now shows an empty rail — we know the elapsed time
  // and nothing else, and the UI says exactly that.
</script>

<div class="progress" class:seekable class:dragging class:scrubbing>
  <div
    class="hit"
    bind:this={rail}
    role="slider"
    aria-label="Seek"
    aria-disabled={!seekable}
    aria-valuemin={0}
    aria-valuemax={Math.round(duration)}
    aria-valuenow={Math.round(position)}
    aria-valuetext={formatTime(position)}
    tabindex={seekable ? 0 : -1}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onkeydown={onKeyDown}
  >
    <!-- Fill and thumb are driven by the SAME fraction through the same kind of
         property (both layout, both transitioned with an identical curve and
         duration), so they cannot drift apart mid-glide. An earlier version
         animated the fill on `transform` while the thumb jumped via `left` with
         no transition at all — the bar glided and the dot teleported. -->
    <div class="rail">
      <div class="fill" class:gliding style:width="{fraction * 100}%"></div>
    </div>
    {#if seekable}
      <span class="thumb" class:gliding style:left="{fraction * 100}%"></span>
    {/if}
  </div>

  <div class="times">
    <span>{formatTime(position)}</span>
    <span class="right" class:unknown={unknownLength}>
      {unknownLength ? "--:--" : formatTime(duration)}
    </span>
  </div>
</div>

<style>
  .progress {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* A 4px rail is a 4px target — unusable. The hit area is 16px tall and
     transparent, so the bar stays thin while the pointer gets something real
     to aim at. */
  .hit {
    position: relative;
    height: 16px;
    display: flex;
    align-items: center;
    touch-action: none;
  }

  .seekable .hit {
    cursor: pointer;
  }

  .hit:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, white);
    outline-offset: 3px;
    border-radius: 4px;
  }

  .rail {
    position: relative;
    width: 100%;
    height: 4px;
    border-radius: 999px;
    /* Inset rather than flat: the unfilled track reads as a groove cut into the
       glass instead of a grey line drawn on top of it. */
    background: rgba(0, 0, 0, 0.28);
    box-shadow:
      inset 0 1px 1px rgba(0, 0, 0, 0.3),
      inset 0 -1px 0 rgba(255, 255, 255, 0.06);
    overflow: hidden;
    transition:
      height 160ms var(--ease),
      background-color 160ms var(--ease);
  }

  /* Grows under the pointer — the affordance that says "this is draggable". */
  .seekable .hit:hover .rail {
    height: 5.5px;
  }

  /* Held: taller again, and the groove darkens so the accent fill gains
     contrast against it. Weight under the finger. */
  .dragging .rail {
    height: 7px;
    background: rgba(0, 0, 0, 0.42);
  }

  /* Sized by `width`, not `scaleX`.
     scaleX is cheaper in the abstract, but it distorts everything painted on the
     element: the glow below gets squashed horizontally at low progress, and the
     gradient stretches instead of being revealed. This is one small absolutely
     positioned box with no children and nothing depending on its size, so the
     layout it triggers is a single-box relayout — negligible, and it buys an
     undistorted glow plus exact parity with the thumb's `left`. */
  .fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 0;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--accent) 60%, white) 0%,
      var(--accent) 70%,
      color-mix(in srgb, var(--accent) 80%, white) 100%
    );
    box-shadow: 0 0 12px -1px color-mix(in srgb, var(--accent) 80%, transparent);
    transition: filter 160ms var(--ease);
  }

  /* Applied only for user-caused jumps — see `glide()` in the script.
     A blanket transition here would be a bug: the fill is re-rendered every
     animation frame from the rAF interpolator while music plays, so it would
     chase each micro-update and sit permanently ~350 ms behind the music. */
  .fill.gliding {
    transition:
      width 450ms cubic-bezier(0.16, 1, 0.3, 1),
      filter 160ms var(--ease);
  }

  /* Scrubbing is 1:1 with the pointer, so there is nothing to interpolate:
     any transition here — even 50 ms — puts the fill behind the cursor for the
     whole gesture, which is exactly the drag lag being fixed. The rAF loop is
     already the frame-rate limiter.
     Keyed on `.scrubbing`, NOT `.dragging`: a press that has not moved yet is
     still a click-jump and must keep the glide above. */
  .scrubbing .fill,
  .scrubbing .fill.gliding {
    transition: none !important;
  }

  .dragging .fill {
    filter: brightness(1.18) saturate(1.1);
  }

  .thumb {
    position: absolute;
    top: 50%;
    width: 11px;
    height: 11px;
    margin-left: -5.5px;
    border-radius: 50%;
    background: #fff;
    box-shadow:
      0 1px 4px rgba(0, 0, 0, 0.5),
      0 0 8px color-mix(in srgb, var(--accent) 80%, transparent);
    transform: translateY(-50%) scale(0);
    /* Two independent animations on one element:
         `transform` — the elastic pop on hover/press (y2 = 1.56, so it
                       overshoots past the target and settles by itself)
         `left`      — travel along the rail, which must match the fill exactly
       Only the pop is active at rest; `left` gets its curve from `.gliding`. */
    transition: transform 260ms cubic-bezier(0.34, 1.56, 0.64, 1);
    pointer-events: none;
  }

  /* Identical curve and duration to `.fill.gliding`. That equality is the whole
     mechanism keeping the dot welded to the end of the bar during a click-jump;
     any mismatch shows up as the dot leading or trailing the fill. */
  .thumb.gliding {
    transition:
      transform 350ms cubic-bezier(0.34, 1.56, 0.64, 1),
      left 450ms cubic-bezier(0.16, 1, 0.3, 1);
  }

  .seekable .hit:hover .thumb {
    transform: translateY(-50%) scale(1.3);
  }

  /* Held: slightly smaller than hover, so the grab reads as a pinch not a grow.
     Travel timing is left alone here so a stationary press still glides. */
  .dragging .thumb {
    transform: translateY(-50%) scale(1.15);
  }

  /* Scrubbing: the dot must sit exactly under the pointer, with no easing of
     any kind between frames. */
  .scrubbing .thumb,
  .scrubbing .thumb.gliding {
    transition: none !important;
  }

  .times {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    font-weight: 550;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
    color: var(--ink-faint);
    transition: color 160ms var(--ease);
  }

  .dragging .times {
    color: var(--ink);
  }

  /* Dimmed rather than accented: an unknown length is an absence of
     information, not a feature worth highlighting. */
  .unknown {
    opacity: 0.55;
  }
</style>
