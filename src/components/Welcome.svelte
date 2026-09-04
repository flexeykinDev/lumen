<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { host } from "../lib/bridge";
  import type { AppConfig } from "../lib/types";
  import { t } from "../lib/i18n";

  // First run, once.
  //
  // Most of what Lumen does is invisible until someone happens to scroll over
  // the taskbar. A settings window full of switches does not fix that — it
  // answers questions nobody has thought to ask yet. So this shows the features
  // that change how the machine behaves, one at a time, and asks about each.
  //
  // Every step is answerable with one click, none of them is required, and it
  // never appears again. What it must not become is a wall someone has to fight
  // through to reach their music: Skip is always right there.

  interface Props {
    cfg: AppConfig;
    /** Applies a change and persists it, same as the settings panel. */
    patch: (change: (c: AppConfig) => void) => void;
    /** Marks the tour finished and hands over to the settings panel. */
    finish: () => void;
  }

  const { cfg, patch, finish }: Props = $props();

  /** Which little animation plays above the text. */
  type Art =
    | "wheel"
    | "close"
    | "dock"
    | "lyrics"
    | "boost"
    | "startup"
    | "games"
    | "shortcut"
    | "done";

  interface Step {
    art: Art;
    title: string;
    body: string;
    /** What the answer sets. Absent on the closing step. */
    apply?: (c: AppConfig, yes: boolean) => void;
    /** Whether it is currently on, to preselect the answer. */
    on?: () => boolean;
    /** Shown under the buttons when the honest answer needs a caveat. */
    caveat?: string;
  }

  const STEPS: Step[] = [
    {
      art: "wheel",
      title: "Volume, from the taskbar",
      body: "Scroll anywhere over the taskbar to change volume. Over an app's button it moves that app alone — which is the one that reaches a stream or a recording, because the master slider is applied after your audio has already been captured.",
      apply: (c, yes) => {
        c.mouse.taskbarWheelVolume = yes;
        c.mouse.taskbarWheelPerApp = yes;
      },
      on: () => cfg.mouse.taskbarWheelVolume,
    },
    {
      art: "close",
      title: "Close an app from the taskbar",
      body: "Middle-click an app's taskbar button to close it, without finding its window first. It sends the same request the X button does, so nothing is forced and unsaved work still prompts.",
      apply: (c, yes) => (c.mouse.taskbarCloseButton = yes ? "middle" : "none"),
      on: () => cfg.mouse.taskbarCloseButton !== "none",
    },
    {
      art: "dock",
      title: "Put the capsule where you want it",
      body: "It sits above the taskbar by default. Drag it anywhere on screen — near a corner it snaps to that corner and stays there; dropped in open space it stays exactly where you let go. Middle-click hides it until the next track.",
      apply: (c, yes) => (c.snapThreshold = yes ? 50 : 0),
      on: () => cfg.snapThreshold > 0,
    },
    {
      art: "lyrics",
      title: "Lyrics",
      body: "Synced lyrics under the track, following the music line by line.",
      caveat:
        "This is the only feature that uses the network: it sends the artist, title and album of what you play to lrclib.net.",
      apply: (c, yes) => (c.lyrics.enabled = yes),
      on: () => cfg.lyrics.enabled,
    },
    {
      art: "boost",
      title: "Louder than 100%",
      body: "Windows caps every volume control at 100%. Lumen can go past it, and add bass, by processing the sound itself.",
      caveat:
        "It captures the playing app, turns it right down, and plays a boosted copy instead — about 30 ms of delay and some CPU while it runs. Easy to turn on later in Audio.",
      apply: (c, yes) => (c.boost.enabled = yes),
      on: () => cfg.boost.enabled,
    },
    {
      art: "games",
      title: "Over games, or out of the way",
      body: "The capsule sits above every other window, games included, which is how you can see a track change without leaving a match. If it ever lands somewhere awkward it can stand down for full-screen games instead — and either way, binding the hide/show hotkey in Settings gives you a way to make it vanish and come back.",
      apply: (c, yes) => (c.onTop = yes ? "always" : "games"),
      on: () => cfg.onTop === "always",
    },
    {
      art: "shortcut",
      title: "Somewhere to find it again",
      body: "Lumen is a single portable file with no installer, which means it lives wherever you put it. A shortcut on the desktop and in the Start menu is how you get it back after the download folder is tidied.",
      // The only step that writes nothing to the config: shortcuts are files
      // on disk, and the effect below is what puts them there.
      apply: (_c, yes) => void (shortcutsWanted = yes),
      on: () => shortcutsWanted,
    },
    {
      art: "startup",
      title: "Start with Windows",
      body: "Lumen sits in the tray using no CPU when nothing is playing. Starting it with Windows means it is simply always there.",
      apply: (c, yes) => (c.startWithWindows = yes),
      on: () => cfg.startWithWindows,
    },
    {
      art: "done",
      title: "That's everything",
      body: "The capsule appears above the taskbar when something plays. Middle-click it to hide it, drag it anywhere, and everything here — plus hotkeys, Discord and the rest — is in Settings whenever you want it.",
    },
  ];

  // Answered during the tour, acted on when it moves to the next step: writing
  // two `.lnk` files is a filesystem change, and doing it as the answer is
  // clicked keeps the tour's promise literal.
  let shortcutsWanted = $state(false);
  $effect(() => {
    void host.shortcutSet("desktop", shortcutsWanted).catch(() => {});
    void host.shortcutSet("start", shortcutsWanted).catch(() => {});
  });

  let index = $state(0);
  const step = $derived(STEPS[index]);
  const last = $derived(index === STEPS.length - 1);

  function answer(yes: boolean) {
    const apply = step?.apply;
    if (apply) patch((c) => apply(c, yes));
    next();
  }

  function next() {
    if (last) finish();
    else index += 1;
  }
