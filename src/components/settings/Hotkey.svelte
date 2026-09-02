<script lang="ts">
  // A hotkey field that records the next chord you press.
  //
  // Typing "Ctrl+Shift+F6" into a text box is how this is usually done and it is
  // consistently miserable: the format is undocumented, mistakes are silent, and
  // you cannot tell a rejected binding from a broken one. Pressing the keys is
  // unambiguous, and it produces exactly the string the host parses.

  interface Props {
    value: string;
    label: string;
    onchange: (value: string) => void;
  }

  const { value, label, onchange }: Props = $props();

  let recording = $state(false);

  /** Keys that only ever qualify another key. */
  const MODIFIERS = new Set(["Control", "Shift", "Alt", "Meta", "OS"]);

  /** Turn a KeyboardEvent into the host's accelerator syntax. */
  function toAccelerator(e: KeyboardEvent): string | null {
    if (MODIFIERS.has(e.key)) return null;

    const parts: string[] = [];
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.shiftKey) parts.push("Shift");
    if (e.altKey) parts.push("Alt");
    if (e.metaKey) parts.push("Super");

    let key = e.key;
    if (key === " ") key = "Space";
    else if (key.length === 1) key = key.toUpperCase();
    else if (/^F\d{1,2}$/.test(key)) {
      // Already the right shape.
    } else if (key.startsWith("Arrow")) key = key.slice(5);

    parts.push(key);
    return parts.join("+");
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();

    // Escape abandons; Backspace and Delete clear the binding, which is the only
    // way to turn a hotkey off.
    if (e.key === "Escape") {
      recording = false;
      return;
    }
    if (e.key === "Backspace" || e.key === "Delete") {
      onchange("");
      recording = false;
      return;
    }

    const accelerator = toAccelerator(e);
    // A bare modifier is someone still reaching for the rest of the chord.
    if (!accelerator) return;
    onchange(accelerator);
    recording = false;
  }
</script>

<svelte:window on:keydown={onKeyDown} />

<div class="hotkey">
  <button
    type="button"
    class="field"
    class:recording
    aria-label={recording ? `Press a key for ${label}` : `${label}: ${value || "not set"}`}
    onclick={() => (recording = !recording)}
    onblur={() => (recording = false)}
  >
    {#if recording}
      <span class="prompt">Press keys…</span>
    {:else if value}
      <span class="keys">
        {#each value.split("+") as part (part)}
          <kbd>{part}</kbd>
        {/each}
      </span>
    {:else}
      <span class="empty">Not set</span>
    {/if}
  </button>

  {#if value && !recording}
    <button type="button" class="clear" title="Clear this binding" onclick={() => onchange("")}>
      ✕
    </button>
  {/if}
</div>

<style>
  .hotkey {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .field {
    min-width: 168px;
    min-height: 34px;
    padding: 5px 10px;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.05);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      background 150ms var(--ease),
      box-shadow 150ms var(--ease);
  }

  .field:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  /* Recording is a mode, and a mode that is not obvious is a trap — every
     keystroke is being swallowed while it is on. */
  .field.recording {
    background: color-mix(in srgb, var(--settings-accent) 18%, transparent);
    box-shadow: inset 0 0 0 1px var(--settings-accent);
    animation: pulse 1.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--settings-accent) 45%, transparent);
    }
  }

  .keys {
    display: flex;
    gap: 4px;
  }

  kbd {
    font-family: inherit;
    font-size: 11.5px;
    font-weight: 600;
    padding: 3px 7px;
    border-radius: 5px;
    background: rgba(255, 255, 255, 0.1);
    box-shadow:
      inset 0 0 0 1px var(--line-strong),
      0 1px 0 rgba(0, 0, 0, 0.3);
  }

  .prompt {
    font-size: 12px;
    font-weight: 600;
    color: var(--settings-accent);
  }

  .empty {
    font-size: 12px;
    color: var(--ink-faint);
  }

  .clear {
    width: 24px;
    height: 24px;
    border-radius: 6px;
    color: var(--ink-faint);
    font-size: 11px;
  }

  .clear:hover {
    color: var(--danger);
    background: rgba(255, 107, 107, 0.12);
  }

  @media (prefers-reduced-motion: reduce) {
    .field.recording {
      animation: none;
    }
  }
</style>
