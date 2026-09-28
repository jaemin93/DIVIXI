<script lang="ts">
  import { store, NEW_ROUTINE } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import Working from "./Working.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * Second column: every routine, newest activity first. A routine belongs
   * to no track and is nobody's worker, so this is a flat list — there is
   * no owner to group it under.
   *
   * Two ways in, and both are here: the button at the top opens a blank
   * form for the human to fill, and a routine the conductor wrote appears
   * in the list beside it. Neither is the "real" one.
   */
  const now = Date.now();

  function last(r: { updated_at: number; runs: number }): string {
    if (!r.runs) return t("routines.never");
    return t("routines.lastRun", { when: whenLabel(r.updated_at, now, store.lang) });
  }
</script>

<aside>
  <div class="head">
    <Icon name="routines" size={15} />
    <span class="title">{t("rail.routines")}</span>
  </div>

  <div class="newrow">
    <button class="new" class:on={store.routine === NEW_ROUTINE} onclick={() => store.newRoutine()}>
      + {t("routines.new")}
    </button>
  </div>

  <div class="scroll">
    {#if !store.routines.length}
      <p class="empty">{t("routines.empty")}</p>
    {:else}
      {#each store.routines as r (r.id)}
        {@const live = store.routineLive(r.id)}
        <div class="row" class:on={store.routine === r.id}>
          <button class="pick" onclick={() => store.openRoutine(r.id)}>
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
    width: 260px;
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
    color: var(--lab);
  }

  .title {
    font-size: 13px;
    font-weight: 600;
    color: var(--hi);
  }

  .newrow {
    flex-shrink: 0;
    padding: 10px 12px 6px;
  }

  .new {
    width: 100%;
    height: 32px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--lab);
    font-size: 13px;
  }

  .new:hover,
  .new.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 2px 0 12px;
  }

  .empty {
    margin: 10px 16px;
    font-size: 12px;
    line-height: 1.6;
    color: var(--lab);
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
