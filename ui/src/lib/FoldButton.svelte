<script lang="ts">
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * A list column's fold button: the last thing in its head, after the "+"
   * (NewButton). It folds the column away; the rail's item brings it back.
   * Every list column draws this one (tracks, designs, libraries, routines).
   */
  let { onclick }: { onclick: () => void } = $props();
</script>

<button class="x" {onclick} aria-label={t("tracks.close")} title={t("tracks.close")}>
  <Icon name="collapse" />
</button>

<style>
  .x {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .x:hover {
    color: var(--hi);
    background: var(--sel);
  }
</style>
