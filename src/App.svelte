<script lang="ts">
  import { onMount } from "svelte";
  import Island from "./components/Island.svelte";
  import { island } from "./lib/state.svelte";

  onMount(() => {
    let dispose: (() => void) | undefined;
    let cancelled = false;

    island.connect().then((fn) => {
      // A hot reload can unmount before `connect` resolves; without this the
      // listeners outlive the component and fire into a dead tree.
      if (cancelled) fn();
      else dispose = fn;
    });

    return () => {
      cancelled = true;
      dispose?.();
    };
  });
</script>

<Island />
