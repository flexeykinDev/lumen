<script lang="ts">
  // Clawd, the pixel pet.
  //
  // An easter egg, and deliberately built like one: every style in this file is
  // scoped under `.clawd-test-overlay`, it renders in its own fixed-size box,
  // and it touches nothing around it. Deleting this component and its one line
  // in `Island.svelte` removes it completely.
  //
  // Drawn as pixels rather than shipped as a GIF. At 20px on a 4K screen a GIF
  // is either blurry or huge, it cannot follow the capsule's accent, and it
  // animates on its own clock; this is ~40 rects that stay crisp at any DPI and
  // freeze exactly when the music does.
  //
  // The motion is `steps()` transforms, not tweens: a pixel character that
  // slides smoothly stops looking like pixel art. Two poses per cycle, snapped.

  interface Props {
    /** Dance only while the music does — see the animation gate below. */
    playing: boolean;
  }

  const { playing }: Props = $props();

  // Local, not persisted: this is a toy, and a toy that remembers a mood across
  // restarts is a setting. The switch that *is* remembered lives in Settings.
  let dancing = $state(true);

  const active = $derived(dancing && playing);
</script>

<button
  class="clawd-test-overlay"
  class:dancing={active}
  type="button"
  aria-pressed={dancing}
  aria-label={dancing ? "Clawd is dancing. Click to settle him down." : "Clawd is idle. Click to make him dance."}
  title={dancing ? "Clawd — click to idle" : "Clawd — click to dance"}
  onpointerdown={(e) => {
    // The capsule starts a window drag from pointerdown. Without this, every
    // poke at Clawd would pick the whole capsule up instead.
    e.stopPropagation();
  }}
  onclick={(e) => {
    e.stopPropagation();
    dancing = !dancing;
  }}
>
  <svg viewBox="0 0 16 16" aria-hidden="true">
    <!-- Legs first, so the body paints over their roots. -->
    <g class="legs">
      <rect class="dark" x="2" y="12" width="1" height="2" />
      <rect class="dark" x="4" y="13" width="1" height="1" />
      <rect class="dark" x="11" y="13" width="1" height="1" />
      <rect class="dark" x="13" y="12" width="1" height="2" />
    </g>

    <g class="claw left">
      <rect class="mid" x="0" y="7" width="2" height="2" />
      <rect class="mid" x="1" y="9" width="2" height="1" />
      <rect class="dark" x="0" y="6" width="1" height="1" />
    </g>
    <g class="claw right">
      <rect class="mid" x="14" y="7" width="2" height="2" />
      <rect class="mid" x="13" y="9" width="2" height="1" />
      <rect class="dark" x="15" y="6" width="1" height="1" />
    </g>

    <g class="body">
      <!-- Shell: wider in the middle, tapering top and bottom. -->
      <rect class="base" x="5" y="4" width="6" height="1" />
      <rect class="base" x="4" y="5" width="8" height="7" />
      <rect class="base" x="3" y="7" width="1" height="4" />
      <rect class="base" x="12" y="7" width="1" height="4" />
      <!-- A lighter band across the top reads as a highlight at this size. -->
      <rect class="light" x="5" y="5" width="6" height="1" />
      <rect class="light" x="4" y="6" width="2" height="1" />

      <g class="face">
        <rect class="white" x="5" y="7" width="2" height="2" />
        <rect class="white" x="9" y="7" width="2" height="2" />
        <rect class="pupil" x="6" y="8" width="1" height="1" />
        <rect class="pupil" x="10" y="8" width="1" height="1" />
        <!-- Mouth: one pixel wide, which is all a face this size needs. -->
        <rect class="dark" x="7" y="10" width="2" height="1" />
      </g>
    </g>
</svg>
</button>

