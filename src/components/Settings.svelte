<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { host } from "../lib/bridge";
  import { currentLanguage, setLanguage, t } from "../lib/i18n";
  import type { AppConfig, PresenceButton } from "../lib/types";
  import Welcome from "./Welcome.svelte";
  import Row from "./settings/Row.svelte";
  import Toggle from "./settings/Toggle.svelte";
  import Hotkey from "./settings/Hotkey.svelte";

  // Settings, in its own window.
  //
  // Every control writes straight through to the host and the file — there is no
  // Apply button and no draft state. A panel that can disagree with what the app
  // is doing is a second source of truth, and the "did that save?" question it
  // creates is worse than the round-trip it avoids.
  //
  // Settings whose subsystem is built once at startup carry a `restart` badge
  // rather than pretending to take effect immediately.

  type Tab =
    | "general"
    | "appearance"
    | "audio"
    | "hotkeys"
    | "discord"
    | "lyrics"
    | "advanced"
    | "about";

  const TABS: { id: Tab; label: string; icon: string }[] = [
    { id: "general", label: "General", icon: "M4 6h16M4 12h16M4 18h10" },
    {
      id: "appearance",
      label: "Appearance",
      icon: "M12 3a9 9 0 100 18 3 3 0 003-3v-1a2 2 0 012-2h1a3 3 0 003-3 9 9 0 00-9-9z",
    },
    { id: "audio", label: "Audio & mouse", icon: "M4 10v4h3l4 4V6L7 10H4zm12-2a5 5 0 010 8" },
    { id: "hotkeys", label: "Hotkeys", icon: "M4 7h16v10H4zM8 11h.01M12 11h.01M16 11h.01M8 14h8" },
    {
      id: "discord",
      label: "Discord",
      icon: "M8 6h8a4 4 0 014 4v4a4 4 0 01-4 4H8a4 4 0 01-4-4v-4a4 4 0 014-4zM9 12h.01M15 12h.01",
    },
    { id: "lyrics", label: "Lyrics", icon: "M5 5h14M5 10h14M5 15h9" },
    {
      id: "advanced",
      label: "Advanced",
      icon: "M12 8a4 4 0 100 8 4 4 0 000-8zM12 2v3M12 19v3M2 12h3M19 12h3",
    },
    { id: "about", label: "About", icon: "M12 8h.01M11 12h1v5h1M12 3a9 9 0 100 18 9 9 0 000-18z" },
  ];

  /** Players common enough to be worth listing before they are playing. */
  const KNOWN_SOURCES = [
    "Spotify",
    "Firefox",
    "Chrome",
    "Edge",
    "Yandex Music",
    "AIMP",
    "foobar2000",
    "VLC",
  ];

  let tab = $state<Tab>("general");
  let cfg = $state<AppConfig | null>(null);
  let info = $state<{ configPath: string | null; portable: boolean; version: string } | null>(null);
  let liveSources = $state<string[]>([]);
  let saved = $state(false);
  let savedTimer: ReturnType<typeof setTimeout> | undefined;

  /**
   * The language in force, as state rather than a module variable.
   *
   * `t()` reads a module-level setting, so on its own it would never re-run.
   * Reading this inside `tr()` makes every translated string in the template a
   * dependency of it, and `{#key}` on the panel body rebuilds the parts that go
   * through child components instead.
   */
  let lang = $state<"en" | "ru">("en");

  const tr = (text: string | undefined) => {
    void lang;
    return t(text);
  };

  onMount(async () => {
    cfg = await host.getConfig();
    applyLanguage(cfg.language);
    info = await host.runtimeInfo();
    // Whatever is publishing right now, so the player list is about this machine
    // rather than a guess at what people use.
    try {
      liveSources = (await host.sessions()).map((s) => s.source);
    } catch {
      liveSources = [];
    }
  });

  /**
   * Apply a change and persist it.
   *
   * The host echoes back what it stored and that is what we adopt, so the panel
   * shows what was actually saved rather than what was requested — the two
   * differ whenever the host clamps a value.
   */
  function applyLanguage(pref: AppConfig["language"]) {
    setLanguage(pref);
    lang = currentLanguage();
  }

  async function patch(change: (c: AppConfig) => void) {
    if (!cfg) return;
    const next = structuredClone($state.snapshot(cfg)) as AppConfig;
    change(next);
    cfg = next;
    try {
      cfg = await host.setConfig(next);
      saved = true;
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => (saved = false), 1400);
    } catch (e) {
      console.error("could not save settings", e);
    }
  }

  /** Every player worth offering a switch for, in a stable order. */
  const sourceList = $derived([
    ...new Set([...liveSources, ...(cfg?.discord.hiddenSources ?? []), ...KNOWN_SOURCES]),
  ]);

  const isPublished = (source: string) =>
    !(cfg?.discord.hiddenSources ?? []).some((h) => h.toLowerCase() === source.toLowerCase());

  /**
   * Hiding is what gets stored; showing is the absence of an entry.
   *
   * That way a player installed tomorrow is published without anyone having to
   * come back here and tick it.
   */
  function setPublished(source: string, on: boolean) {
    patch((c) => {
      const without = c.discord.hiddenSources.filter(
        (h) => h.toLowerCase() !== source.toLowerCase(),
      );
      c.discord.hiddenSources = on ? without : [...without, source];
    });
  }

  function setButton(index: number, change: (b: PresenceButton) => void) {
    patch((c) => {
      while (c.discord.buttons.length <= index) {
        c.discord.buttons.push({ enabled: false, label: "", url: "" });
      }
      const target = c.discord.buttons[index];
      if (target) change(target);
    });
  }

  /** The eight resize handles, as direction plus the class that places it. */
  const RESIZE_EDGES = [
    { dir: "North" as const, css: "n" },
    { dir: "South" as const, css: "s" },
    { dir: "East" as const, css: "e" },
    { dir: "West" as const, css: "w" },
    { dir: "NorthWest" as const, css: "nw" },
    { dir: "NorthEast" as const, css: "ne" },
    { dir: "SouthWest" as const, css: "sw" },
    { dir: "SouthEast" as const, css: "se" },
  ];

  const button = (index: number): PresenceButton =>
    cfg?.discord.buttons[index] ?? { enabled: false, label: "", url: "" };
