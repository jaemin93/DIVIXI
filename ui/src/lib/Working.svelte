<script lang="ts">
  import Mark from "./Mark.svelte";

  /**
   * "Working": the mark with its X turning. One component for every place
   * that used to blink a green dot while an agent was busy, so the motion
   * is the brand's, not a generic spinner.
   */
  let { size = 14, label = "" }: { size?: number; label?: string } = $props();
</script>

<span class="working" role="status">
  <Mark {size} live={true} />
  {#if label}<span class="mono label">{label}</span>{/if}
</span>

<style>
  .working {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    flex-shrink: 0;
  }

  .label {
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }
</style>