<style>
  /* Everything below is scoped to this class on purpose. Nothing here inherits
     from, or leaks into, the capsule's own styles. */
  .clawd-test-overlay {
    flex: none;
    width: 20px;
    height: 20px;
    padding: 0;
    margin: 0 2px;
    border: 0;
    background: none;
    border-radius: 5px;
    display: grid;
    place-items: center;
    cursor: pointer;
    /* The capsule is click-through except where it opts in. */
    pointer-events: auto;
    -webkit-app-region: no-drag;
    transition: box-shadow 140ms ease;
  }

  /* The footprint, on demand. Semi-transparent so it shows the box without
     hiding what is inside it. */
  .clawd-test-overlay:hover,
  .clawd-test-overlay:focus-visible {
    box-shadow:
      inset 0 0 0 1px rgba(255, 255, 255, 0.45),
      0 0 0 1px rgba(255, 255, 255, 0.12);
    outline: none;
  }

  .clawd-test-overlay svg {
    width: 20px;
    height: 20px;
    /* No smoothing: the whole point is that the pixels stay pixels. */
    shape-rendering: crispEdges;
    overflow: visible;
  }

  /* Claude's rust, at three depths. Fixed rather than tied to the album accent:
     Clawd is a character, and a character that changes colour with the artwork
     stops being one. */
  .clawd-test-overlay .base {
    fill: #d97757;
  }
  .clawd-test-overlay .light {
    fill: #e8907a;
  }
  .clawd-test-overlay .mid {
    fill: #c96545;
  }
  .clawd-test-overlay .dark {
    fill: #8f4227;
  }
  .clawd-test-overlay .white {
    fill: #fdf6f2;
  }
  .clawd-test-overlay .pupil {
    fill: #2a1712;
  }

  /* --- the dance ---
     Three groups, three transforms, and `steps(2)` so each lands as a pose
     rather than sliding between them. Paused by default: the animations exist
     but are held still, which costs nothing and means starting the dance is a
     property change rather than a re-layout. */
  .clawd-test-overlay .body,
  .clawd-test-overlay .claw,
  .clawd-test-overlay .legs {
    animation-play-state: paused;
    transform-box: fill-box;
    transform-origin: center;
  }

  .clawd-test-overlay .body {
    animation: clawd-bob 620ms steps(2, jump-none) infinite alternate;
  }
  .clawd-test-overlay .claw.left {
    animation: clawd-claw-left 620ms steps(2, jump-none) infinite alternate;
    transform-origin: right center;
  }
  .clawd-test-overlay .claw.right {
    animation: clawd-claw-right 620ms steps(2, jump-none) infinite alternate;
    transform-origin: left center;
  }
  .clawd-test-overlay .legs {
    animation: clawd-shuffle 620ms steps(2, jump-none) infinite alternate;
  }

  .clawd-test-overlay.dancing .body,
  .clawd-test-overlay.dancing .claw,
  .clawd-test-overlay.dancing .legs {
    animation-play-state: running;
  }

  @keyframes clawd-bob {
    from {
      transform: translateY(0.6px);
    }
    to {
      transform: translateY(-1.2px);
    }
  }

  /* The claws work against the bob — up while the body is down — which is what
     makes two poses read as a dance instead of a wobble. */
  @keyframes clawd-claw-left {
    from {
      transform: translateY(-1px) rotate(-12deg);
    }
    to {
      transform: translateY(1px) rotate(10deg);
    }
  }

  @keyframes clawd-claw-right {
    from {
      transform: translateY(1px) rotate(-10deg);
    }
    to {
      transform: translateY(-1px) rotate(12deg);
    }
  }

  @keyframes clawd-shuffle {
    from {
      transform: translateX(-0.7px);
    }
    to {
      transform: translateX(0.7px);
    }
  }

  /* Idle is not frozen: he keeps breathing, slowly, so a paused pet still looks
     alive rather than switched off. */
  .clawd-test-overlay:not(.dancing) .body {
    animation: clawd-breathe 2.6s ease-in-out infinite alternate;
    animation-play-state: running;
  }

  @keyframes clawd-breathe {
    to {
      transform: translateY(-0.5px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .clawd-test-overlay .body,
    .clawd-test-overlay .claw,
    .clawd-test-overlay .legs {
      animation: none !important;
    }
  }
</style>
