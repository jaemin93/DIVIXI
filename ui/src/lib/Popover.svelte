<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * A small panel anchored above its trigger. Not a modal: the rest of the
   * screen stays live, and Escape or a click elsewhere closes it.
   */
  let {
    open = $bindable(false),
    align = "left",
    width = 320,
    trigger,
    children,
  }: {
    open?: boolean;
    align?: "left" | "right";
    width?: number;
    trigger: Snippet;
    children: Snippet;
  } = $props();

  let root = $state<HTMLDivElement>();

  function onDocClick(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") open = false;
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={onKey} />

<div class="root" bind:this={root}>
  {@render trigger()}
  {#if open}
    <div class="panel" class:right={align === "right"} style="width: {width}px">
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .root {
    position: relative;
    display: inline-flex;
  }

  .panel {
    position: absolute;
    bottom: calc(100% + 8px);
    left: 0;
    z-index: 30;
    background: var(--card);
    border: 1px solid var(--lines);
    display: flex;
    flex-direction: column;
    max-height: 420px;
  }

  .panel.right {
    left: auto;
    right: 0;
  }
</style>
