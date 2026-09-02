<script lang="ts">
  import type { AppConfig } from "../lib/types";

  // Clawd, the pixel pet.
  //
  // Everything here is scoped under `.clawd-test-overlay`, renders in its own
  // fixed box, and touches nothing around it. Deleting this file and its two
  // lines in `Island.svelte` removes the feature completely.
  //
  // Drawn as pixels rather than shipped as a GIF: at this size a GIF is either
  // blurry or huge, cannot follow a configured colour, and animates on its own
  // clock instead of stopping when the music does.
  //
  // The character is built on a 16×16 grid with a dark outline, because at 20px
  // a shape without an outline dissolves into whatever glass is behind it —
  // which is exactly what the first version did.

  interface Props {
    playing: boolean;
    pet: AppConfig["pet"];
    /** Bumped when the track changes, so a random dance can pick again. */
    revision?: number;
    /** True for the copy that lives in the expanded panel. */
    expanded?: boolean;
  }

  const { playing, pet, revision = 0, expanded = false }: Props = $props();

  const DANCES = ["bob", "sway", "hop", "spin"] as const;

  // Local, not persisted: a toy that remembers a mood across restarts is a
  // setting, and the setting is in Settings.
  let awake = $state(true);
  /** Set for the length of the reaction whenever he is poked. */
  let poked = $state(false);
  let pokeTimer: ReturnType<typeof setTimeout> | undefined;

  /**
   * Which dance is running.
   *
   * `random` picks per *track*: a new dance every song is a small surprise, one
   * per frame is a seizure, and one per install is the same two poses forever.
   * Derived from the revision rather than `Math.random`, so a re-render that
   * has nothing to do with the music cannot change it mid-song.
   */
  const dance = $derived.by(() => {
    if (pet.dance !== "random") return pet.dance;
    return DANCES[Math.abs(Math.imul(revision, 2654435761)) % DANCES.length] ?? "bob";
  });

  // Asleep whenever the music is not. This is the state he is in most of the
  // time, so it had better not be a frozen sprite.
  const asleep = $derived(!playing);
  const active = $derived(awake && playing);

  const size = $derived(Math.min(32, Math.max(12, pet.size ?? 20)));

  /**
   * `auto` follows the album accent the capsule is already tinted with.
   *
   * Everything else in the capsule takes its colour from the artwork, so a
   * character who ignores it is the one element on screen that does not belong
   * to the track.
   */
  const shell = $derived(
    !pet.color || pet.color === "auto" ? "var(--accent, #d97757)" : pet.color,
  );

  function poke() {
    awake = !awake;
    poked = true;
    clearTimeout(pokeTimer);
    pokeTimer = setTimeout(() => (poked = false), 620);
  }

  $effect(() => () => clearTimeout(pokeTimer));
</script>

<button
  class="clawd-test-overlay {dance}"
  class:dancing={active}
  class:asleep
  class:poked
  class:expanded
  type="button"
  aria-pressed={awake}
  aria-label={asleep
    ? "Clawd is asleep. Click to wake him."
    : awake
      ? "Clawd is dancing. Click to settle him."
      : "Clawd is idle. Click to make him dance."}
  title={asleep ? "Clawd - asleep" : awake ? "Clawd - click to settle" : "Clawd - click to dance"}
  style:--clawd-size="{size}px"
  style:--clawd-shell={shell}
  onpointerdown={(e) => {
    // The capsule starts a window drag from pointerdown; without this, poking
    // Clawd would pick the whole capsule up instead.
    e.stopPropagation();
  }}
  onclick={(e) => {
    e.stopPropagation();
    poke();
  }}
