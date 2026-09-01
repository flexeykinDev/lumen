<script lang="ts">
  // Scrolls only when the text genuinely overflows, and only while the island is
  // showing it. A permanently-running marquee would keep the compositor awake
  // and break the zero-idle-cost promise.

  interface Props {
    text: string;
    /** Scrolling is suppressed entirely when false. */
    active?: boolean;
    /** Pixels per second. Slow enough to read, fast enough not to feel stuck. */
    speed?: number;
  }

  let { text, active = true, speed = 26 }: Props = $props();

  let track = $state<HTMLSpanElement | null>(null);
  let viewport = $state<HTMLSpanElement | null>(null);
  let overflow = $state(0);

  // Re-measure whenever the text or the box changes. `text` is read explicitly
  // so the effect re-runs on a track change even if the width happens to match.
  $effect(() => {
    void text;
    const el = track;
    const box = viewport;
    if (!el || !box) return;

    const measure = () => {
      overflow = Math.max(0, Math.ceil(el.scrollWidth - box.clientWidth));
    };
    measure();

    const ro = new ResizeObserver(measure);
    ro.observe(box);
    ro.observe(el);
    return () => ro.disconnect();
  });

  const scrolling = $derived(active && overflow > 0);
  // A fixed duration would crawl for long titles and sprint for short ones.
  const seconds = $derived(Math.max(4, (overflow * 2) / speed));
</script>

<span class="viewport" bind:this={viewport}>
  <span
    class="track"
    class:scrolling
    bind:this={track}
    style:--shift="{-overflow}px"
    style:--dur="{seconds}s"
    title={text}
  >
    {text}
  </span>
</span>

<style>
  .viewport {
    display: block;
    overflow: hidden;
    /* Fades the cut edge instead of chopping a glyph in half. */
    mask-image: linear-gradient(90deg, #000 0, #000 calc(100% - 18px), transparent 100%);
  }

  .track {
    display: inline-block;
    white-space: nowrap;
    will-change: transform;
  }

  .scrolling {
    animation: drift var(--dur) var(--ease, ease-in-out) infinite alternate;
    /* Hold each end long enough to actually read it. */
    animation-delay: 1.4s;
  }

  @keyframes drift {
    0%,
    18% {
      transform: translateX(0);
    }
    82%,
    100% {
      transform: translateX(var(--shift));
    }
  }
</style>
