<script lang="ts">
  // Crossfades between covers instead of swapping them. `revision` is bumped by
  // the host on every track identity change, so it — not the URI — is the key:
  // two different tracks can legitimately share artwork, and re-fading in that
  // case would look like a glitch.

  interface Props {
    src: string | null;
    revision: number;
    /** `#rrggbb` used for the placeholder and the disc sheen. */
    accent: string;
  }

  let { src, revision, accent }: Props = $props();

  interface Layer {
    key: number;
    src: string | null;
  }

  let layers = $state<Layer[]>([]);
  let shown = $state.raw(-1);

  $effect(() => {
    const rev = revision;
    const uri = src;
    if (rev === shown) return;
    shown = rev;

    layers = [...layers.slice(-1), { key: rev, src: uri }];
  });

  function retire(key: number) {
    // Drop the outgoing layer once its fade has finished, so the DOM never holds
    // more than two base64 covers alive at a time.
    layers = layers.filter((l) => l.key === key || l === layers.at(-1));
  }
</script>

<div class="art" style:--accent={accent}>
  {#each layers as layer (layer.key)}
    <div
      class="layer"
      class:top={layer === layers.at(-1)}
      onanimationend={() => retire(layer.key)}
    >
      {#if layer.src}
        <img src={layer.src} alt="" draggable="false" />
      {:else}
        <div class="placeholder" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
            <path d="M9 18V5l10-2v13" stroke-linecap="round" stroke-linejoin="round" />
            <circle cx="6.5" cy="18" r="2.6" />
            <circle cx="16.5" cy="16" r="2.6" />
          </svg>
        </div>
      {/if}
    </div>
  {/each}
  <div class="sheen" aria-hidden="true"></div>
</div>

<style>
  /* The cover has to sit *on* the glass, not be pasted into it. Three layers do
     that: a drop shadow for lift, a hairline highlight along the top edge, and
     an accent halo picked from the artwork itself so the cover appears to be
     casting its own colour onto the capsule. */
  .art {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: inherit;
    overflow: hidden;
    background: color-mix(in srgb, var(--accent) 22%, #101219);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.22),
      inset 0 0 0 1px rgba(255, 255, 255, 0.1),
      0 6px 18px -6px rgba(0, 0, 0, 0.8),
      0 0 22px -6px color-mix(in srgb, var(--accent) 55%, transparent);
    transition: box-shadow var(--dur, 340ms) var(--ease, ease-out);
  }

  .layer {
    position: absolute;
    inset: 0;
    opacity: 0;
  }

  .layer.top {
    opacity: 1;
    animation: fade-in 420ms var(--ease, ease-out) both;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
      transform: scale(1.06);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .placeholder {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: color-mix(in srgb, var(--accent) 70%, white);
    background: linear-gradient(
      140deg,
      color-mix(in srgb, var(--accent) 30%, #0d0f16),
      #0d0f16
    );
  }

  .placeholder svg {
    width: 42%;
    height: 42%;
    opacity: 0.75;
  }

  /* A single diagonal highlight so the cover reads as glass, not a flat sticker. */
  .sheen {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      145deg,
      rgba(255, 255, 255, 0.16) 0%,
      rgba(255, 255, 255, 0) 42%
    );
    pointer-events: none;
  }
</style>
