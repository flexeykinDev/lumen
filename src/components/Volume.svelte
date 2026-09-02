<script lang="ts">
  import { host } from "../lib/bridge";
  import { island } from "../lib/state.svelte";

  // Compact readout for the expanded panel. The wheel is handled once on the
  // whole capsule (Island.svelte) rather than here, so scrolling adjusts volume
  // anywhere over the island — not only over this 52px bar.

  // Whichever volume last moved, not the master. The wheel targets the playing
  // app's own session now, so reading `island.volume` here left this bar frozen
  // at the master level while the number it was meant to show changed.
  const level = $derived(island.volumeHud ?? { label: null, ...island.volume });
  const pct = $derived(Math.round(level.scalar * 100));
  const muted = $derived(level.muted || level.scalar <= 0.0001);

  // --- first-run scroll hint ----------------------------------------------
  //
  // The wheel gesture is genuinely undiscoverable: nothing about a read-only
  // bar suggests it is scrollable. One hint, once, then never again.

  const HINT_KEY = "lumen.volumeHintSeen";

  // localStorage throws in some embedding contexts, and is per-origin — which
  // for a Tauri app is stable across launches. Treat any failure as "already
  // seen" so a storage error can never produce a toast on every single launch.
  function hintAlreadySeen(): boolean {
    try {
      return localStorage.getItem(HINT_KEY) === "1";
    } catch {
      return true;
    }
  }

  function markHintSeen() {
    try {
      localStorage.setItem(HINT_KEY, "1");
    } catch {
      /* storage unavailable; the in-memory flag still suppresses repeats */
    }
  }

  const russian = navigator.language?.toLowerCase().startsWith("ru") ?? false;

  const hintText = russian ? "Крутите колёсико мыши" : "Scroll wheel to adjust volume";

  /// Asked for explicitly, so it can afford to say more. The modifiers match
  /// `Island.onWheel`: Ctrl multiplies the step by five, Shift divides it by
  /// four.
  const detailText = russian
    ? "Колесо · Ctrl — крупный шаг · Shift — точный"
    : "Scroll · Ctrl for big steps · Shift for fine";

  let detailed = $state(false);
  let showHint = $state(false);
  let hintSpent = $state(hintAlreadySeen());
  let hintTimer: ReturnType<typeof setTimeout> | undefined;

  function offerHint() {
    if (hintSpent || showHint) return;
    detailed = false;
    showHint = true;
    clearTimeout(hintTimer);
    hintTimer = setTimeout(() => (showHint = false), 1800);
  }

  /// The bar does not move on a click, and a control that ignores a click owes
  /// an explanation. This one is deliberately *not* once-only: clicking it is a
  /// question, and a question asked twice deserves the same answer.
  function explain() {
    detailed = true;
    showHint = true;
    clearTimeout(hintTimer);
    hintTimer = setTimeout(() => (showHint = false), 3200);
  }

  function retireHint() {
    // Using the wheel proves the gesture was discovered — stop offering it.
    if (hintSpent) return;
    hintSpent = true;
    markHintSeen();
    clearTimeout(hintTimer);
    showHint = false;
  }

  // Any actual volume change means they found it (by wheel, by this button, or
  // by the system mixer) — either way the hint has served its purpose.
  $effect(() => {
    if (island.volumeTouchedAt) retireHint();
  });

  $effect(() => () => clearTimeout(hintTimer));
</script>

<div
  class="volume"
  class:muted
  role="group"
  aria-label="Volume"
  onpointerenter={offerHint}
  onpointerleave={() => (showHint = false)}
