<script lang="ts">
  /**
   * A grab strip on one edge of a column. Drag to resize, arrow keys to
   * nudge, double-click to reset. The parent must be `position: relative`.
   *
   * `edge` is which edge of the parent the handle sits on: "right" for a
   * column on the left of the screen (width grows as the pointer moves
   * right), "left" for a column on the right (width grows as it moves left).
   */
  let {
    edge,
    width,
    min,
    max,
    reset,
    onchange,
    label = "너비 조절",
  }: {
    edge: "left" | "right";
    width: number;
    min: number;
    max: number;
    reset: number;
    /** Called while dragging (persist=false) and once at the end (persist=true). */
    onchange: (px: number, persist: boolean) => void;
    label?: string;
  } = $props();

  let dragging = $state(false);

  function clamp(px: number): number {
    return Math.min(max, Math.max(min, Math.round(px)));
  }

  function startDrag(e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = width;
    dragging = true;
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      onchange(clamp(edge === "right" ? startWidth + dx : startWidth - dx), false);
    };
    const stop = () => {
      dragging = false;
      onchange(clamp(width), true);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
  }

  function onKey(e: KeyboardEvent) {
    const grow = edge === "right" ? "ArrowRight" : "ArrowLeft";
    const shrink = edge === "right" ? "ArrowLeft" : "ArrowRight";
    if (e.key === grow) onchange(clamp(width + 24), true);
    if (e.key === shrink) onchange(clamp(width - 24), true);
  }
</script>

<!-- A focusable window splitter is a recognised ARIA pattern; Svelte's
     generic checks do not know it. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="handle"
  class:left={edge === "left"}
  class:right={edge === "right"}
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuenow={width}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  onpointerdown={startDrag}
  onkeydown={onKey}
  ondblclick={() => onchange(reset, true)}
  title="드래그해서 너비 조절 · 더블클릭으로 기본 너비"
></div>

<style>
  /* Six pixels straddling the hairline; the strip itself turns accent on
     hover and while dragging, so the affordance is the border. */
  .handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 6px;
    cursor: col-resize;
    z-index: 5;
  }

  .handle.left {
    left: -3px;
  }

  .handle.right {
    right: -3px;
  }

  .handle:hover,
  .handle.dragging {
    background: var(--acc);
    opacity: 0.6;
  }

  .handle:focus-visible {
    outline: 1px solid var(--acc);
  }
</style>