</script>

{#if cfg && !cfg.onboarded}
  <!-- First run owns the whole window: an introduction competing with the
       panel it is introducing would be neither. -->
  <Welcome
    {cfg}
    {patch}
    finish={() => patch((c) => (c.onboarded = true))}
  />
{:else}
<div class="shell">
  <!-- Our own title bar: the window is undecorated so it can match the capsule,
       which means providing the two things a title bar is actually for. -->
  <header data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region>
      <span class="dot"></span>
      Lumen
      {#if info}<span class="version">{info.version}</span>{/if}
    </div>
    <div class="right">
      <span class="saved" class:on={saved}>{tr("Saved")}</span>
      <!-- Restart lives next to the badge that keeps asking for one. Lumen
           does not own playback, so a restart costs nothing but the capsule
           blinking out for a second. -->
      <button
        class="restart"
        title={tr("Restart Lumen")}
        aria-label={tr("Restart Lumen")}
        onclick={() => host.restart()}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M20 12a8 8 0 11-2.3-5.6M20 3v4h-4" />
        </svg>
        {tr("Restart")}
      </button>
      <button
        class="close"
        title={tr("Close")}
        aria-label="Close settings"
        onclick={() => getCurrentWindow().close()}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </div>
  </header>

  <div class="body">
    <nav aria-label="Settings sections">
      {#each TABS as item (item.id)}
        <button class="tab" class:active={tab === item.id} onclick={() => (tab = item.id)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d={item.icon} /></svg>
          {tr(item.label)}
        </button>
      {/each}
    </nav>

    <main>
      <!-- Row, Toggle and Hotkey translate their own props, so a language
           change has to rebuild them rather than merely re-render this file. -->
      {#key lang}
      {#if !cfg}
        <p class="hint">{tr("Loading settings…")}</p>
      {:else if tab === "general"}
        <h2>{tr("General")}</h2>
        <section>
          <!-- First, because it changes how everything below reads. Applied
               before the save round-trip so the panel switches on the click
               rather than after the file has been written. -->
          <Row label="Language" description="Follows Windows unless you pick one.">
            <select
              value={cfg.language}
              aria-label="Language"
              onchange={(e) => {
                const next = e.currentTarget.value as AppConfig["language"];
                applyLanguage(next);
                patch((c) => (c.language = next));
              }}
            >
              <option value="auto">{tr("Auto")}</option>
              <option value="en">English</option>
              <option value="ru">Русский</option>
            </select>
          </Row>

          <Row
            label="Start with Windows"
            description="Adds Lumen to this user's startup entries. The path is re-checked at every launch, so moving the exe does not silently break it."
          >
            <Toggle
              checked={cfg.startWithWindows}
              label="Start with Windows"
              onchange={(v) => patch((c) => (c.startWithWindows = v))}
            />
          </Row>

          <Row
            label="Stay on screen while paused"
            description="Off hides the capsule until playback resumes."
          >
            <Toggle
              checked={cfg.showWhilePaused}
              label="Stay on screen while paused"
              onchange={(v) => patch((c) => (c.showWhilePaused = v))}
            />
          </Row>

          <Row
            label="Open the panel on a track change"
            description="Briefly expands to show the new track, then collapses again."
          >
            <Toggle
              checked={cfg.autoExpandOnTrackChange}
              label="Open the panel on a track change"
              onchange={(v) => patch((c) => (c.autoExpandOnTrackChange = v))}
            />
          </Row>

          <Row label="How long it stays open">
            <div class="slider">
              <input
                type="range"
                min="800"
                max="8000"
                step="200"
                value={cfg.autoExpandMs}
                aria-label="Auto expand duration"
                oninput={(e) => patch((c) => (c.autoExpandMs = Number(e.currentTarget.value)))}
              />
              <span class="value">{(cfg.autoExpandMs / 1000).toFixed(1)}s</span>
            </div>
          </Row>

          <Row
            label="Always keep the panel open"
            description="Stays expanded instead of collapsing back to the pill. It still hides when nothing is playing — there is nothing to show."
          >
            <Toggle
              checked={cfg.alwaysExpanded}
              label="Always keep the panel open"
              onchange={(v) => patch((c) => (c.alwaysExpanded = v))}
            />
          </Row>

          <Row
            label="Pause when the machine locks"
            description="Only ever resumes what it paused itself, and only if nothing else started playing while you were away."
            restart
          >
            <Toggle
              checked={cfg.smartPause.enabled}
              label="Pause when the machine locks"
              onchange={(v) => patch((c) => (c.smartPause.enabled = v))}
            />
          </Row>

          <Row label="Start it playing again on unlock" restart>
            <Toggle
              checked={cfg.smartPause.resumeOnUnlock}
              disabled={!cfg.smartPause.enabled}
              label="Resume on unlock"
              onchange={(v) => patch((c) => (c.smartPause.resumeOnUnlock = v))}
            />
          </Row>
        </section>
      {:else if tab === "appearance"}
        <h2>{tr("Appearance")}</h2>
        <section>
          <Row label="Interface size" description="For a 2K or 4K screen running at 100%, where everything correctly sized is also tiny. Scales the capsule and this window together.">
            <div class="slider">
              <input
                type="range"
                min="0.75"
                max="2"
                step="0.05"
                value={cfg.uiScale}
                aria-label="Interface size"
                onchange={(e) => patch((c) => (c.uiScale = Number(e.currentTarget.value)))}
              />
              <span class="value">{Math.round(cfg.uiScale * 100)}%</span>
            </div>
          </Row>

          <Row label="Position" description="Where the capsule sits when it is not being dragged.">
            <select
              value={cfg.dock}
              aria-label="Dock position"
              onchange={(e) => patch((c) => (c.dock = e.currentTarget.value as AppConfig["dock"]))}
            >
              <option value="taskbar-center">Above the taskbar</option>
              <option value="bottom-left">Bottom left</option>
              <option value="bottom-right">Bottom right</option>
              <option value="top-left">Top left</option>
              <option value="top-right">Top right</option>
              <option value="free">Wherever I dropped it</option>
            </select>
          </Row>

          <Row
            label="Backdrop"
            description="Acrylic samples whatever is behind the window; Mica only tints from the wallpaper."
            restart
          >
            <select
              value={cfg.backdrop}
              aria-label="Backdrop"
              onchange={(e) =>
                patch((c) => (c.backdrop = e.currentTarget.value as AppConfig["backdrop"]))}
            >
              <option value="auto">Auto</option>
              <option value="acrylic">Acrylic</option>
              <option value="mica">Mica</option>
            </select>
          </Row>

          <Row label="Corners" restart>
            <select
              value={cfg.shape}
              aria-label="Corner shape"
              onchange={(e) => patch((c) => (c.shape = e.currentTarget.value as AppConfig["shape"]))}
            >
              <option value="round">Rounded</option>
              <option value="square">Square</option>
            </select>
          </Row>

          <Row label="Theme" restart>
            <select
              value={cfg.theme}
              aria-label="Theme"
              onchange={(e) => patch((c) => (c.theme = e.currentTarget.value as AppConfig["theme"]))}
            >
              <option value="system">Follow Windows</option>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </select>
          </Row>

          <Row label="Monitor" description="Which display to dock to when several are attached.">
            <select
              value={cfg.monitor}
              aria-label="Monitor"
              onchange={(e) =>
                patch((c) => (c.monitor = e.currentTarget.value as AppConfig["monitor"]))}
            >
              <option value="primary">Primary</option>
              <option value="cursor">Wherever the pointer is</option>
            </select>
          </Row>

          <Row
            label="Clawd"
            description="A pixel crab who dances in the capsule while music plays. Click him to make him stop, or start again."
          >
            <Toggle
              checked={cfg.pet.enabled}
              label="Clawd"
              onchange={(v) => patch((c) => (c.pet.enabled = v))}
            />
          </Row>

          <Row
            label="Spectrum"
            description="Live audio bars behind the panel. The only feature that costs CPU while it runs — about half a percent of one core, and only while the panel is open and playing."
          >
            <Toggle
              checked={cfg.spectrum.enabled}
              label="Spectrum"
              onchange={(v) => patch((c) => (c.spectrum.enabled = v))}
            />
          </Row>
        </section>
      {:else if tab === "audio"}
        <h2>{tr("Audio & mouse")}</h2>

        <h3>{tr("Boost")}</h3>
        <div class="notice">{tr("notice.boost")}</div>
        <section>
          <Row label="Volume boost" description="Past the 100% Windows allows. A limiter keeps peaks from clipping, so the loudest parts compress rather than distort.">
            <Toggle
              checked={cfg.boost.enabled}
              label="Volume boost"
              onchange={(v) => patch((c) => (c.boost.enabled = v))}
            />
          </Row>

          <Row label="Loudness">
            <div class="slider">
              <input
                type="range"
                min="1"
                max="3"
                step="0.05"
                disabled={!cfg.boost.enabled}
                value={cfg.boost.gain}
                aria-label="Boost amount"
                onchange={(e) => patch((c) => (c.boost.gain = Number(e.currentTarget.value)))}
              />
              <span class="value">{Math.round(cfg.boost.gain * 100)}%</span>
            </div>
          </Row>

          <Row label="Bass boost" description="A shelf below 120 Hz, so it lifts weight without muddying voices.">
            <div class="slider">
              <input
                type="range"
                min="0"
                max="12"
                step="0.5"
                disabled={!cfg.boost.enabled}
                value={cfg.boost.bassDb}
                aria-label="Bass boost"
                onchange={(e) => patch((c) => (c.boost.bassDb = Number(e.currentTarget.value)))}
              />
              <span class="value">+{cfg.boost.bassDb.toFixed(1)} dB</span>
            </div>
          </Row>
        </section>

        <h3>{tr("Wheel and clicks")}</h3>
        <section>
          <Row label="Volume step" description="How far one wheel notch moves the level.">
            <div class="slider">
              <input
                type="range"
                min="0.01"
                max="0.1"
                step="0.01"
                value={cfg.volumeStep}
                aria-label="Volume step"
                oninput={(e) => patch((c) => (c.volumeStep = Number(e.currentTarget.value)))}
              />
              <span class="value">{Math.round(cfg.volumeStep * 100)}%</span>
            </div>
          </Row>

          <Row
            label="Global mouse gestures"
            description="Installs one system-wide low-level hook. Off means no hook is created at all, and none of the gestures below exist."
            restart
          >
            <Toggle
              checked={cfg.mouse.enabled}
              label="Global mouse gestures"
              onchange={(v) => patch((c) => (c.mouse.enabled = v))}
            />
          </Row>

          <Row
            label="Scroll the taskbar to change volume"
            description="Over an app's button it moves that app; over an empty stretch it moves the system master."
            restart
          >
            <Toggle
              checked={cfg.mouse.taskbarWheelVolume}
              disabled={!cfg.mouse.enabled}
              label="Taskbar wheel volume"
              onchange={(v) => patch((c) => (c.mouse.taskbarWheelVolume = v))}
            />
          </Row>

          <Row
            label="Move the app's own volume, not the master"
            description="This is the one that reaches a stream: the master is applied at the endpoint, after anything capturing your audio has already taken it."
            restart
          >
            <Toggle
              checked={cfg.mouse.taskbarWheelPerApp}
              disabled={!cfg.mouse.enabled || !cfg.mouse.taskbarWheelVolume}
              label="Per-app volume"
              onchange={(v) => patch((c) => (c.mouse.taskbarWheelPerApp = v))}
            />
          </Row>

          <Row
            label="Keep working over full-screen windows"
            description="Scrolling where the taskbar would be still works when a game covers it, and targets the game."
            restart
          >
            <Toggle
              checked={cfg.mouse.taskbarWheelOverFullscreen}
              disabled={!cfg.mouse.enabled || !cfg.mouse.taskbarWheelVolume}
              label="Work over full-screen windows"
              onchange={(v) => patch((c) => (c.mouse.taskbarWheelOverFullscreen = v))}
            />
          </Row>

          <Row
            label="Close an app from its taskbar button"
            description="Sends a close request, which the app can still refuse or prompt about. Right-click replaces the jump list — choose it deliberately."
            restart
          >
            <select
              value={cfg.mouse.taskbarCloseButton}
              disabled={!cfg.mouse.enabled}
              aria-label="Taskbar close button"
              onchange={(e) =>
                patch(
                  (c) =>
                    (c.mouse.taskbarCloseButton = e.currentTarget
                      .value as AppConfig["mouse"]["taskbarCloseButton"]),
                )}
            >
              <option value="none">Off</option>
              <option value="middle">Middle-click</option>
              <option value="right">Right-click</option>
            </select>
          </Row>

          <Row label="Middle-click the capsule to hide it" restart>
            <Toggle
              checked={cfg.mouse.middleClickHides}
              disabled={!cfg.mouse.enabled}
              label="Middle-click hides"
              onchange={(v) => patch((c) => (c.mouse.middleClickHides = v))}
            />
          </Row>

          <Row label="Alt + middle-click quits Lumen" restart>
            <Toggle
              checked={cfg.mouse.altMiddleQuits}
              disabled={!cfg.mouse.enabled}
              label="Alt middle-click quits"
              onchange={(v) => patch((c) => (c.mouse.altMiddleQuits = v))}
            />
          </Row>
        </section>
      {:else if tab === "hotkeys"}
        <h2>{tr("Hotkeys")}</h2>
        <p class="hint">{tr("hint.hotkeys")}</p>
        <section>
          <Row label="Previous track" restart>
            <Hotkey
              value={cfg.hotkeys.previous}
              label="Previous track"
              onchange={(v) => patch((c) => (c.hotkeys.previous = v))}
            />
          </Row>
          <Row label="Play / pause" restart>
            <Hotkey
              value={cfg.hotkeys.playPause}
              label="Play or pause"
              onchange={(v) => patch((c) => (c.hotkeys.playPause = v))}
            />
          </Row>
          <Row label="Next track" restart>
            <Hotkey
              value={cfg.hotkeys.next}
              label="Next track"
              onchange={(v) => patch((c) => (c.hotkeys.next = v))}
            />
          </Row>
          <Row label="Switch source" description="Follow the next app that is playing." restart>
            <Hotkey
              value={cfg.hotkeys.cycleSession}
              label="Switch source"
              onchange={(v) => patch((c) => (c.hotkeys.cycleSession = v))}
            />
          </Row>

          <Row label="Volume up" description="The playing app's own level, same as the taskbar wheel." restart>
            <Hotkey
              value={cfg.hotkeys.volumeUp}
              label="Volume up"
              onchange={(v) => patch((c) => (c.hotkeys.volumeUp = v))}
            />
          </Row>
          <Row label="Volume down" restart>
            <Hotkey
              value={cfg.hotkeys.volumeDown}
              label="Volume down"
              onchange={(v) => patch((c) => (c.hotkeys.volumeDown = v))}
            />
          </Row>
          <Row label="Repeat" description="Steps the player's own repeat mode: off, whole list, one track." restart>
            <Hotkey
              value={cfg.hotkeys.repeat}
              label="Repeat"
              onchange={(v) => patch((c) => (c.hotkeys.repeat = v))}
            />
          </Row>
          <Row label="Hide or show the capsule" restart>
            <Hotkey
              value={cfg.hotkeys.toggleVisible}
              label="Hide or show the capsule"
              onchange={(v) => patch((c) => (c.hotkeys.toggleVisible = v))}
            />
          </Row>
          <Row label="Keep the panel open" description="Toggles the setting below, so it survives a restart." restart>
            <Hotkey
              value={cfg.hotkeys.togglePinned}
              label="Keep the panel open"
              onchange={(v) => patch((c) => (c.hotkeys.togglePinned = v))}
            />
          </Row>
        </section>
      {:else if tab === "discord"}
        <h2>{tr("Discord")}</h2>
        <p class="hint">{tr("hint.discord")}</p>

        <section>
          <Row
            label="Rich Presence"
            description="The master switch for everything on this page."
            restart
          >
            <Toggle
              checked={cfg.discord.enabled}
              label="Discord Rich Presence"
              onchange={(v) => patch((c) => (c.discord.enabled = v))}
            />
          </Row>

          <!-- The one place the two are exclusive, so the trade is stated where
               the choice is made rather than left to be discovered. -->
          <Row
            label="Show as"
            description="Listening gives you the progress bar, Playing gives you the buttons. Discord does not draw buttons on a Listening activity, so this is the choice between them."
          >
            <select
              value={cfg.discord.activity}
              aria-label="Activity type"
              onchange={(e) =>
                patch(
                  (c) =>
                    (c.discord.activity = e.currentTarget.value as AppConfig["discord"]["activity"]),
                )}
            >
              <option value="listening">{tr("Listening to — with progress")}</option>
              <option value="playing">{tr("Playing — with buttons")}</option>
            </select>
          </Row>
          <Row
            label="Application ID"
            description="discord.com/developers → your application → Application ID."
            restart
          >
            <input
              class="text mono"
              type="text"
              spellcheck="false"
              placeholder="18-digit ID"
              value={cfg.discord.applicationId}
              aria-label="Discord application ID"
              onchange={(e) =>
                patch((c) => (c.discord.applicationId = e.currentTarget.value.trim()))}
            />
          </Row>
        </section>

        <h3>{tr("What it shows")}</h3>
        <section>
          <Row label="Artist" description="The second line. Off publishes the title alone.">
            <Toggle
              checked={cfg.discord.showArtist}
              label="Show artist"
              onchange={(v) => patch((c) => (c.discord.showArtist = v))}
            />
          </Row>
          <Row label="Album" description="Hover text on the large image.">
            <Toggle
              checked={cfg.discord.showAlbum}
              label="Show album"
              onchange={(v) => patch((c) => (c.discord.showAlbum = v))}
            />
          </Row>
          <Row
            label="Which player"
            description="Names the app — “Listening via Lumen · Spotify” — instead of Lumen alone."
          >
            <Toggle
              checked={cfg.discord.showSource}
              label="Show the player"
              onchange={(v) => patch((c) => (c.discord.showSource = v))}
            />
          </Row>
          <Row
            label="Elapsed time"
            description="Discord animates this clock itself, so it keeps running between updates. Never shown while paused — it would count up on something that is not moving."
          >
            <Toggle
              checked={cfg.discord.showTimestamps}
              label="Show elapsed time"
              onchange={(v) => patch((c) => (c.discord.showTimestamps = v))}
            />
          </Row>
          <Row
            label="Keep it up while paused"
            description="Off clears the presence on a pause. A profile that still says “listening” hours after the music stopped is worse than one that says nothing."
            restart
          >
            <Toggle
              checked={cfg.discord.showWhilePaused}
              label="Show while paused"
              onchange={(v) => patch((c) => (c.discord.showWhilePaused = v))}
            />
          </Row>
        </section>

        <h3>{tr("Album cover")}</h3>
        <div class="notice">{tr("notice.cover")}</div>
        <section>
          <Row label="Use the real cover art" restart>
            <Toggle
              checked={cfg.discord.albumArt}
              label="Use the real cover art"
              onchange={(v) => patch((c) => (c.discord.albumArt = v))}
            />
          </Row>
        </section>

        <h3>{tr("Buttons")}</h3>
        <p class="hint">
          {tr("hint.buttons")}
          <!-- Placeholders stay in English: they are literal syntax, not prose. -->
          <code>{"{title}"}</code>, <code>{"{artist}"}</code>, <code>{"{album}"}</code>
        </p>
        <section>
          {#each [0, 1] as index (index)}
            <div class="button-editor">
              <div class="button-head">
                <Toggle
                  checked={button(index).enabled}
                  label={`Enable button ${index + 1}`}
                  onchange={(v) => setButton(index, (b) => (b.enabled = v))}
                />
                <input
                  class="text label-input"
                  type="text"
                  maxlength="32"
                  placeholder={index === 0 ? "Find this track" : "Get Lumen"}
                  value={button(index).label}
                  aria-label={`Button ${index + 1} label`}
                  onchange={(e) => setButton(index, (b) => (b.label = e.currentTarget.value))}
                />
              </div>
              <input
                class="text url-input mono"
                type="text"
                spellcheck="false"
                placeholder="https://…"
                value={button(index).url}
                aria-label={`Button ${index + 1} URL`}
                onchange={(e) => setButton(index, (b) => (b.url = e.currentTarget.value.trim()))}
              />
            </div>
          {/each}
        </section>

        <h3>{tr("Players")}</h3>
        <p class="hint">{tr("hint.players")}</p>
        <section>
          {#each sourceList as source (source)}
            <Row
              label={source}
              description={liveSources.includes(source) ? "Playing now" : undefined}
            >
              <Toggle
                checked={isPublished(source)}
                label={`Publish ${source}`}
                onchange={(v) => setPublished(source, v)}
              />
            </Row>
          {/each}
        </section>
      {:else if tab === "lyrics"}
        <h2>{tr("Lyrics")}</h2>
        <div class="notice">{tr("notice.lyrics")}</div>
        <section>
          <Row
            label="Show lyrics"
            description="Synced lyrics follow the song, word by word."
            restart
          >
            <Toggle
              checked={cfg.lyrics.enabled}
              label="Show lyrics"
              onchange={(v) => patch((c) => (c.lyrics.enabled = v))}
            />
          </Row>

          <Row
            label="Fall back to Genius"
            description="For tracks with no timed lyrics anywhere. Genius has no lyrics API, so this reads their web page and will break when it changes; the timings it produces are estimates, and are shown in italics."
            restart
          >
            <Toggle
              checked={cfg.lyrics.geniusFallback}
              disabled={!cfg.lyrics.enabled}
              label="Fall back to Genius"
              onchange={(v) => patch((c) => (c.lyrics.geniusFallback = v))}
            />
          </Row>

          <!-- Live: the island applies this when it picks a line, so dragging
               the slider moves the words against the song that is playing.
               That is the only way anyone can actually find the right value. -->
          <Row
            label="Sync"
            description="Shifts every line. Drag it while the song plays: left if the words come late, right if they run ahead."
          >
            <div class="slider">
              <input
                type="range"
                min="-3000"
                max="3000"
                step="100"
                value={cfg.lyrics.offsetMs}
                aria-label="Lyric sync offset"
                oninput={(e) => patch((c) => (c.lyrics.offsetMs = Number(e.currentTarget.value)))}
              />
              <span class="value">
                {cfg.lyrics.offsetMs > 0 ? "+" : ""}{(cfg.lyrics.offsetMs / 1000).toFixed(1)}s
              </span>
            </div>
          </Row>

          <Row
            label="Nudge estimated timings"
            description="On top of the sync above, for guessed timings only — they drift by their nature, and a real synced lyric should not be corrected twice."
          >
            <div class="slider">
              <input
                type="range"
                min="-5000"
                max="5000"
                step="250"
                value={cfg.lyrics.estimatedOffsetMs}
                aria-label="Estimated lyric offset"
                oninput={(e) =>
                  patch((c) => (c.lyrics.estimatedOffsetMs = Number(e.currentTarget.value)))}
              />
              <span class="value">{(cfg.lyrics.estimatedOffsetMs / 1000).toFixed(2)}s</span>
            </div>
          </Row>
        </section>
      {:else if tab === "advanced"}
        <h2>{tr("Advanced")}</h2>
        <p class="hint">{tr("hint.advanced")}</p>
        <section>
          <Row
            label="Gap from the edge"
            description="Between the capsule and the edge it is docked against."
          >
            <input
              class="number"
              type="number"
              min="0"
              max="200"
              value={cfg.taskbarGap}
              aria-label="Gap from the edge"
              onchange={(e) => patch((c) => (c.taskbarGap = Number(e.currentTarget.value)))}
            />
          </Row>

          <Row label="Corner inset" description="Inset from the side for the corner positions.">
            <input
              class="number"
              type="number"
              min="0"
              max="200"
              value={cfg.edgeMargin}
              aria-label="Corner inset"
              onchange={(e) => patch((c) => (c.edgeMargin = Number(e.currentTarget.value)))}
            />
          </Row>

          <Row
            label="Snap distance"
            description="How near a corner a drop must land to snap to it. Zero always keeps the exact drop position."
          >
            <input
              class="number"
              type="number"
              min="0"
              max="400"
              value={cfg.snapThreshold}
              aria-label="Snap distance"
              onchange={(e) => patch((c) => (c.snapThreshold = Number(e.currentTarget.value)))}
            />
          </Row>

          <Row
            label="Free position"
            description="Where a dropped capsule sits, measured from the work area's top-left. Used only by the “wherever I dropped it” position."
          >
            <div class="pair">
              <input
                class="number"
                type="number"
                value={cfg.freeX}
                aria-label="Free position X"
                onchange={(e) => patch((c) => (c.freeX = Number(e.currentTarget.value)))}
              />
              <input
                class="number"
                type="number"
                value={cfg.freeY}
                aria-label="Free position Y"
                onchange={(e) => patch((c) => (c.freeY = Number(e.currentTarget.value)))}
              />
            </div>
          </Row>
        </section>
      {:else}
        <h2>{tr("About")}</h2>
        <section class="about">
          <p class="tagline">{tr("A glass music capsule for Windows 11.")}</p>
          {#if info}
            <dl>
              <dt>{tr("Version")}</dt>
              <dd>{info.version}</dd>
              <dt>{tr("Settings file")}</dt>
              <dd class="mono">{info.configPath ?? tr("not persisted")}</dd>
              <dt>{tr("Mode")}</dt>
              <dd>
                {info.portable
                  ? tr("Portable — settings live beside the exe")
                  : tr("Roaming — settings in %APPDATA%")}
              </dd>
            </dl>
          {/if}
          <p class="note">{tr("about.note")}</p>
          <p class="note">
            Nothing leaves this machine unless you switch it on: lyrics and Discord cover art are
            the only features that use the network, and each says so where it is enabled.
          </p>
        </section>
      {/if}
      {/key}
    </main>
  </div>

  <!-- An undecorated window has no frame, and therefore none of the invisible
       borders Windows normally hands you to drag. These put them back: eight
       thin strips that ask the host to run a real resize loop, so it behaves
       like any other window rather than like a fixed panel. -->
  {#each RESIZE_EDGES as edge (edge.dir)}
    <div
      class="grip {edge.css}"
      role="presentation"
      onmousedown={(e) => {
        if (e.button === 0) getCurrentWindow().startResizeDragging(edge.dir);
      }}
    ></div>
  {/each}
</div>
{/if}

<style>
  /* 6px is the width Windows itself uses for a resize border: wide enough to
     hit without aiming, narrow enough not to steal clicks from the content. */
  .grip {
    position: fixed;
    z-index: 10;
  }
  .grip.n,
  .grip.s {
    left: 6px;
    right: 6px;
    height: 6px;
    cursor: ns-resize;
  }
  .grip.e,
  .grip.w {
    top: 6px;
    bottom: 6px;
    width: 6px;
    cursor: ew-resize;
  }
  .grip.n {
    top: 0;
  }
  .grip.s {
    bottom: 0;
  }
  .grip.w {
    left: 0;
  }
  .grip.e {
    right: 0;
  }
  .grip.nw,
  .grip.ne,
  .grip.sw,
  .grip.se {
    width: 12px;
    height: 12px;
  }
  .grip.nw {
    top: 0;
    left: 0;
    cursor: nwse-resize;
  }
  .grip.se {
    bottom: 0;
    right: 0;
    cursor: nwse-resize;
  }
  .grip.ne {
    top: 0;
    right: 0;
    cursor: nesw-resize;
  }
  .grip.sw {
    bottom: 0;
    left: 0;
    cursor: nesw-resize;
  }

  .shell {
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
    padding: 0 8px 0 16px;
    background: var(--sidebar);
    border-bottom: 1px solid var(--line);
    cursor: default;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--settings-accent);
    box-shadow: 0 0 10px var(--settings-accent);
  }

  .version {
    font-size: 11px;
    font-weight: 500;
    color: var(--ink-faint);
  }

  .right {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  /* Confirmation without a dialog: settings write immediately, and a brief
     acknowledgement is what tells you it landed. */
  .saved {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--ok);
    opacity: 0;
    transform: translateY(-2px);
    transition:
      opacity 200ms ease,
      transform 200ms ease;
  }

  .saved.on {
    opacity: 1;
    transform: none;
  }

  .restart {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px;
    border-radius: 7px;
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-dim);
    background: rgba(255, 255, 255, 0.05);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }

  .restart:hover {
    color: #fff;
    background: color-mix(in srgb, var(--settings-accent) 26%, transparent);
  }

  .restart svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .close {
    width: 30px;
    height: 30px;
    border-radius: 7px;
    display: grid;
    place-items: center;
    color: var(--ink-dim);
  }

  .close:hover {
    background: var(--danger);
    color: #fff;
  }

  .close svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
  }

  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 208px 1fr;
    min-height: 0;
  }

  nav {
    background: var(--sidebar);
    border-right: 1px solid var(--line);
    padding: 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 550;
    color: var(--ink-dim);
    text-align: left;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .tab:hover {
    background: rgba(255, 255, 255, 0.05);
    color: var(--ink);
  }

  .tab.active {
    background: color-mix(in srgb, var(--settings-accent) 20%, transparent);
    color: #fff;
  }

  .tab svg {
    width: 16px;
    height: 16px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  main {
    padding: 24px 30px 48px;
    overflow-y: auto;
    min-width: 0;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 19px;
    font-weight: 680;
    letter-spacing: -0.02em;
  }

  h3 {
    margin: 26px 0 0;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-faint);
  }

  .hint {
    margin: 8px 0 4px;
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--ink-dim);
    max-width: 68ch;
  }

  /* Said plainly, before the switch — not in a footnote after it. */
  .notice {
    margin: 14px 0 6px;
    padding: 12px 14px;
    border-radius: var(--radius);
    background: rgba(255, 190, 100, 0.08);
    box-shadow: inset 0 0 0 1px rgba(255, 190, 100, 0.22);
    font-size: 12.5px;
    line-height: 1.6;
    color: #ffe0b8;
    max-width: 74ch;
  }

  section {
    margin-top: 10px;
    background: var(--panel-2);
    border-radius: var(--radius);
    box-shadow: inset 0 0 0 1px var(--line);
    padding: 4px 18px;
  }

  select,
  .number,
  .text {
    background: rgba(255, 255, 255, 0.05);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    border: 0;
    border-radius: 8px;
    padding: 7px 10px;
    font-size: 12.5px;
    min-height: 34px;
  }

  select {
    min-width: 190px;
    cursor: pointer;
  }

  .number {
    width: 96px;
    text-align: right;
  }

  .text {
    width: 260px;
  }

  .mono {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 11.5px;
  }

  select:hover,
  .number:hover,
  .text:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  select:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* The open menu is drawn by the OS in its own colours; only the closed control
     is ours to style. */
  option {
    background: var(--panel-2);
    color: var(--ink);
  }

  .pair {
    display: flex;
    gap: 8px;
  }

  .slider {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  input[type="range"] {
    width: 180px;
    accent-color: var(--settings-accent);
  }

  .value {
    min-width: 46px;
    text-align: right;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--ink-dim);
  }

  /* A button is three fields that only mean anything together, so it gets its
     own block rather than three unrelated rows. */
  .button-editor {
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .button-editor:last-child {
    border-bottom: 0;
  }

  .button-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .label-input {
    width: 220px;
    font-weight: 600;
  }

  .url-input {
    width: 100%;
  }

  code {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 11px;
    background: rgba(255, 255, 255, 0.07);
    border-radius: 4px;
    padding: 1px 5px;
  }

  .about .tagline {
    font-size: 14px;
    color: var(--ink-dim);
    margin: 12px 0 20px;
  }

  dl {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 10px 16px;
    margin: 0 0 20px;
    font-size: 12.5px;
  }

  dt {
    color: var(--ink-faint);
  }

  dd {
    margin: 0;
    word-break: break-all;
  }

  .note {
    font-size: 12.5px;
    line-height: 1.7;
    color: var(--ink-dim);
    max-width: 70ch;
  }

</style>
