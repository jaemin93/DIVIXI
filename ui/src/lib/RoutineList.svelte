<script lang="ts">
  import { store, NEW_ROUTINE } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import NewButton from "./NewButton.svelte";
  import FoldButton from "./FoldButton.svelte";
  import { overWeb } from "./ipc.svelte";
  import { phoneWidth } from "./phone";
  import { foldsOnPick } from "./uiMemory";
  import SplitHandle from "./SplitHandle.svelte";
  import Working from "./Working.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * Second column: every routine, newest activity first. A routine belongs
   * to no track and is nobody's worker, so this is a flat list — there is
   * no owner to group it under.
   *
   * Two ways in, and both are here: the "+" in the head (the one every list
   * column has, NewButton) opens a blank form for the human to fill, and a routine the conductor wrote appears
   * in the list beside it. Neither is the "real" one.
   */
  const now = Date.now();

  function last(r: { updated_at: number; runs: number }): string {
    if (!r.runs) return t("routines.never");
    return t("routines.lastRun", { when: whenLabel(r.updated_at, now, store.lang) });
  }
</script>

<aside style="width: {store.routineListWidth}px">
  <SplitHandle
    edge="right"
    width={store.routineListWidth}
    min={200}
    max={480}
    reset={260}
    label={t("routines.width")}
    onchange={(px, persist) => store.setRoutineListWidth(px, persist)}
  />
  <!-- As the tracks column: the title, the "+", then folding the column away, over a rule. -->
  <div class="head">
    <span class="mlab">{t("rail.routines")}</span>
    <span class="grow"></span>
    <NewButton label={t("routines.new")} on={store.routine === NEW_ROUTINE} onclick={() => store.newRoutine()} />
    <FoldButton onclick={() => store.setRoutineList(false)} />
  </div>

  <div class="scroll">
    <!-- No routines: the list is left empty; the head's "+" makes one. -->
    {#if store.routines.length}
      {#each store.routines as r (r.id)}
        {@const live = store.routineLive(r.id)}
        <div class="row" class:on={store.routine === r.id}>
          <button
            class="pick"
            onclick={() => {
              void store.openRoutine(r.id);
              // A phone: done with once a row is picked, as the other lists are.
              if (foldsOnPick(overWeb, phoneWidth(innerWidth))) void store.setRoutineList(false);
            }}
          >
            <span class="mark">
              {#if live}<Working size={11} />{:else}<span class="dot"></span>{/if}
            </span>
            <span class="text">
              <span class="name">{r.name}</span>
              <span class="sub">{live ? t("routines.running") : last(r)}</span>
            </span>
          </button>
          <button
            class="go"
            title={live ? t("routines.stop") : t("routines.runNow")}
            aria-label={live ? t("routines.stop") : t("routines.runNow")}
            onclick={() => (live ? store.cancelRoutine(r.id) : store.runRoutine(r.id))}
          >
            <Icon name={live ? "close" : "play"} size={13} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</aside>

<style>
  aside {
    position: relative;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 16px;
    border-bottom: 1px solid var(--line);
  }

  .grow {
    flex: 1;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0 12px;
  }

  .row {
    display: flex;
    align-items: stretch;
  }

  .row.on,
  .row:hover {
    background: var(--sel);
  }

  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 7px 4px 7px 16px;
    background: transparent;
    border: 0;
    text-align: left;
  }

  .mark {
    width: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  /* A routine at rest: present, not asking for anything. */
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--lab);
  }

  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .name {
    font-size: 13px;
    color: var(--hi);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub {
    font-size: 11px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The one action on the row, and the whole point of the list. */
  .go {
    width: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .go:hover {
    color: var(--hi);
  }
</style>
