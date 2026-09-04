<script lang="ts">
  import AlbumArt from "./AlbumArt.svelte";
  import Controls from "./Controls.svelte";
  import Marquee from "./Marquee.svelte";
  import Clawd from "./Clawd.svelte";
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

  // Lyric follow, with no polling at all.
  //
  // The obvious build is a timer that re-reads the clock and repaints. Even at
  // 200 ms that is five wakeups a second forever, and the karaoke fill would
  // step rather than sweep. Instead:
  //
  //   - the *sweep* is a CSS animation over `--fill`, so it runs on the
  //     compositor at whatever frame rate the machine is already painting at,
  //     and costs the main thread nothing;
  //   - a negative `animation-delay` starts it partway through, which is what
  //     makes a seek or a late-arriving lyric land in the right place;
  //   - `animation-play-state` follows playback, so pausing freezes the sweep
  //     without any JavaScript noticing;
  //   - and the only timer is a single `setTimeout` scheduled for the exact
  //     moment the next line begins.
  //
  // So one wakeup per lyric line, rather than five per second.
  // The spectrum gate.
  //
  // This is the one part of Lumen that costs CPU while it runs — it captures
  // audio and transforms it twenty times a second. So the capture exists only
  // while the bars are actually on screen: expanded, playing, and switched on.
  // Collapsed, hidden or paused, the thread does not exist and the audio
  // endpoint is closed.
  //
  // Driven from here rather than the host because this is the only side that
  // knows what is visible.
  const spectrumWanted = $derived(
    island.expanded && island.playing && (island.config?.spectrum?.enabled ?? true),
  );
  $effect(() => {
    const on = spectrumWanted;
    host.spectrumEnable(on).catch(() => {});
    // Stopping on teardown as well: a reload must not leave a capture running
    // with nothing listening to it.
    return () => {
      if (on) host.spectrumEnable(false).catch(() => {});
    };
  });

  let lyricEpoch = $state(0);
  const lyric = $derived.by(() => {
    // `lyricEpoch` is read so the derivation re-runs when the boundary fires.
    lyricEpoch;
    return island.expanded ? island.lyricAt(performance.now()) : null;
  });

  $effect(() => {
    // Re-arms whenever the epoch, the track, playback or visibility changes.
    lyricEpoch;
    if (!island.expanded || !island.playing || island.lyrics.length === 0) return;

    const until = island.nextLyricBoundary(performance.now());
    if (until === null) return;
    // A floor so a pile-up of near-identical timestamps — which estimated
    // timings can produce on a short track — cannot spin this into a busy loop.
    const id = setTimeout(() => (lyricEpoch += 1), Math.max(50, until * 1000));
    return () => clearTimeout(id);
  });

  // Fitting a whole lyric line into 286px.
  //
  // The artist line this replaces is a name, so it was styled to end in an
  // ellipsis when it did not fit. A lyric ending in "…" is a different thing
  // entirely: the words that got cut are the ones being sung. So instead the
  // line is shrunk until it fits, down to a floor, and only wraps to a second
  // line when even that is not enough.
  //
  // Measured with `scrollWidth`, once per line rather than per frame: reading
  // it forces layout, and a lyric changes every few seconds.
  const LYRIC_MAX = 11.5;
  const LYRIC_MIN = 8.75;
  let lyricEl = $state<HTMLElement | null>(null);
  let lyricSize = $state(LYRIC_MAX);
  let lyricWrapped = $state(false);

  $effect(() => {
    // Re-runs on every new line, and on a resize of the panel.
    const text = lyric?.text;
    const el = lyricEl;
    if (!el || !text) return;

    lyricSize = LYRIC_MAX;
    lyricWrapped = false;

    // Two passes at most: overflow scales down by the ratio that would make it
    // fit, which lands within a rounding error of the right size in one step.
    const fits = () => el.scrollWidth <= el.clientWidth + 1;
    if (!fits()) {
      const ratio = el.clientWidth / el.scrollWidth;
      lyricSize = Math.max(LYRIC_MIN, LYRIC_MAX * ratio);
    }
  });

  $effect(() => {
    // A second pass after the shrink has been painted: if the smallest size
    // still overflows, the line is genuinely long and wrapping is the only way
    // to show all of it.
    const el = lyricEl;
    if (!el || !lyric?.text) return;
    if (lyricSize <= LYRIC_MIN + 0.01 && el.scrollWidth > el.clientWidth + 1) {
      lyricWrapped = true;
    }
  });

  // The easter egg.
  //
  // Seven clicks on the album art, within three seconds of each other. Seven
  // because three is an accident and twenty is a chore, and the window resets so
  // an ordinary double-click over a long session never accumulates into a
  // surprise. Nothing hints at it, which is the point: a secret advertised in a
  // settings list was never a secret.
  const UNLOCK_CLICKS = 7;
  const UNLOCK_WINDOW_MS = 3000;

  let taps = 0;
  let lastTap = 0;
  /** Set for one animation frame when he is first revealed. */
  let revealing = $state(false);

  function tapArt() {
    const cfg = island.config;
    if (!cfg || cfg.pet?.unlocked) return;

    const now = performance.now();
    taps = now - lastTap > UNLOCK_WINDOW_MS ? 1 : taps + 1;
    lastTap = now;
    if (taps < UNLOCK_CLICKS) return;

    taps = 0;
    revealing = true;
    setTimeout(() => (revealing = false), 1400);
    host
      .setConfig({ ...cfg, pet: { ...cfg.pet, unlocked: true, enabled: true } })
      .catch(() => {});
  }

  const pet = $derived(island.config?.pet);
  const showPet = $derived(Boolean(pet?.unlocked && pet?.enabled));

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

  /** Cosmetic only: the host owns the gesture itself. */
  let dragging = $state(false);

  /**
   * Hand the drag to the host and get out of the way.
   *
   * This used to be a full pointer-capture gesture here: `setPointerCapture`,
   * a `pointermove` handler, a `drag_to` per animation frame, and velocity
   * sampling. It broke under exactly the movement a drag is made of — the
   * window chases the pointer, a fast flick outruns it, the WebView loses
   * capture as the pointer crosses out of the moving window, and
   * `lostpointercapture` cancelled the gesture mid-air.
   *
   * The host reads `GetCursorPos` and `GetAsyncKeyState` instead, neither of
   * which a moving window can take away. All this side does is start it.
   */
  function onDragPointerDown(event: PointerEvent) {
    // Left button only, and never when the press started on a control.
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button, .hit, .volume")) return;

    dragging = true;
    host.dragStart().catch(() => (dragging = false));
  }

  /**
   * Clear the local "being dragged" styling once the button is up.
   *
   * The host owns the gesture, so this is cosmetic only — and it deliberately
   * does not tell the host anything: a `pointerup` that never arrives (the
   * pointer left the window during a throw) must not be able to strand the
   * capsule mid-drag, and one that arrives early must not cut the host's
   * gesture short.
   */
  function onDragPointerUp() {
    dragging = false;
  }

  // The capsule being hidden mid-drag no longer needs handling here: the host's
  // loop ends on the real button release, whatever the window is doing, and
  // `Island::cancel_drag` is only reached when the gesture never moved.
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
  onpointerup={onDragPointerUp}
