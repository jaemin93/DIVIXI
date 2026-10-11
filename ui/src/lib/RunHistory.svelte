<script lang="ts">
  import { store, type RoutineRun } from "./store.svelte";
  import { plans } from "./plan.svelte";
  import { resultGlyphs, resultsOf } from "./planGraph";
  import { whenFull } from "./time";
  import { t } from "./i18n.svelte";

  /**
   * A routine's runs as a menu under the plan: closed, it names the run the
   * plan shows (or the latest); open, the newest few, and the older ones by
   * scrolling, read a page at a time. Picking a run shows its step results
   * on the plan.
   */
  let open = $state(false);
  let loading = $state(false);
  let el = $state<HTMLDivElement>();

  const runs = $derived(store.routineRuns);
  const chosen = $derived(plans.resultRun ? runs.find((r) => r.id === plans.resultRun) : undefined);

  function glyphs(run: RoutineRun): string {
    const order = run.plan?.steps?.map((s) => s.id) ?? [];
    return order.length ? resultGlyphs(order, resultsOf(run.steps)) : "";
  }

  function pick(id: string) {
    plans.resultRun = id;
    open = false;
  }

  /** Near the bottom of the list: read the next page, if there may be one. */
  async function scrolled(e: Event) {
    const box = e.currentTarget as HTMLElement;
    if (loading || box.scrollTop + box.clientHeight < box.scrollHeight - 24) return;
    loading = true;
    try {
      await store.loadMoreRoutineRuns();
    } finally {
      loading = false;
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") open = false;
  }

  function outside(e: MouseEvent) {
    if (open && el && !el.contains(e.target as Node)) open = false;
  }
</script>

<svelte:document onclick={outside} onkeydown={key} />

<div class="hist" bind:this={el}>
  <button type="button" class="pickbtn" aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="flab">{t("routines.history")}</span>
    <span class="cur">{chosen ? whenFull(chosen.started_at, store.lang) : t("plan.runsLatest")}</span>
    <span aria-hidden="true">▾</span>
  </button>
  {#if open}
    <div class="menu" role="menu" aria-label={t("routines.history")} onscroll={scrolled}>
      <button type="button" class="run" class:on={!plans.resultRun} role="menuitemradio" aria-checked={!plans.resultRun} onclick={() => pick("")}>
        <span class="when">{t("plan.runsLatest")}</span>
      </button>
      {#each runs as run (run.id)}
        {@const g = glyphs(run)}
        <button
          type="button"
          class="run"
          class:on={plans.resultRun === run.id}
          role="menuitemradio"
          aria-checked={plans.resultRun === run.id}
          onclick={() => pick(run.id)}
        >
          <span class="when">{whenFull(run.started_at, store.lang)}</span>
          <span class="meta">
            <span class="st {run.status}">{t(`routines.status.${run.status}`)}</span>
            <span class="mono">{run.plan ? `v${run.plan.version}` : t("plan.runNoPlan")}</span>
            {#if g}<span class="mono g">{g}</span>{/if}
          </span>
        </button>
      {/each}
      {#if loading}<p class="more">…</p>{/if}
    </div>
  {/if}
</div>

<style>
  .hist {
    position: relative;
  }

  .pickbtn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 26px;
    padding: 0 10px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--txt);
    font-size: 12px;
  }

  .pickbtn:hover,
  .pickbtn:focus-visible {
    border-color: var(--acc);
  }

  .flab {
    font-size: 11px;
    font-weight: 600;
    color: var(--lab);
  }

  /* Opens upwards: it sits at the bottom of the page. About five runs show; the rest scroll. */
  .menu {
    position: absolute;
    left: 0;
    bottom: 32px;
    z-index: 20;
    width: 320px;
    max-height: 236px;
    overflow-y: auto;
    overscroll-behavior: contain;
    background: var(--card);
    border: 1px solid var(--lines);
    display: flex;
    flex-direction: column;
  }

  .run {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex-shrink: 0;
    text-align: left;
    padding: 7px 10px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    border-bottom: 1px solid var(--lineq);
    color: var(--txt);
    font-size: 12px;
  }

  .run:hover,
  .run:focus-visible {
    background: var(--sel);
  }

  .run:focus-visible {
    outline: 1px solid var(--focus);
    outline-offset: -1px;
  }

  .run.on {
    border-left-color: var(--acc);
    background: var(--sel);
  }

  .meta {
    display: flex;
    gap: 10px;
    font-size: 11px;
    color: var(--lab);
  }

  .st.failed {
    color: var(--deltx);
  }

  .st.running {
    color: var(--acct);
  }

  .g {
    letter-spacing: 0.1em;
    color: var(--txt);
  }

  .more {
    margin: 0;
    padding: 6px 10px;
    color: var(--lab);
    font-size: 11px;
  }

  /* A phone: the menu as a sheet across the bottom, above the keyboard line. */
  @media (max-width: 640px) {
    .menu {
      position: fixed;
      left: 8px;
      right: 8px;
      bottom: 56px;
      width: auto;
      max-height: 50vh;
    }

    .run {
      padding: 10px 12px;
    }
  }
</style>
