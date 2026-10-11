<script lang="ts">
  import { plans } from "./plan.svelte";
  import PlanStepEditor from "./PlanStepEditor.svelte";
  import { DRAWER_MIN, DRAWER_RESET, drawerMax } from "./planDrawer";
  import { t } from "./i18n.svelte";

  /**
   * The step drawer under the graph, made as the terminal under a track is:
   * it slides up and down from the toolbar's toggle, its top edge drags its
   * height (and the keyboard, and a double-click back to the start), and the
   * chevron on its bar lowers it. Picking a step does not raise it; raised,
   * it shows the step picked.
   */
  const step = $derived(plans.doc.steps.find((s) => s.id === plans.picked));

  // The slide, as the terminal's: the outer box's height goes between 0 and
  // the drawer's while the inside keeps its size; `gone` takes the lowered
  // drawer out of the focus order once it has slid away.
  let shown = $state(false);
  let gone = $state(true);
  let dragging = $state(false);
  $effect(() => {
    const open = plans.drawerOpen;
    if (open) gone = false;
    const frame = requestAnimationFrame(() => (shown = open));
    const timer = open ? 0 : setTimeout(() => (gone = !plans.drawerOpen), 320);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
    };
  });
  function onSlid(e: TransitionEvent) {
    if (e.target === e.currentTarget && !plans.drawerOpen) gone = true;
  }

  /** Ends a drag in progress: on release, on a cancelled pointer, and when the drawer goes. */
  let endDrag: (() => void) | null = null;

  function startDrag(e: PointerEvent) {
    e.preventDefault();
    endDrag?.();
    const grip = e.currentTarget as HTMLElement;
    // Captured, so a release outside the window still ends it.
    grip.setPointerCapture(e.pointerId);
    dragging = true;
    const startY = e.clientY;
    const start = plans.drawerHeight;
    const move = (ev: PointerEvent) => plans.setDrawerHeight(start + (startY - ev.clientY));
    const stop = () => {
      if (!endDrag) return;
      endDrag = null;
      dragging = false;
      plans.setDrawerHeight(plans.drawerHeight, true);
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", stop);
      grip.removeEventListener("pointercancel", stop);
      grip.removeEventListener("lostpointercapture", stop);
    };
    endDrag = stop;
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", stop);
    grip.addEventListener("pointercancel", stop);
    grip.addEventListener("lostpointercapture", stop);
  }
  $effect(() => () => endDrag?.());

  function onGripKey(e: KeyboardEvent) {
    const by = e.shiftKey ? 80 : 24;
    if (e.key === "ArrowUp") plans.setDrawerHeight(plans.drawerHeight + by, true);
    else if (e.key === "ArrowDown") plans.setDrawerHeight(plans.drawerHeight - by, true);
    else if (e.key === "Home") plans.setDrawerHeight(DRAWER_RESET, true);
    else return;
    e.preventDefault();
  }

  let armed = $state(false);
  function remove() {
    if (!step) return;
    if (!armed) {
      armed = true;
      return;
    }
    armed = false;
    void plans.apply([{ op: "delete", ids: [step.id] }]);
    plans.pick("");
  }
</script>

<section
  class="drawer"
  class:shown
  class:gone
  class:dragging
  style="height: {shown ? plans.drawerHeight : 0}px"
  ontransitionend={onSlid}
  aria-label={t("plan.drawer")}
  aria-hidden={!plans.drawerOpen}
>
  <div class="inner" style="height: {plans.drawerHeight}px">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="grip"
      role="separator"
      aria-orientation="horizontal"
      aria-label={t("plan.drawerResize")}
      aria-valuenow={plans.drawerHeight}
      aria-valuemin={DRAWER_MIN}
      aria-valuemax={drawerMax(window.innerHeight)}
      tabindex="0"
      onpointerdown={startDrag}
      onkeydown={onGripKey}
      ondblclick={() => plans.setDrawerHeight(DRAWER_RESET, true)}
      title={t("plan.drawerResize")}
    ></div>
    <div class="bar">
      {#if step}
        <span class="mono sid">{step.id}</span>
        <span class="dt">{step.title}</span>
      {:else}
        <span class="dt none">{t("plan.drawer")}</span>
      {/if}
      <span class="grow"></span>
      {#if step}
        <button class="btn" class:armed onclick={remove} onblur={() => (armed = false)}>
          {armed ? t("plan.step.deleteArm") : t("plan.step.delete")}
        </button>
      {/if}
      <button class="icon" onclick={() => plans.setDrawer(false)} aria-label={t("plan.drawerHide")} title={t("plan.drawerHide")}>
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M2 4l4 4 4-4" /></svg>
      </button>
    </div>
    <div class="body">
      {#if step}
        {#key step.id}
          <PlanStepEditor {step} />
        {/key}
      {:else}
        <p class="empty">{t("plan.drawerEmpty")}</p>
      {/if}
    </div>
  </div>
</section>

<style>
  /* As the terminal: slides up from the bottom edge and back down. */
  .drawer {
    position: relative;
    flex-shrink: 0;
    overflow: hidden;
    border-top: 1px solid transparent;
    background: var(--bg);
    transition:
      height 220ms cubic-bezier(0.2, 0.8, 0.2, 1),
      border-color 220ms;
  }

  .drawer.shown {
    border-top-color: var(--line);
  }

  .drawer.gone {
    visibility: hidden;
  }

  .drawer.dragging {
    transition: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .drawer {
      transition: none;
    }
  }

  .inner {
    position: relative;
    display: flex;
    flex-direction: column;
  }

  .grip {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 6px;
    cursor: row-resize;
    z-index: 5;
  }

  .bar {
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px 0 18px;
  }

  .sid {
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--dim);
  }

  .dt {
    font-weight: 600;
    color: var(--hi);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dt.none {
    font-weight: 400;
    color: var(--lab);
    font-size: 12px;
  }

  .grow {
    flex: 1;
  }

  .btn {
    height: 24px;
    padding: 0 9px;
    font-size: 12px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .btn:hover {
    color: var(--hi);
    border-color: var(--acc);
  }

  .btn.armed {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .icon {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
    padding: 0;
  }

  .icon:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .empty {
    margin: 0;
    padding: 6px 18px;
    font-size: 12px;
    color: var(--lab);
  }

  @media (max-width: 640px) {
    .bar {
      padding-left: 14px;
    }

    .empty {
      padding-left: 14px;
    }
  }
</style>
