<script lang="ts">
  import { plans } from "./plan.svelte";
  import PlanCard from "./PlanCard.svelte";
  import { arrows, CARD_H, CARD_W, closesCycle, layout, runView, shownSteps, suggestions, type StepResult } from "./planGraph";
  import { t } from "./i18n.svelte";

  /**
   * The plan as a graph: a column per wave, arrows from a step to the ones
   * that wait for it. Drag the empty space to pan, the wheel to zoom; drag a
   * card to put it somewhere (it stays there until the layout is redone);
   * drag a card's right handle onto another card to make that one wait for
   * it. A wait that would close a cycle is shown as refused while dragging,
   * and the core refuses it anyway. On a phone: one finger pans, two pinch
   * to zoom, a tap picks a card (its drawer opens below).
   */
  let { results }: { results: Map<string, StepResult> } = $props();

  /** A run in flight: the graph is the version it was given, with where each step is. */
  const running = $derived(runView(plans.doc.steps, plans.live));
  const shown = $derived(running ? running.steps.map((step) => ({ step, removed: false })) : shownSteps(plans.doc));
  const sug = $derived(running ? new Map() : suggestions(plans.doc));
  /** Where each card is: laid out, or where it is being dragged. */
  let dragging = $state<{ id: string; x: number; y: number } | null>(null);
  const at = $derived.by(() => {
    const placed = layout(shown.map((s) => s.step));
    if (dragging) placed.set(dragging.id, { x: dragging.x, y: dragging.y });
    return placed;
  });
  const lines = $derived(arrows(shown.map((s) => s.step)));

  let el = $state<HTMLDivElement>();
  let view = $state({ x: 40, y: 40, k: 1 });
  let panning: { x: number; y: number; vx: number; vy: number } | null = null;

  /** Fit the whole plan in view, no larger than life. */
  export function fit() {
    if (!el || !at.size) return;
    const xs = [...at.values()].map((p) => p.x);
    const ys = [...at.values()].map((p) => p.y);
    const x0 = Math.min(...xs) - 30;
    const y0 = Math.min(...ys) - 30;
    const x1 = Math.max(...xs) + CARD_W + 30;
    const y1 = Math.max(...ys) + CARD_H + 60;
    const k = Math.min(1, el.clientWidth / (x1 - x0), el.clientHeight / (y1 - y0));
    view = { k, x: (el.clientWidth - (x1 - x0) * k) / 2 - x0 * k, y: (el.clientHeight - (y1 - y0) * k) / 2 - y0 * k };
  }

  // Framed when the plan first has steps, when another plan opens, and
  // after a layout (plans.refit), so what moved is in view.
  let framed = "";
  $effect(() => {
    const key = `${plans.id}:${shown.length > 0}:${plans.refit}`;
    if (key === framed || !el) return;
    framed = key;
    queueMicrotask(fit);
  });

  // ----- two fingers: pinch to zoom about their middle -----
  const touches = new Map<number, { x: number; y: number }>();
  let pinch: { d: number; k: number } | null = null;
  const spread = () => {
    const [a, b] = [...touches.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };

  function toWorld(e: PointerEvent): { x: number; y: number } {
    const r = el!.getBoundingClientRect();
    return { x: (e.clientX - r.left - view.x) / view.k, y: (e.clientY - r.top - view.y) / view.k };
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    const r = el!.getBoundingClientRect();
    const cx = e.clientX - r.left;
    const cy = e.clientY - r.top;
    const k = Math.min(2, Math.max(0.3, view.k * Math.exp(-e.deltaY * 0.0015)));
    view = { k, x: cx - ((cx - view.x) * k) / view.k, y: cy - ((cy - view.y) * k) / view.k };
  }

  function downEmpty(e: PointerEvent) {
    if (e.pointerType === "touch") {
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (touches.size === 2) {
        // A second finger: no pan, no card drag, a pinch.
        panning = null;
        moving = null;
        pinch = { d: spread(), k: view.k };
        return;
      }
    }
    // A card is its own press; a button (the zoom's) keeps its click, which
    // capturing the pointer for a pan would take from it.
    if ((e.target as HTMLElement).closest("[data-step], button")) return;
    plans.pick("");
    panning = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  // ----- moving a card -----
  let moving: { id: string; dx: number; dy: number; sx: number; sy: number; moved: boolean } | null = null;

  function downCard(e: PointerEvent, id: string, removed: boolean) {
    if (removed) return;
    // During a run the graph is the run's: a card opens its step (if the
    // working plan still has it) and does not move.
    if (running) {
      if (plans.doc.steps.some((s) => s.id === id)) plans.pick(id, e.shiftKey);
      return;
    }
    plans.pick(id, e.shiftKey);
    const p = at.get(id)!;
    const w = toWorld(e);
    moving = { id, dx: w.x - p.x, dy: w.y - p.y, sx: e.clientX, sy: e.clientY, moved: false };
    el!.setPointerCapture(e.pointerId);
  }

  // ----- connecting: from a card's handle onto another card -----
  let linking = $state<{ from: string; x: number; y: number; over: string } | null>(null);
  const refused = $derived(!!linking?.over && closesCycle(plans.doc.steps, linking.over, linking.from));

  function downHandle(e: PointerEvent, id: string) {
    const w = toWorld(e);
    linking = { from: id, x: w.x, y: w.y, over: "" };
    el!.setPointerCapture(e.pointerId);
  }

  function cardUnder(e: PointerEvent): string {
    const hit = document.elementsFromPoint(e.clientX, e.clientY).find((n) => (n as HTMLElement).dataset?.step);
    return (hit as HTMLElement | undefined)?.dataset.step ?? "";
  }

  function move(e: PointerEvent) {
    if (touches.has(e.pointerId)) touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pinch && touches.size === 2) {
      const r = el!.getBoundingClientRect();
      const [a, b] = [...touches.values()];
      const cx = (a.x + b.x) / 2 - r.left;
      const cy = (a.y + b.y) / 2 - r.top;
      const k = Math.min(2, Math.max(0.3, (pinch.k * spread()) / Math.max(1, pinch.d)));
      view = { k, x: cx - ((cx - view.x) * k) / view.k, y: cy - ((cy - view.y) * k) / view.k };
      return;
    }
    if (panning) {
      view = { ...view, x: panning.vx + e.clientX - panning.x, y: panning.vy + e.clientY - panning.y };
    } else if (moving) {
      if (!moving.moved && Math.hypot(e.clientX - moving.sx, e.clientY - moving.sy) < 4) return;
      moving.moved = true;
      const w = toWorld(e);
      dragging = { id: moving.id, x: Math.round(w.x - moving.dx), y: Math.round(w.y - moving.dy) };
    } else if (linking) {
      const w = toWorld(e);
      const over = cardUnder(e);
      linking = { ...linking, x: w.x, y: w.y, over: over === linking.from ? "" : over };
    }
  }

  function up(e?: PointerEvent) {
    if (e) touches.delete(e.pointerId);
    if (pinch) {
      if (touches.size < 2) pinch = null;
      return;
    }
    if (moving?.moved && dragging) void plans.apply([{ op: "move", id: dragging.id, x: dragging.x, y: dragging.y }]);
    if (linking?.over && !refused) void plans.apply([{ op: "link", from: linking.from, to: linking.over }]);
    panning = null;
    moving = null;
    // Held until the core's answer comes back, so the card does not jump back first.
    setTimeout(() => (dragging = null), 150);
    linking = null;
  }

  /** An arrow from the right edge of one card to the left edge of another. */
  function curve(a: { x: number; y: number }, b: { x: number; y: number }) {
    const x1 = a.x + CARD_W;
    const y1 = a.y + CARD_H / 2;
    const x2 = b.x - 6;
    const y2 = b.y + CARD_H / 2;
    const mx = (x1 + x2) / 2;
    return `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`;
  }

  const picked = $derived(new Set(plans.doc.steps.filter((s) => s.id === plans.picked).map((s) => s.id)));
</script>

<div
  class="plate"
  bind:this={el}
  role="application"
  aria-label={t("plan.graphLabel")}
  onwheel={wheel}
  onpointerdown={downEmpty}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
>
  <div class="plane" style="transform: translate({view.x}px, {view.y}px) scale({view.k})">
    <svg class="edges" aria-hidden="true">
      <defs>
        <marker id="plan-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto">
          <path d="M0,0 L10,5 L0,10 z" class="tip" />
        </marker>
      </defs>
      {#each lines as l (l.from + ">" + l.to)}
        {@const a = at.get(l.from)}
        {@const b = at.get(l.to)}
        {#if a && b}
          <path
            class="line"
            class:sug={sug.has(l.to)}
            class:hot={picked.has(l.from) || picked.has(l.to)}
            d={curve(a, b)}
            marker-end="url(#plan-arrow)"
          />
        {/if}
      {/each}
      {#if linking}
        {@const a = at.get(linking.from)}
        {#if a}
          <path class="line drawing" class:refused d={curve(a, { x: linking.x + 6, y: linking.y - CARD_H / 2 })} />
        {/if}
      {/if}
    </svg>
    {#each shown as s (s.step.id + (s.removed ? "-gone" : ""))}
      {@const p = at.get(s.step.id)}
      {#if p}
        <div class="slot" class:target={linking?.over === s.step.id} class:refused={linking?.over === s.step.id && refused} style="left: {p.x}px; top: {p.y}px">
          <PlanCard
            step={s.step}
            suggestion={sug.get(s.step.id)}
            removed={s.removed}
            result={running ? undefined : results.get(s.step.id)}
            run={running?.states.get(s.step.id)}
            told={running?.told ?? false}
            pinned={s.step.x !== undefined && p.x === s.step.x && p.y === s.step.y}
            onpointerdown={(e) => downCard(e, s.step.id, s.removed)}
            onhandle={s.removed || running ? undefined : (e) => downHandle(e, s.step.id)}
          />
        </div>
      {/if}
    {/each}
  </div>
  {#if linking?.over && refused}
    <div class="note">{t("plan.cycleRefused")}</div>
  {/if}
  <div class="zoom">
    <button type="button" onclick={() => (view = { ...view, k: Math.max(0.3, view.k / 1.2) })} aria-label={t("plan.zoomOut")}>−</button>
    <button type="button" onclick={() => (view = { ...view, k: Math.min(2, view.k * 1.2) })} aria-label={t("plan.zoomIn")}>+</button>
    <button type="button" class="fit" onclick={fit}>{t("plan.fit")}</button>
  </div>
</div>

<style>
  .plate {
    position: relative;
    flex: 1 1 auto;
    min-height: 140px;
    overflow: hidden;
    background-color: var(--kg-paper);
    background-image: linear-gradient(var(--kg-grid) 1px, transparent 1px), linear-gradient(90deg, var(--kg-grid) 1px, transparent 1px);
    background-size: 24px 24px;
    touch-action: none;
    cursor: grab;
  }

  .plane {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
  }

  .edges {
    position: absolute;
    left: 0;
    top: 0;
    width: 1px;
    height: 1px;
    overflow: visible;
    pointer-events: none;
  }

  .line {
    fill: none;
    stroke: var(--kg-edge);
    stroke-width: 1.5;
  }

  .line.hot {
    stroke: var(--acc);
  }

  .line.sug {
    stroke: var(--acc);
    stroke-dasharray: 5 4;
  }

  .line.drawing {
    stroke: var(--hi);
    stroke-dasharray: 3 3;
  }

  .line.drawing.refused {
    stroke: var(--deltx);
  }

  .tip {
    fill: var(--dim);
  }

  .slot {
    position: absolute;
  }

  .slot.target :global(.card) {
    border-color: var(--ok);
  }

  .slot.refused :global(.card) {
    border-color: var(--deltx);
  }

  .note {
    position: absolute;
    left: 14px;
    bottom: 14px;
    padding: 4px 8px;
    font-size: 12px;
    color: var(--deltx);
    background: var(--delbg);
    border: 1px solid var(--deltx);
  }

  .zoom {
    position: absolute;
    right: 14px;
    bottom: 14px;
    display: flex;
    background: var(--card);
    border: 1px solid var(--line);
  }

  .zoom button {
    width: 30px;
    height: 28px;
    background: none;
    border: 0;
    border-right: 1px solid var(--line);
    color: var(--lab);
  }

  .zoom .fit {
    width: auto;
    padding: 0 9px;
    border-right: 0;
    font-size: 11px;
  }

  .zoom button:hover {
    color: var(--hi);
  }
</style>
