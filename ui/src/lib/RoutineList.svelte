<script lang="ts">
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import Working from "./Working.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * Second column: every routine, grouped under the track it belongs to.
   * A routine is a saved instruction the human runs again when they want
   * it — never on a clock — so the row carries what they need to decide
   * that: whether it is running, when it last did and how it went.
   *
   * There is no "new routine" button here. A routine is made by telling the
   * conductor, which is the one place that has the instruction to save; the
   * empty state says so rather than offering a form that would start with a
   * blank box the human has to fill from memory.
   */
  const now = Date.now();

  const groups = $derived(
    store.tracks
      .map((tr) => ({
        id: tr.id,
        name: tr.name,
        items: store.routines.filter((r) => r.track === tr.id),
      }))
      .filter((g) => g.items.length > 0),
  );

  /** Routines whose track is gone: shown, so nothing disappears silently. */
  const orphans = $derived(store.routines.filter((r) => !store.tracks.some((tr) => tr.id === r.track)));

  function lastLabel(r: { last_run: string | null; updated_at: number; runs: number }): string {
    if (!r.runs) return t("routines.never");
    return t("routines.lastRun", { when: whenLabel(r.updated_at, now, store.lang) });
  }
</script>

<aside>
  <div class="head">
    <Icon name="routines" size={15} />
    <span class="title">{t("rail.routines")}</span>
  </div>

  <div class="scroll">
    {#if !store.routines.length}
      <p class="empty">{t("routines.empty")}</p>
    {:else}
      {#each [...groups, ...(orphans.length ? [{ id: "", name: t("routines.trackGone"), items: orphans }] : [])] as g (g.id)}
        <div class="group">
          <div class="gname">{g.name}</div>
          {#each g.items as r (r.id)}
            {@const live = store.routineLive(r)}
            <div class="row" class:on={store.routine === r.id}>
              <button class="pick" onclick={() => store.openRoutine(r.id)}>
                <span class="mark">
                  {#if live}<Working size={11} />{:else}<span class="dot"></span>{/if}
                </span>
                <span class="text">
                  <span class="name">{r.name}</span>
                  <span class="sub">{live ? t("routines.running") : lastLabel(r)}</span>
                </span>
              </button>
              <button
                class="go"
                disabled={live}
                title={live ? t("routines.running") : t("routines.runNow")}
                aria-label={t("routines.runNow")}
                onclick={() => store.runRoutine(r.id)}
              >
                <Icon name="play" size={13} />
              </button>
            </div>
          {/each}
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

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 0 12px;
  }

  .empty {
    margin: 14px 16px;
    font-size: 12px;
    line-height: 1.6;
    color: var(--lab);
  }

  .gname {
    padding: 10px 16px 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
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

  .go:hover:not(:disabled) {
    color: var(--hi);
  }

  .go:disabled {
    opacity: 0.35;
  }
</style>