</script>

<div class="tour">
  <header data-tauri-drag-region>
    <span class="brand" data-tauri-drag-region><span class="dot"></span>Lumen</span>
    <!-- Leaving early is a legitimate answer, and hiding it would make this a
         wall rather than an introduction. Nothing is applied on the way out. -->
    <button class="skip" onclick={finish}>{t("Skip the tour")}</button>
  </header>

  {#if step}
    {#key index}
      <main>
        <!-- Drawn rather than recorded. A GIF of this would be a megabyte, soft
             on a 4K screen, wrong in the other theme, and stuck at whatever the
             capture's frame rate was; this is a few hundred bytes, sharp at any
             scale, and follows the accent colour. -->
        <svg class="art" viewBox="0 0 160 90" aria-hidden="true">
          {#if step.art === "wheel"}
            <!-- A pointer over a taskbar button, and the level answering it. -->
            <rect class="bar" x="10" y="66" width="140" height="16" rx="4" />
            <rect class="btn" x="46" y="69" width="22" height="10" rx="2.5" />
            <rect class="btn dim" x="72" y="69" width="22" height="10" rx="2.5" />
            <g class="cursor">
              <path d="M57 44l0 12" class="wheel-line" />
              <path d="M53 48l4-4 4 4" class="wheel-up" />
              <path d="M53 52l4 4 4-4" class="wheel-down" />
            </g>
            <g class="meter">
              <rect x="104" y="24" width="6" height="30" rx="3" class="track" />
              <rect x="104" y="24" width="6" height="30" rx="3" class="fill" />
            </g>
          {:else if step.art === "close"}
            <!-- Middle-click, and the window it dismisses. -->
            <g class="window">
              <rect x="46" y="14" width="68" height="44" rx="5" class="frame" />
              <path d="M46 26h68" class="frame-line" />
              <path d="M96 18l0 0M104 18l0 0" class="frame-line" />
            </g>
            <rect class="bar" x="10" y="66" width="140" height="16" rx="4" />
            <rect class="btn click" x="70" y="69" width="22" height="10" rx="2.5" />
          {:else if step.art === "dock"}
            <!-- A screen, and the capsule crossing it into a corner. -->
            <rect x="14" y="8" width="132" height="74" rx="6" class="frame" />
            <rect class="bar" x="14" y="70" width="132" height="12" rx="0" />
            <g class="capsule">
              <rect x="0" y="0" width="34" height="11" rx="5.5" />
              <circle cx="7" cy="5.5" r="3" class="capsule-dot" />
            </g>
            <rect x="106" y="16" width="34" height="11" rx="5.5" class="ghost" />
          {:else if step.art === "games"}
            <!-- A game filling the screen, with the capsule riding above it. -->
            <rect x="10" y="10" width="140" height="62" rx="4" class="frame" />
            <path d="M40 44h16M48 36v16" class="pad" />
            <circle cx="112" cy="40" r="4" class="pad-dot" />
            <circle cx="124" cy="48" r="4" class="pad-dot" />
            <g class="over">
              <rect x="46" y="60" width="68" height="16" rx="8" />
              <circle cx="56" cy="68" r="4" class="capsule-dot" />
            </g>
          {:else if step.art === "shortcut"}
            <!-- A desktop icon, arriving. -->
            <rect x="14" y="8" width="132" height="74" rx="6" class="frame" />
            <g class="icon">
              <rect x="66" y="28" width="28" height="28" rx="7" />
              <circle cx="80" cy="42" r="5" class="capsule-dot" />
            </g>
            <path d="M72 64h16" class="lyric future" />
          {:else if step.art === "lyrics"}
            <!-- One line filling in time with the music. -->
            <path d="M28 26h104" class="lyric past" />
            <g class="lyric-live">
              <path d="M34 46h92" class="lyric track" />
              <path d="M34 46h92" class="lyric fill" />
            </g>
            <path d="M42 66h76" class="lyric future" />
          {:else if step.art === "boost"}
            <!-- The same waveform, before and after. -->
            <g class="wave">
              {#each [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] as i (i)}
                <rect
                  class="stem"
                  x={20 + i * 11}
                  width="6"
                  rx="3"
                  style="--i: {i}; --h: {[9, 16, 26, 38, 30, 20, 34, 44, 28, 18, 12, 8][i]}"
                />
              {/each}
            </g>
          {:else if step.art === "startup"}
            <!-- A power mark, drawing itself on. -->
            <g class="power">
              <circle cx="80" cy="45" r="20" class="ring" />
              <path d="M80 30v14" class="stem-line" />
            </g>
            <g class="sparks">
              <circle cx="52" cy="24" r="1.6" style="--d: 0s" />
              <circle cx="112" cy="30" r="1.6" style="--d: 0.4s" />
              <circle cx="104" cy="70" r="1.6" style="--d: 0.8s" />
            </g>
          {:else}
            <path d="M58 46l14 14 30-32" class="check" />
          {/if}
        </svg>
        <h1>{t(step.title)}</h1>
        <p class="body">{t(step.body)}</p>
        {#if step.caveat}<p class="caveat">{t(step.caveat)}</p>{/if}

        <div class="actions">
          {#if step.apply}
            <button class="no" onclick={() => answer(false)}>{t("Not now")}</button>
            <button class="yes" onclick={() => answer(true)}>
              {step.on?.() ? t("Keep it on") : t("Turn it on")}
            </button>
          {:else}
            <button class="yes wide" onclick={finish}>{t("Open settings")}</button>
            <button class="no" onclick={() => getCurrentWindow().close()}>{t("Done")}</button>
          {/if}
        </div>
      </main>
    {/key}
  {/if}

  <footer>
    {#each STEPS as _, i (i)}
      <span class="pip" class:on={i <= index}></span>
    {/each}
  </footer>
</div>

<style>
  .tour {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-radius: 12px;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }

  header {
    height: 44px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px 0 16px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 650;
  }

  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--settings-accent);
    box-shadow: 0 0 10px var(--settings-accent);
  }

  .skip {
    font-size: 12px;
    color: var(--ink-faint);
    padding: 6px 10px;
    border-radius: 7px;
  }

  .skip:hover {
    color: var(--ink);
    background: rgba(255, 255, 255, 0.06);
  }

  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 0 12% 20px;
    /* Each step arrives rather than snapping: the movement is what tells you
       something changed on a screen whose layout stays put. */
    animation: arrive 260ms var(--ease) both;
  }

  @keyframes arrive {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }

  .art {
    width: 232px;
    height: 130px;
    margin-bottom: 20px;
    overflow: visible;
  }

  /* Shared vocabulary for the scenes: outlines in the accent, surfaces in a
     faint white, so every step looks like the same drawing. */
  .art .bar,
  .art .track,
  .art .frame,
  .art .ring {
    fill: rgba(255, 255, 255, 0.05);
    stroke: var(--line-strong);
    stroke-width: 1.5;
  }

  .art .btn {
    fill: color-mix(in srgb, var(--settings-accent) 55%, transparent);
  }

  .art .btn.dim {
    fill: rgba(255, 255, 255, 0.14);
  }

  .art path {
    fill: none;
    stroke: var(--settings-accent);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Taskbar wheel: the pointer nudges, the arrows pulse, the level rises. */
  .cursor {
    animation: nudge 2.6s ease-in-out infinite;
  }
  .wheel-up {
    animation: flick 2.6s ease-in-out infinite;
  }
  .wheel-down {
    opacity: 0.25;
  }
  .meter .fill {
    fill: var(--settings-accent);
    stroke: none;
    transform-box: fill-box;
    transform-origin: bottom;
    animation: raise 2.6s ease-in-out infinite;
  }
  @keyframes nudge {
    0%,
    60%,
    100% {
      transform: translateY(0);
    }
    30% {
      transform: translateY(-4px);
    }
  }
  @keyframes flick {
    0%,
    100% {
      opacity: 0.3;
    }
    25% {
      opacity: 1;
    }
  }
  @keyframes raise {
    0%,
    100% {
      transform: scaleY(0.35);
    }
    45%,
    70% {
      transform: scaleY(1);
    }
  }

  /* Close: the button is pressed, the window goes. */
  .window {
    transform-box: fill-box;
    transform-origin: center;
    animation: dismiss 3s ease-in-out infinite;
  }
  .frame-line {
    stroke: var(--line-strong);
  }
  .btn.click {
    animation: press 3s ease-in-out infinite;
  }
  @keyframes press {
    0%,
    35%,
    100% {
      fill: color-mix(in srgb, var(--settings-accent) 55%, transparent);
    }
    45% {
      fill: #fff;
    }
  }
  @keyframes dismiss {
    0%,
    45% {
      opacity: 1;
      transform: scale(1);
    }
    62%,
    88% {
      opacity: 0;
      transform: scale(0.9);
    }
    100% {
      opacity: 1;
      transform: scale(1);
    }
  }

  /* Docking: the capsule is dragged up to a corner, where it snaps in. The
     ghost is the target, so the snap reads as arrival rather than drift. */
  .capsule rect {
    fill: color-mix(in srgb, var(--settings-accent) 70%, transparent);
  }
  .capsule-dot {
    fill: #fff;
    opacity: 0.9;
  }
  .capsule {
    animation: dock 3.4s cubic-bezier(0.5, 0, 0.2, 1) infinite;
  }
  .ghost {
    fill: none;
    stroke: var(--settings-accent);
    stroke-width: 1.5;
    stroke-dasharray: 4 4;
    opacity: 0.5;
  }
  @keyframes dock {
    0%,
    8% {
      transform: translate(56px, 52px);
    }
    /* The last stretch is the snap: it covers little distance and ends abruptly. */
    62% {
      transform: translate(98px, 22px);
    }
    72%,
    100% {
      transform: translate(106px, 16px);
    }
  }

  /* Games: the capsule rides above a full-screen window. */
  .pad,
  .art .pad {
    stroke: rgba(255, 255, 255, 0.35);
    stroke-width: 2.5;
  }
  .pad-dot {
    fill: rgba(255, 255, 255, 0.35);
  }
  .over rect {
    fill: color-mix(in srgb, var(--settings-accent) 70%, transparent);
  }
  .over {
    animation: clawd-arrive 2.4s ease-in-out infinite alternate;
  }

  /* Shortcut: the icon lands on the desktop. */
  .icon rect {
    fill: color-mix(in srgb, var(--settings-accent) 70%, transparent);
  }
  .icon {
    transform-box: fill-box;
    transform-origin: center;
    animation: land 2.6s ease-in-out infinite alternate;
  }
  @keyframes land {
    from {
      transform: translateY(-4px) scale(0.94);
    }
    to {
      transform: translateY(0) scale(1);
    }
  }

  /* Lyrics: the live line fills left to right, like the capsule's karaoke. */
  .lyric {
    stroke-width: 6;
    stroke-linecap: round;
  }
  .lyric.past,
  .lyric.future {
    stroke: rgba(255, 255, 255, 0.14);
  }
  .lyric.track {
    stroke: rgba(255, 255, 255, 0.22);
  }
  .lyric.fill {
    stroke: var(--settings-accent);
    stroke-dasharray: 92;
    animation: sing 3.2s linear infinite;
  }
  @keyframes sing {
    from {
      stroke-dashoffset: 92;
    }
    to {
      stroke-dashoffset: 0;
    }
  }

  /* Boost: every bar grows, the low ones furthest — a shelf, drawn. */
  .stem {
    fill: var(--settings-accent);
    y: 58px;
    height: calc(var(--h) * 1px);
    transform-box: fill-box;
    transform-origin: bottom;
    animation: swell 2.4s ease-in-out infinite;
    animation-delay: calc(var(--i) * -0.12s);
  }
  @keyframes swell {
    0%,
    100% {
      transform: scaleY(0.55) translateY(0);
      opacity: 0.55;
    }
    50% {
      transform: scaleY(1.35);
      opacity: 1;
    }
  }

  /* Startup: the mark draws itself, then a few sparks. */
  .ring {
    stroke: var(--settings-accent);
    stroke-dasharray: 100 26;
    transform-box: fill-box;
    transform-origin: center;
    animation: spin 4s linear infinite;
  }
  .stem-line {
    stroke-width: 3;
  }
  .sparks circle {
    fill: var(--settings-accent);
    opacity: 0;
    animation: twinkle 2.4s ease-in-out infinite;
    animation-delay: var(--d);
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes twinkle {
    0%,
    100% {
      opacity: 0;
      transform: scale(0.6);
    }
    40% {
      opacity: 1;
      transform: scale(1);
    }
  }

  /* Done: the tick is drawn, once. */
  .check {
    stroke-width: 4;
    stroke-dasharray: 70;
    animation: draw 700ms var(--ease-snap) 120ms both;
  }
  @keyframes draw {
    from {
      stroke-dashoffset: 70;
    }
    to {
      stroke-dashoffset: 0;
    }
  }

  h1 {
    margin: 0 0 12px;
    font-size: 25px;
    font-weight: 700;
    letter-spacing: -0.025em;
  }

  .body {
    margin: 0;
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--ink-dim);
    max-width: 54ch;
  }

  /* The cost of a feature belongs next to the button that turns it on, not in
     a document nobody opens. */
  .caveat {
    margin: 14px 0 0;
    padding: 10px 14px;
    border-radius: var(--radius);
    background: rgba(255, 190, 100, 0.08);
    box-shadow: inset 0 0 0 1px rgba(255, 190, 100, 0.2);
    font-size: 12px;
    line-height: 1.55;
    color: #ffe0b8;
    max-width: 52ch;
  }

  .actions {
    display: flex;
    gap: 10px;
    margin-top: 28px;
  }

  .actions button {
    min-width: 128px;
    height: 38px;
    padding: 0 20px;
    border-radius: 9px;
    font-size: 13px;
    font-weight: 600;
    transition:
      background 140ms var(--ease),
      transform 140ms var(--ease-snap);
  }

  .actions button:active {
    transform: scale(0.97);
  }

  .yes {
    background: var(--settings-accent);
    color: #fff;
  }

  .yes:hover {
    background: color-mix(in srgb, var(--settings-accent) 85%, white);
  }

  .no {
    background: rgba(255, 255, 255, 0.06);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--ink-dim);
  }

  .no:hover {
    color: var(--ink);
    background: rgba(255, 255, 255, 0.1);
  }

  footer {
    flex: none;
    display: flex;
    justify-content: center;
    gap: 6px;
    padding: 0 0 22px;
  }

  .pip {
    width: 22px;
    height: 3px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.12);
    transition: background 200ms var(--ease);
  }

  .pip.on {
    background: var(--settings-accent);
  }

  /* Every scene loops forever, which is exactly what this setting exists to
     stop. The drawings still read perfectly standing still. */
  @media (prefers-reduced-motion: reduce) {
    main,
    .art *,
    .art {
      animation: none !important;
    }
  }
</style>