>
  <div class="veil" aria-hidden="true"></div>
  <div class="tint" aria-hidden="true"></div>

  <!-- Behind the content and out of the layout: the bars are atmosphere, not a
       control, and must never push the panel around. -->
  {#if spectrumWanted && island.spectrum.length > 0}
    <div class="spectrum" aria-hidden="true">
      {#each island.spectrum as level, i (i)}
        <i style:transform="scaleY({Math.max(0.03, level)})"></i>
      {/each}
    </div>
  {/if}

  <!-- The album art doubles as the way in to the easter egg. It stays a plain
       div: it is not a control, and giving it button semantics would announce
       a secret to every screen reader that met it. -->
  <div
    class="art-slot"
    style:transform={artTransform}
    onclickcapture={tapArt}
    role="presentation"
  >
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
    <!-- Easter egg, opt-in, and entirely self-contained. -->
    {#if showPet && pet}
      <div class="clawd-slot" class:revealing>
        <Clawd {pet} playing={island.playing && island.visible} revision={now?.revision ?? 0} />
      </div>
    {/if}
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
        {#if lyric}
          <!-- Keyed on the text so each new line restarts its own animation
               rather than continuing the previous line's sweep. -->
          {#key lyric.text}
            <div
              bind:this={lyricEl}
              class="artist lyric"
              class:estimated={island.lyricsEstimated}
              class:wrapped={lyricWrapped}
              style:--sweep="{lyric.duration}s"
              style:--sweep-delay="{-lyric.elapsed}s"
              style:--sweep-state={island.playing ? "running" : "paused"}
              style:font-size="{lyricSize}px"
            >
              {lyric.text}
            </div>
          {/key}
        {:else}
          <div class="artist">{artist || "—"}</div>
        {/if}
      </div>
    {/key}

    <div class="rail-row"><Progress /></div>

    <div class="tray">
      <Controls />
      <!-- The same pet, in the layer the pointer is actually over. Hovering the
           capsule expands it, so a Clawd that only existed in the collapsed row
           was unclickable by construction. -->
      {#if showPet && pet}
        <div class="clawd-slot" class:revealing>
          <Clawd {pet} playing={island.playing && island.visible} revision={now?.revision ?? 0} expanded />
        </div>
      {/if}
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

  /* --- live spectrum ---
     Sits behind everything, anchored to the bottom edge, and takes no part in
     layout. Low contrast on purpose: this is the room the music is playing in,
     not something to read. */
  .spectrum {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 34px;
    display: flex;
    align-items: flex-end;
    gap: 2px;
    padding: 0 10px;
    opacity: 0.5;
    pointer-events: none;
    /* Fades out toward the top so the bars dissolve into the glass rather than
       ending on a hard line across the capsule. */
    mask-image: linear-gradient(180deg, transparent, #000 70%);
    -webkit-mask-image: linear-gradient(180deg, transparent, #000 70%);
  }

  .spectrum i {
    flex: 1;
    height: 100%;
    transform-origin: bottom;
    border-radius: 2px 2px 0 0;
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--accent) 70%, white) 0%,
      var(--accent) 100%
    );
    /* Bands arrive 20 times a second; the transition carries each bar between
       them so the movement reads as continuous at whatever frame rate the
       compositor is running. transform only, so it never touches layout. */
    transition: transform 60ms linear;
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

  /* Wrapper for the pet. Holds the reveal effect, so the component itself stays
     ignorant of where it is mounted. */
  .clawd-slot {
    position: relative;
    display: grid;
    place-items: center;
    flex: none;
  }

  /* One burst, the first time he is ever shown. */
  .clawd-slot.revealing::after {
    content: "";
    position: absolute;
    inset: -6px;
    border-radius: 50%;
    background: radial-gradient(circle, rgba(255, 255, 255, 0.55), transparent 65%);
    animation: clawd-pop 900ms var(--ease) both;
    pointer-events: none;
  }

  @keyframes clawd-pop {
    0% {
      opacity: 0;
      transform: scale(0.3);
    }
    35% {
      opacity: 1;
      transform: scale(1.15);
    }
    100% {
      opacity: 0;
      transform: scale(1.6);
    }
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

  /* --- karaoke fill ---
     `--fill` is registered so it can be *animated*. A bare custom property is a
     string as far as the engine is concerned and jumps from 0% to 100%; giving
     it a syntax makes it a real percentage the compositor can interpolate,
     which is what turns this from a step into a sweep. */
  @property --fill {
    syntax: "<percentage>";
    inherits: false;
    initial-value: 0%;
  }

  /* Two-stop gradient clipped to the glyphs: everything left of `--fill` is lit,
     everything right of it is the dim ink the artist line uses. One element and
     one animated property, so the whole effect stays on the compositor.
     The stops are coincident, so the boundary is a clean edge rather than a
     blur creeping ahead of the words. */
  .artist.lyric {
    font-weight: 560;
    /* Overrides the artist line's ellipsis: the fitting above guarantees the
       whole line is on screen, and a truncated lyric is worse than a small one. */
    text-overflow: clip;
    line-height: 1.3;
    background-image: linear-gradient(
      90deg,
      var(--ink) var(--fill),
      var(--ink-dim) var(--fill)
    );
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
    animation: karaoke var(--sweep, 3s) linear var(--sweep-delay, 0s) 1 both;
    animation-play-state: var(--sweep-state, paused);
  }

  /* The overflow case: a line too long even at the smallest size wraps onto a
     second row. The sweep is dropped with it — one horizontal gradient across a
     two-line box would light both rows at once, which reads as wrong rather
     than as karaoke. Whole words beat a pretty effect. */
  .artist.lyric.wrapped {
    white-space: normal;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    animation: none;
    background-image: none;
    color: var(--ink);
  }

  /* Estimated timings are a guess — plain lyrics spread evenly across the track
     — and they drift. Saying so quietly is better than presenting a guess with
     the confidence of a measurement: the words stay readable, the sweep just
     does not claim to be exact. */
  .artist.lyric.estimated {
    background-image: linear-gradient(
      90deg,
      color-mix(in srgb, var(--ink) 80%, transparent) var(--fill),
      var(--ink-faint) var(--fill)
    );
    font-style: italic;
  }

  @keyframes karaoke {
    from {
      --fill: 0%;
    }
    to {
      --fill: 100%;
    }
  }

  /* Someone who has asked for less motion should not get a sweeping highlight;
     the line still changes, it simply arrives fully lit. */
  @media (prefers-reduced-motion: reduce) {
    .artist.lyric {
      animation: none;
      background-image: none;
      color: var(--ink);
    }
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
    /* Shrinkable, and capped. A long player name ("Yandex Music") is a label,
       and a label must never push into the controls beside it — which is what
       `flex: none` on an uncapped name did to the volume slider. */
    flex: 0 1 auto;
    min-width: 0;
    max-width: 96px;
    overflow: hidden;
    text-overflow: ellipsis;
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