>
  <svg viewBox="0 0 16 16" aria-hidden="true">
    <!-- Legs, behind the shell so their roots are hidden by it. -->
    <g class="legs">
      <rect class="ink" x="1" y="11" width="2" height="1" />
      <rect class="ink" x="0" y="12" width="1" height="2" />
      <rect class="ink" x="13" y="11" width="2" height="1" />
      <rect class="ink" x="15" y="12" width="1" height="2" />
      <rect class="ink" x="4" y="13" width="2" height="1" />
      <rect class="ink" x="10" y="13" width="2" height="1" />
    </g>

    <!-- Claws: outlined blocks with a notch cut out, which is what makes them
         read as pincers rather than mittens. -->
    <g class="claw left">
      <rect class="ink" x="0" y="6" width="4" height="4" />
      <rect class="shell" x="1" y="7" width="2" height="2" />
      <rect class="ink" x="3" y="7" width="1" height="1" />
      <rect class="glass" x="1" y="7" width="1" height="1" />
    </g>
    <g class="claw right">
      <rect class="ink" x="12" y="6" width="4" height="4" />
      <rect class="shell" x="13" y="7" width="2" height="2" />
      <rect class="ink" x="12" y="7" width="1" height="1" />
      <rect class="glass" x="14" y="7" width="1" height="1" />
    </g>

    <g class="body">
      <!-- Outline first, shell on top of it: one rect fewer than drawing the
           border as four strips, and it cannot leave a gap at a corner. -->
      <rect class="ink" x="3" y="4" width="10" height="9" rx="1" />
      <rect class="shell" x="4" y="5" width="8" height="7" />
      <rect class="shell" x="3" y="6" width="1" height="5" />
      <rect class="shell" x="12" y="6" width="1" height="5" />
      <!-- A lighter band across the top: at 20px this is the whole difference
           between a shell and a rectangle. -->
      <rect class="glass" x="5" y="5" width="6" height="1" />
      <rect class="glass" x="4" y="6" width="1" height="2" />

      <!-- Eyes on stalks, which is what makes a crab a crab. -->
      <g class="eyes">
        <rect class="ink" x="5" y="2" width="1" height="2" />
        <rect class="ink" x="10" y="2" width="1" height="2" />
        <rect class="ink" x="4" y="1" width="3" height="3" />
        <rect class="ink" x="9" y="1" width="3" height="3" />
        <rect class="white" x="5" y="2" width="1" height="1" />
        <rect class="white" x="10" y="2" width="1" height="1" />
      </g>

      <!-- Mouth, two pixels, slightly off-centre so he looks amused. -->
      <rect class="ink" x="6" y="9" width="1" height="1" />
      <rect class="ink" x="7" y="10" width="2" height="1" />
      <rect class="ink" x="9" y="9" width="1" height="1" />
    </g>

    <!-- Sleep: three Zs rising, rendered only while he is actually asleep. An
         element animating behind a dancing crab is wasted work. -->
    {#if asleep}
      <g class="zzz" aria-hidden="true">
        <text x="11" y="3" class="z" style="--d: 0s">z</text>
        <text x="13" y="1" class="z" style="--d: 0.9s">z</text>
        <text x="12" y="-1" class="z" style="--d: 1.8s">z</text>
      </g>
    {/if}

    <!-- The answer to being poked: a ring and four sparks, once per press. -->
    {#if poked}
      <g class="spark" aria-hidden="true">
        <circle cx="8" cy="8" r="7" class="ring" />
        <rect class="glass" x="1" y="2" width="1" height="1" />
        <rect class="glass" x="14" y="3" width="1" height="1" />
        <rect class="glass" x="2" y="13" width="1" height="1" />
        <rect class="glass" x="13" y="12" width="1" height="1" />
      </g>
    {/if}

    {#if pet.hat === "cap"}
      <g class="hat">
        <rect class="ink" x="4" y="0" width="8" height="1" />
        <rect class="accent" x="4" y="0" width="8" height="1" />
        <rect class="accent" x="11" y="1" width="3" height="1" />
      </g>
    {:else if pet.hat === "crown"}
      <g class="hat">
        <rect class="gold" x="4" y="0" width="1" height="2" />
        <rect class="gold" x="7" y="0" width="1" height="2" />
        <rect class="gold" x="10" y="0" width="1" height="2" />
        <rect class="gold" x="4" y="1" width="7" height="1" />
      </g>
    {:else if pet.hat === "headphones"}
      <g class="hat">
        <rect class="ink" x="4" y="0" width="7" height="1" />
        <rect class="accent" x="2" y="1" width="2" height="3" />
        <rect class="accent" x="11" y="1" width="2" height="3" />
      </g>
    {:else if pet.hat === "antenna"}
      <g class="hat">
        <rect class="ink" x="7" y="0" width="1" height="2" />
        <rect class="accent" x="6" y="0" width="1" height="1" />
        <rect class="accent" x="8" y="0" width="1" height="1" />
      </g>
    {/if}
  </svg>
</button>

<style>
  .clawd-test-overlay {
    flex: none;
    width: var(--clawd-size, 20px);
    height: var(--clawd-size, 20px);
    padding: 0;
    margin: 0 3px;
    border: 0;
    background: none;
    border-radius: 5px;
    display: grid;
    place-items: center;
    cursor: pointer;
    pointer-events: auto;
    /* Arrives with a pop the first time he is ever shown — see `.revealing`
       on the wrapper in Island.svelte. */
    animation: clawd-arrive 700ms cubic-bezier(0.34, 1.56, 0.64, 1) both;
    transition: box-shadow 140ms ease;
  }

  @keyframes clawd-arrive {
    0% {
      opacity: 0;
      transform: scale(0.2) rotate(-40deg);
    }
    60% {
      opacity: 1;
      transform: scale(1.25) rotate(8deg);
    }
    100% {
      opacity: 1;
      transform: scale(1) rotate(0);
    }
  }

  /* The footprint, on demand. Semi-transparent, so it shows the box without
     hiding what is in it. */
  .clawd-test-overlay:hover,
  .clawd-test-overlay:focus-visible {
    box-shadow:
      inset 0 0 0 1px rgba(255, 255, 255, 0.45),
      0 0 0 1px rgba(255, 255, 255, 0.12);
    outline: none;
  }

  .clawd-test-overlay svg {
    width: 100%;
    height: 100%;
    /* The whole point is that the pixels stay pixels. */
    shape-rendering: crispEdges;
    overflow: visible;
  }

  /* One configured colour, four derived tones. `color-mix` keeps the palette
     coherent for any hue the user picks, including the ones that would look
     wrong with hand-written shades. */
  .clawd-test-overlay .shell {
    fill: var(--clawd-shell);
  }
  .clawd-test-overlay .glass {
    fill: color-mix(in srgb, var(--clawd-shell) 55%, white);
  }
  .clawd-test-overlay .ink {
    fill: color-mix(in srgb, var(--clawd-shell) 45%, #1a0d08);
  }
  .clawd-test-overlay .white {
    fill: #fdf6f2;
  }
  .clawd-test-overlay .accent {
    fill: color-mix(in srgb, var(--clawd-shell) 30%, #2b3a67);
  }
  .clawd-test-overlay .gold {
    fill: #f2c14e;
  }

  /* --- the dances ---
     Four of them, each a pair of poses rather than a tween: `steps(2)` is what
     keeps a pixel character from sliding around like a vector one. All are
     paused unless `.dancing`, so a still Clawd costs the compositor nothing. */
  .clawd-test-overlay .body,
  .clawd-test-overlay .claw,
  .clawd-test-overlay .legs,
  .clawd-test-overlay svg {
    animation-play-state: paused;
    transform-box: fill-box;
    transform-origin: center;
  }

  .clawd-test-overlay.dancing .body,
  .clawd-test-overlay.dancing .claw,
  .clawd-test-overlay.dancing .legs,
  .clawd-test-overlay.dancing svg {
    animation-play-state: running;
  }

  /* bob — up and down, claws counter-swinging. */
  .clawd-test-overlay.bob .body {
    animation: clawd-bob 620ms steps(2, jump-none) infinite alternate;
  }
  .clawd-test-overlay.bob .claw.left {
    animation: clawd-claw-a 620ms steps(2, jump-none) infinite alternate;
  }
  .clawd-test-overlay.bob .claw.right {
    animation: clawd-claw-b 620ms steps(2, jump-none) infinite alternate;
  }
  .clawd-test-overlay.bob .legs {
    animation: clawd-shuffle 620ms steps(2, jump-none) infinite alternate;
  }

  /* sway — leans from the feet, like something keeping time. */
  .clawd-test-overlay.sway svg {
    animation: clawd-sway 900ms steps(2, jump-none) infinite alternate;
    transform-origin: bottom center;
  }
  .clawd-test-overlay.sway .claw.left {
    animation: clawd-claw-a 900ms steps(2, jump-none) infinite alternate;
  }
  .clawd-test-overlay.sway .claw.right {
    animation: clawd-claw-b 900ms steps(2, jump-none) infinite alternate;
  }

  /* hop — leaves the ground, and squashes on the landing. */
  .clawd-test-overlay.hop svg {
    animation: clawd-hop 700ms steps(3, jump-none) infinite;
    transform-origin: bottom center;
  }
  .clawd-test-overlay.hop .claw {
    animation: clawd-claw-up 700ms steps(2, jump-none) infinite alternate;
  }

  /* spin — four steps, so it turns rather than smears. */
  .clawd-test-overlay.spin svg {
    animation: clawd-spin 1.4s steps(4, jump-none) infinite;
  }

  @keyframes clawd-bob {
    from {
      transform: translateY(0.6px);
    }
    to {
      transform: translateY(-1.4px);
    }
  }
  @keyframes clawd-claw-a {
    from {
      transform: translateY(-1px) rotate(-14deg);
    }
    to {
      transform: translateY(1px) rotate(10deg);
    }
  }
  @keyframes clawd-claw-b {
    from {
      transform: translateY(1px) rotate(-10deg);
    }
    to {
      transform: translateY(-1px) rotate(14deg);
    }
  }
  @keyframes clawd-claw-up {
    from {
      transform: translateY(0.5px);
    }
    to {
      transform: translateY(-1.5px);
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
  @keyframes clawd-sway {
    from {
      transform: rotate(-9deg);
    }
    to {
      transform: rotate(9deg);
    }
  }
  @keyframes clawd-hop {
    0% {
      transform: translateY(0) scaleY(0.88) scaleX(1.1);
    }
    45% {
      transform: translateY(-3px) scaleY(1.06) scaleX(0.96);
    }
    100% {
      transform: translateY(0) scaleY(1) scaleX(1);
    }
  }
  @keyframes clawd-spin {
    to {
      transform: rotateY(360deg);
    }
  }

  /* --- asleep ---
     Where he spends most of his life, so it is a state rather than a stop: he
     sinks, breathes slowly, shuts his eyes, and lets out the occasional z. */
  .clawd-test-overlay.asleep .body {
    animation: clawd-sleep 3.4s ease-in-out infinite alternate;
    animation-play-state: running;
  }
  .clawd-test-overlay.asleep .eyes {
    transform: translateY(1px) scaleY(0.35);
    transform-origin: bottom center;
  }
  .clawd-test-overlay.asleep .claw,
  .clawd-test-overlay.asleep .legs {
    animation: none;
    transform: translateY(1px);
  }

  @keyframes clawd-sleep {
    from {
      transform: translateY(0.5px) scaleY(0.97);
    }
    to {
      transform: translateY(1.5px) scaleY(0.93);
    }
  }

  .clawd-test-overlay .z {
    font-family: "Segoe UI", system-ui, sans-serif;
    font-size: 4px;
    font-weight: 700;
    fill: color-mix(in srgb, var(--clawd-shell) 40%, white);
    animation: clawd-z 2.7s linear infinite;
    animation-delay: var(--d);
    opacity: 0;
  }

  @keyframes clawd-z {
    0% {
      opacity: 0;
      transform: translate(0, 2px) scale(0.6);
    }
    25% {
      opacity: 0.9;
    }
    100% {
      opacity: 0;
      transform: translate(3px, -6px) scale(1.1);
    }
  }

  /* --- poked ---
     One burst on every press, whichever state he was in. The whole point of a
     pet is that it answers. */
  .clawd-test-overlay.poked svg {
    animation: clawd-startle 620ms cubic-bezier(0.34, 1.56, 0.64, 1);
    animation-play-state: running;
  }

  @keyframes clawd-startle {
    0% {
      transform: translateY(0) scale(1);
    }
    30% {
      transform: translateY(-3.5px) scale(1.18) rotate(-8deg);
    }
    60% {
      transform: translateY(0) scale(0.94) rotate(4deg);
    }
    100% {
      transform: translateY(0) scale(1) rotate(0);
    }
  }

  .clawd-test-overlay .ring {
    fill: none;
    stroke: color-mix(in srgb, var(--clawd-shell) 50%, white);
    stroke-width: 0.6;
    transform-origin: center;
    animation: clawd-ring 620ms ease-out both;
  }

  @keyframes clawd-ring {
    0% {
      opacity: 0.9;
      transform: scale(0.5);
    }
    100% {
      opacity: 0;
      transform: scale(1.6);
    }
  }

  .clawd-test-overlay .spark rect {
    transform-origin: center;
    animation: clawd-spark 620ms ease-out both;
  }

  @keyframes clawd-spark {
    0% {
      opacity: 0;
      transform: scale(0.4);
    }
    40% {
      opacity: 1;
    }
    100% {
      opacity: 0;
      transform: scale(1.5);
    }
  }

  /* Idle-but-awake is not frozen either: he keeps breathing, so a settled pet
     still looks alive rather than switched off. */
  .clawd-test-overlay:not(.dancing):not(.asleep) .body {
    animation: clawd-breathe 2.6s ease-in-out infinite alternate;
    animation-play-state: running;
  }

  @keyframes clawd-breathe {
    to {
      transform: translateY(-0.5px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .clawd-test-overlay,
    .clawd-test-overlay svg,
    .clawd-test-overlay .body,
    .clawd-test-overlay .claw,
    .clawd-test-overlay .legs {
      animation: none !important;
    }
  }
</style>
