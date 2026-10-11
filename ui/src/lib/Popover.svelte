<script lang="ts">
  import type { Snippet } from "svelte";
  import { placePanel, type Placed } from "./popoverPlace";

  /**
   * A small panel anchored above its trigger. Not a modal: the rest of the
   * screen stays live, and Escape or a click elsewhere closes it.
   *
   * It stays inside the window: narrower than the window, pushed in from an
   * edge it would cross, and opened below when there is no room above.
   */
  let {
    open = $bindable(false),
    align = "left",
    width = 320,
    label,
    trigger,
    children,
  }: {
    open?: boolean;
    align?: "left" | "right";
    width?: number;
    /** What the panel is, for a screen reader. */
    label?: string;
    trigger: Snippet;
    children: Snippet;
  } = $props();

  let root = $state<HTMLDivElement>();
  let panel = $state<HTMLDivElement>();
  let placed = $state<Placed | null>(null);

  function place() {
    if (!open || !root || !panel) return;
    const r = root.getBoundingClientRect();
    placed = placePanel(
      { left: r.left, right: r.right, top: r.top, bottom: r.bottom },
      { width: document.documentElement.clientWidth, height: window.innerHeight },
      width,
      panel.scrollHeight,
      align,
    );
  }

  $effect(() => {
    if (!open || !panel) {
      placed = null;
      return;
    }
    place();
    // Again once the width it was given has settled its height; and when
    // what is in it changes (a list filled, filtered), or anything scrolls
    // its trigger along.
    let raf = requestAnimationFrame(place);
    const soon = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(place);
    };
    const changed = new MutationObserver(soon);
    changed.observe(panel, { childList: true, subtree: true, characterData: true });
    window.addEventListener("scroll", soon, true);
    return () => {
      cancelAnimationFrame(raf);
      changed.disconnect();
      window.removeEventListener("scroll", soon, true);
    };
  });

  function onDocClick(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") open = false;
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={onKey} />
<svelte:window onresize={place} />

<div class="root" bind:this={root}>
  {@render trigger()}
  {#if open}
    <div
      class="panel"
      class:right={align === "right"}
      class:below={placed?.below}
      role="dialog"
      aria-label={label}
      bind:this={panel}
      style="width: {placed?.width ?? width}px; transform: translateX({placed?.shift ?? 0}px);{placed ? ` max-height: ${placed.maxHeight}px;` : ''}"
    >
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
    overflow: auto;
  }

  .panel.right {
    left: auto;
    right: 0;
  }

  .panel.below {
    bottom: auto;
    top: calc(100% + 8px);
  }
</style>