>
  <!-- Sits inside the capsule, not floating above it: the window *is* the
       capsule, so anything positioned outside these bounds would be clipped by
       the window edge rather than overlaying the desktop. -->
  <div class="hint" class:on={showHint} role="status" aria-live="polite">
    {showHint ? (detailed ? detailText : hintText) : ""}
  </div>

  <button
    type="button"
    class="speaker"
    aria-label={muted ? "Unmute" : "Mute"}
    onclick={() => {
      offerHint();
      // Same target as the wheel: the app the island is showing.
      host.volumeToggleMuteMedia().catch(() => {});
    }}
  >
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 9.5h3.2L12 5.4v13.2L7.2 14.5H4z" />
      {#if muted}
        <path class="stroke" d="M16.5 9.5l4 5M20.5 9.5l-4 5" />
      {:else}
        <path class="stroke" d="M15.6 9.2a4 4 0 0 1 0 5.6" />
        <path class="stroke wide" d="M18.1 6.9a7.4 7.4 0 0 1 0 10.2" />
      {/if}
    </svg>
  </button>

  <!-- A button around the readout rather than a slider: the level is changed by
       the wheel, and a click here is someone looking for the control that is
       not there. It answers instead of doing nothing. -->
  <button type="button" class="bar" aria-label={detailText} onclick={explain}>
    <div
      class="track"
      role="progressbar"
      aria-valuenow={pct}
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <div class="fill" style:transform="scaleX({muted ? 0 : level.scalar})"></div>
    </div>
  </button>

  <!-- Hidden until the group is hovered or the level moves. A permanently
       visible "100" is noise in a row this tight. -->
  <span class="pct" aria-hidden="true">{pct}</span>
</div>

<style>
  /* Elastic, and the only elastic thing in the tray row.
     Everything either side of it — transport, the pet, the source badge — is a
     fixed size, so when the row runs out of width this is what has to give.
     Without `flex: 1 1 auto` and a shrinkable track the group kept its full
     width, the row overflowed, and the bar ran underneath the source badge. */
  .volume {
    position: relative;
    display: flex;
    align-items: center;
    gap: 7px;
    /* Breathing room from the source badge on the right. */
    padding: 2px 4px 2px 2px;
    flex: 1 1 auto;
    min-width: 0;
  }

  .speaker {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    color: var(--ink-dim);
    flex: none;
    transition:
      color 160ms var(--ease),
      background-color 160ms var(--ease),
      transform 200ms var(--ease-snap);
  }

  .speaker:hover {
    color: var(--ink);
    background: var(--well-hover);
  }

  .speaker:active {
    transform: scale(0.86);
    transition-duration: 50ms;
  }

  .speaker:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, white);
    outline-offset: 1px;
  }

  svg {
    width: 14px;
    height: 14px;
    fill: currentColor;
  }

  .stroke {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.9;
    stroke-linecap: round;
  }

  .wide {
    opacity: 0.55;
  }

  /* The button is only a hit target: it adds padding so a 3px bar is clickable
     without making the bar itself any taller. */
  .bar {
    display: block;
    padding: 6px 0;
    border: 0;
    background: none;
    cursor: help;
    flex: 1 1 auto;
    min-width: 0;
  }

  .track {
    position: relative;
    width: 100%;
    /* Never wider than it needs to be, never so narrow it stops reading as a
       level: the range the row is allowed to squeeze it into. */
    max-width: 50px;
    min-width: 22px;
    height: 3px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.28);
    box-shadow: inset 0 1px 1px rgba(0, 0, 0, 0.3);
    overflow: hidden;
    transition: height 160ms var(--ease);
  }

  .volume:hover .track {
    height: 4px;
  }

  .fill {
    position: absolute;
    inset: 0;
    transform-origin: left center;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--accent) 55%, white),
      var(--accent)
    );
    /* Short and eased: the wheel arrives in bursts, and easing each landing
       makes a fast spin read as one continuous sweep rather than a stutter. */
    transition: transform 140ms var(--ease);
  }

  .muted .fill {
    background: var(--ink-faint);
  }

  /* Collapsed to zero width by default so it takes no space at all, rather than
     being merely transparent and still pushing the layout around. */
  .pct {
    font-size: 9.5px;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    color: var(--ink-dim);
    flex: none;
    width: 0;
    opacity: 0;
    overflow: hidden;
    text-align: right;
    transition:
      width 180ms var(--ease),
      opacity 160ms var(--ease);
  }

  .volume:hover .pct {
    width: 20px;
    opacity: 1;
  }

  .hint {
    position: absolute;
    bottom: calc(100% + 4px);
    right: 0;
    white-space: nowrap;
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.01em;
    color: var(--ink);
    background: rgba(10, 11, 16, 0.82);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    padding: 3px 7px;
    opacity: 0;
    transform: translateY(4px);
    pointer-events: none;
    transition:
      opacity 150ms var(--ease),
      transform 150ms var(--ease);
  }

  .hint.on {
    opacity: 1;
    transform: translateY(0);
  }
</style>
