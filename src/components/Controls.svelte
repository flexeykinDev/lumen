<script lang="ts">
  import { host, type TransportAction } from "../lib/bridge";
  import { island } from "../lib/state.svelte";

  // Optimism is deliberate: SMTC round-trips through the source app and can take
  // 200-400 ms to report the new state. Waiting for it makes the button feel
  // broken, so the glyph flips immediately and the host corrects it after.
  let optimistic = $state<boolean | null>(null);
  let settle: ReturnType<typeof setTimeout> | undefined;

  const playing = $derived(optimistic ?? island.playing);

  $effect(() => {
    const confirmed = island.playing;
    // Drop the prediction only once the host actually agrees with it. Clearing
    // on any incoming event would undo the glyph on the timeline update that
    // routinely arrives *before* the source app reports its new play state.
    if (optimistic !== null && confirmed === optimistic) {
      clearTimeout(settle);
      optimistic = null;
    }
  });

  $effect(() => () => clearTimeout(settle));

  async function send(action: TransportAction) {
    if (action === "playPause") {
      optimistic = !playing;
      // A source can legitimately refuse the command. Give up predicting after
      // a beat rather than leaving the glyph permanently lying.
      clearTimeout(settle);
      settle = setTimeout(() => (optimistic = null), 1200);
    }
    try {
      await host.transport(action);
    } catch (e) {
      clearTimeout(settle);
      optimistic = null;
      console.warn(`transport ${action} failed`, e);
    }
  }
</script>

<div class="controls">
  <button type="button" class="skip" aria-label="Previous track" onclick={() => send("previous")}>
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M7.5 5.5v13M18.5 6.2 10 12l8.5 5.8z" />
    </svg>
  </button>

  <button
    type="button"
    class="primary"
    aria-label={playing ? "Pause" : "Play"}
    onclick={() => send("playPause")}
  >
    <!--
      Both glyphs are always mounted and cross-faded, rather than swapped with
      an {#if}. A swap is a hard cut with nothing to animate — which is exactly
      what made this button feel dead. Counter-rotating them through the change
      reads as one shape turning into the other.
    -->
    <span class="glyph" class:on={!playing} aria-hidden="true">
      <svg viewBox="0 0 24 24">
        <path d="M7.5 4.8 19.5 12 7.5 19.2z" />
      </svg>
    </span>
    <span class="glyph pause" class:on={playing} aria-hidden="true">
      <svg viewBox="0 0 24 24">
        <path d="M8.8 5.2h2.6v13.6H8.8zM12.6 5.2h2.6v13.6h-2.6z" />
      </svg>
    </span>
  </button>

  <button type="button" class="skip" aria-label="Next track" onclick={() => send("next")}>
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M16.5 5.5v13M5.5 6.2 14 12l-8.5 5.8z" />
    </svg>
  </button>
</div>

<style>
  .controls {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  button {
    display: grid;
    place-items: center;
    border-radius: 50%;
    transition:
      background-color 180ms var(--ease),
      color 180ms var(--ease),
      box-shadow 180ms var(--ease),
      transform 220ms var(--ease-snap);
  }

  /* Press feedback has to land inside the ~50 ms it takes to lift a finger or it
     reads as lag, so the press is near-instant and only the release is sprung. */
  button:active {
    transform: scale(0.86);
    transition-duration: 50ms;
  }

  button:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, white);
    outline-offset: 2px;
  }

  /* Sized against the 36px primary. Too small a skip button next to a filled
     accent circle reads as an afterthought rather than as a pair of siblings.
     Both skips are styled through this one class and never through positional
     selectors (:first-child / :last-child), so previous and next cannot drift
     apart — an earlier revision styled them by position, which is the usual way
     a pair like this ends up asymmetric. */
  .skip {
    width: 32px;
    height: 32px;
    color: rgba(255, 255, 255, 0.82);
    background: transparent;
  }

  .skip:hover,
  .skip:focus-visible {
    background: var(--well-hover);
    color: #fff;
    transform: scale(1.08);
  }

  /* The single strongest upgrade in the panel: a filled, accent-coloured,
     softly-glowing primary. A ring of identical grey circles has no focal
     point, which is why the old control row read as unfinished. */
  .primary {
    width: 36px;
    height: 36px;
    position: relative;
    color: var(--accent-fg);
    background: linear-gradient(
      160deg,
      color-mix(in srgb, var(--accent) 78%, white) 0%,
      var(--accent) 100%
    );
    box-shadow:
      0 2px 10px -2px color-mix(in srgb, var(--accent) 75%, transparent),
      inset 0 1px 0 rgba(255, 255, 255, 0.35);
  }

  .primary:hover {
    transform: scale(1.07);
    box-shadow:
      0 4px 16px -2px color-mix(in srgb, var(--accent) 85%, transparent),
      inset 0 1px 0 rgba(255, 255, 255, 0.45);
  }

  .glyph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    opacity: 0;
    transform: scale(0.5) rotate(-40deg);
    transition:
      opacity 200ms var(--ease),
      transform 300ms var(--ease-snap);
  }

  /* The pause glyph turns in from the other side, so play and pause sweep
     through each other rather than both spinning the same way. */
  .glyph.pause {
    transform: scale(0.5) rotate(40deg);
  }

  .glyph.on {
    opacity: 1;
    transform: scale(1) rotate(0deg);
  }

  svg {
    width: 100%;
    height: 100%;
    fill: currentColor;
    stroke: none;
  }

  .skip svg {
    width: 18px;
    height: 18px;
    filter: drop-shadow(0 1px 1.5px rgba(0, 0, 0, 0.4));
  }

  .glyph svg {
    width: 17px;
    height: 17px;
  }
</style>
