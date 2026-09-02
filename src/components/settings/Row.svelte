<script lang="ts">
  // One setting: a label, an explanation, and a control.
  //
  // Every setting goes through this so the column of controls stays aligned and
  // the explanations sit in the same place. The description is not optional
  // decoration — a toggle called "Per-app volume" means nothing without the
  // sentence saying what it does instead.

  import { t } from "../../lib/i18n";

  interface Props {
    label: string;
    /** Explicitly `| undefined`: callers pass a computed description that is
        sometimes absent, which `exactOptionalPropertyTypes` otherwise refuses. */
    description?: string | undefined;
    /** Marks a setting that needs a restart, which is worth saying up front. */
    restart?: boolean;
    children?: import("svelte").Snippet;
  }

  const { label, description, restart = false, children }: Props = $props();
</script>

<div class="row">
  <div class="text">
    <!-- Translated here rather than at every call site: this component is the
         one place every setting's text passes through. -->
    <div class="label">
      {t(label)}
      {#if restart}<span class="badge" title="Takes effect after restarting Lumen">{t("restart")}</span>{/if}
    </div>
    {#if description}<p class="desc">{t(description)}</p>{/if}
  </div>
  <div class="control">
    {@render children?.()}
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: 24px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }

  .row:last-child {
    border-bottom: 0;
  }

  .text {
    min-width: 0;
  }

  .label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13.5px;
    font-weight: 600;
    letter-spacing: -0.005em;
  }

  .desc {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--ink-dim);
    max-width: 62ch;
  }

  .badge {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: #ffd08a;
    background: rgba(255, 190, 100, 0.14);
    border: 1px solid rgba(255, 190, 100, 0.28);
    border-radius: 999px;
    padding: 2px 7px;
  }

  .control {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
