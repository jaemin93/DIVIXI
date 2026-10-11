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
  import { colorOrNone } from "./cssColor";
  import { whenLabel } from "./time";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import { plans } from "./plan.svelte";
  import ListSearch from "./ListSearch.svelte";
  import { routinesShown, routineTagCount } from "./routineFilter";
  import { toggleTag } from "./trackFilter";

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

  /** The search row and tag filter the other columns have (ListSearch): every picked tag must be on a routine. */
  let query = $state("");
  const shown = $derived(routinesShown(store.routines, query, store.routineTags));

  /** The right-click menu: the one designs have (rename, tags, colour, delete), with no export. */
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  const menued = $derived(menu ? store.routines.find((r) => r.id === menu!.id) : undefined);

  function openMenu(e: MouseEvent, id: string) {
    e.preventDefault();
    menu = { id, x: e.clientX, y: e.clientY };
  }

  /** Delete from the menu: "" when it is gone, else why not (the menu says it). */
  async function remove(id: string): Promise<string> {
    return store.deleteRoutine(id);
  }

  /** When it last ran: the run's own time, not when it was last edited. */
  function last(r: { last_run_at?: number | null; runs: number }): string {
    if (!r.runs || !r.last_run_at) return t("routines.never");
    return t("routines.lastRun", { when: whenLabel(r.last_run_at, now, store.lang) });
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

  <ListSearch
    bind:query
    placeholder={t("routines.search")}
    picked={store.routineTags}
    ontoggle={(tag) => store.setRoutineTags(toggleTag(store.routineTags, tag))}
    onclear={() => store.setRoutineTags([])}
    tagCount={(tag) => routineTagCount(store.routines, tag)}
  />

  <div class="scroll">
    <!-- No routines: the list is left empty; the head's "+" makes one. -->
    {#if store.routines.length}
      {#each shown as r (r.id)}
        {@const live = store.routineLive(r.id)}
        <div
          class="row"
          class:on={store.routine === r.id}
          class:menued={menu?.id === r.id}
          style="border-left-color: {colorOrNone(r.color)}"
          oncontextmenu={(e) => openMenu(e, r.id)}
          role="presentation"
        >
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
              {#if r.tags?.length}
                <span class="taglist" title={r.tags.join(", ")}>
                  {#each r.tags as tag (tag)}
                    <span class="mono chip" style="color: {store.tagColor(tag)}">{tag}</span>
                  {/each}
                </span>
              {/if}
              <span class="name">{r.name}</span>
              <span class="sub">{live ? t("routines.running") : last(r)}</span>
            </span>
          </button>
          <button
            class="go"
            disabled={!live && store.routineRequesting(r.id)}
            title={live ? t("routines.stop") : t("routines.runNow")}
            aria-label={live ? t("routines.stop") : t("routines.runNow")}
            onclick={() => (live ? store.cancelRoutine(r.id) : r.plan ? plans.requestRun(r.id) : store.runRoutine(r.id))}
          >
            <Icon name={live ? "close" : "play"} size={13} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</aside>

{#if menu && menued}
  {@const m = menued}
  <ArtifactMenu
    x={menu.x}
    y={menu.y}
    name={m.name}
    color={m.color ?? ""}
    busy={store.routineLive(m.id)}
    busyNote={t("routines.busyNote")}
    deleteNote={t("routines.deleteNote")}
    onclose={() => (menu = null)}
    onrename={async (name) => void (await store.saveRoutine(m.id, { name }))}
    ontags={() => (store.tagDialog = m.id)}
    oncolor={async (color) => void (await store.saveRoutine(m.id, { color }))}
    ondelete={() => remove(m.id)}
  />
{/if}

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

  /* The bar on the left is the routine's colour, as a track's. */
  .row {
    display: flex;
    align-items: stretch;
    border-left: 3px solid transparent;
  }

  .row.on,
  .row.menued,
  .row:hover {
    background: var(--sel);
  }

  /* Tags above the name, as on a track's row: they give way first. */
  .taglist {
    font-size: 9px;
    line-height: 1.3;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip {
    font-size: 9px;
    letter-spacing: 0.08em;
  }

  .chip + .chip {
    margin-left: 8px;
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
