<script lang="ts">
  // A switch.
  //
  // Built on a real checkbox rather than a styled div: that is what gives it
  // keyboard focus, space to toggle, and a state screen readers can announce,
  // none of which is worth reimplementing badly.

  interface Props {
    checked: boolean;
    disabled?: boolean;
    label: string;
    onchange: (value: boolean) => void;
  }

  const { checked, disabled = false, label, onchange }: Props = $props();
</script>

<label class="switch" class:disabled>
  <input
    type="checkbox"
    {checked}
    {disabled}
    aria-label={label}
    onchange={(e) => onchange(e.currentTarget.checked)}
  />
  <span class="track"><span class="thumb"></span></span>
</label>

<style>
  .switch {
    position: relative;
    display: inline-flex;
    cursor: pointer;
  }

  .switch.disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }

  /* Visually hidden, not `display: none` — a hidden input is not focusable and
     would take the control off the keyboard path entirely. */
  input {
    position: absolute;
    inset: 0;
    opacity: 0;
    margin: 0;
    cursor: inherit;
  }

  .track {
    width: 40px;
    height: 23px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.12);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    transition:
      background 180ms var(--ease),
      box-shadow 180ms var(--ease);
    display: flex;
    align-items: center;
    padding: 0 3px;
  }

  .thumb {
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.45);
    transition: transform 200ms var(--ease-snap);
  }

  input:checked + .track {
    background: var(--settings-accent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--settings-accent) 70%, white);
  }

  input:checked + .track .thumb {
    transform: translateX(17px);
  }

  input:focus-visible + .track {
    outline: 2px solid var(--settings-accent);
    outline-offset: 3px;
  }

  @media (prefers-reduced-motion: reduce) {
    .track,
    .thumb {
      transition: none;
    }
  }
</style>
