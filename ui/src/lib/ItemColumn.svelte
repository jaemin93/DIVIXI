<script module lang="ts">
  /** One row of a list column. */
  export type ColumnItem = {
    id: string;
    name: string;
    tags: string[];
    color: string;
    /** The small fact on the right of the upper line: a time, a count. */
    meta: string;
    metaTitle?: string;
    /** Has a right-click menu (the row of all libraries has none). */
    menu?: boolean;
  };
</script>

<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import SplitHandle from "./SplitHandle.svelte";
  import Icon from "./Icon.svelte";
  import ListSearch from "./ListSearch.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import { narrow } from "./listFilter";

  /**
   * A page's list column, made like the tracks column: the title, a "+"
   * button and the fold button over a rule; the search row and its tag
   * filter (ListSearch, the tracks column's own); a row per item, tags and a
   * small fact above, the name below, its colour as the bar on the left, the
   * open one told by its shade. The "+" opens a name box under the head (a
   * design or a library needs a name only; a track opens its own form).
   * Right-clicking a row asks the page for its menu (rename, tags, colour,
   * delete), which the page draws. `pinned` rows (all libraries) stay on top
   * whatever the search. The designs page and the knowledge page's libraries
   * both use it, so the three columns read as one.
   */

  let {
    title,
    width,
    min = 200,
    max = 480,
    reset = 264,
    widthLabel,
    onwidth,
    onclose,
    newLabel,
    newButton,
    newPlaceholder,
    searchPlaceholder,
    maxlength = 80,
    oncreate,
    note = "",
    pinned = [],
    items,
    picked,
    onpick_tags,
    current,
    menued = null,
    onpick,
    onmenu,
    empty,
  }: {
    title: string;
    width: number;
    min?: number;
    max?: number;
    reset?: number;
    widthLabel: string;
    onwidth: (px: number, persist: boolean) => void;
    onclose: () => void;
    newLabel: string;
    /** The head's button, as the tracks column's "+ TRACK". */
    newButton: string;
    newPlaceholder: string;
    searchPlaceholder: string;
    maxlength?: number;
    /** Make one from the box. Resolves to why not (the box keeps its text), or nothing. */
    oncreate: (name: string) => Promise<string | void> | string | void;
    /** A line under the box: what went wrong with a name, say. */
    note?: string;
    /** Rows above the rest that the search and filter leave alone (all libraries). */
    pinned?: ColumnItem[];
    items: ColumnItem[];
    /** The tags the column is narrowed to, kept as the tracks column keeps its filter. */
    picked: string[];
    onpick_tags: (tags: string[]) => void;
    current: string | null;
    menued?: string | null;
    onpick: (id: string) => void;
    onmenu: (id: string, x: number, y: number) => void;
    empty: string;
  } = $props();

  /** Slides like the tracks column; not for those who asked for less motion. */
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const side = { axis: "x" as const, duration: reduced ? 0 : 200, easing: cubicOut };

  let draft = $state("");
  /** The name box under the head, opened by the head's "+". */
  let creating = $state(false);
  let query = $state("");
  const shown = $derived(narrow(items, query, picked));
  const filtering = $derived(!!query.trim() || picked.length > 0);

  async function create(e: Event) {
    e.preventDefault();
    const why = await oncreate(draft);
    if (!why) {
      draft = "";
      creating = false;
    }
  }

  function toggleTag(tag: string) {
    onpick_tags(picked.includes(tag) ? picked.filter((x) => x !== tag) : [...picked, tag]);
  }

  /** How many of this column's rows carry a tag (pinned rows are no one's). */
  function tagCount(tag: string): number {
    return items.filter((i) => i.tags.includes(tag)).length;
  }
</script>

<div class="sidebox" transition:slide={side}>
  <aside class="list" style="width: {width}px" aria-label={title}>
    <SplitHandle edge="right" {width} {min} {max} {reset} label={widthLabel} onchange={onwidth} />
    <!-- As the tracks column: the title, "+", then folding the column away, over a rule. -->
    <div class="head">
      <span class="mlab">{title}</span>
      <span class="grow"></span>
      <button class="btn newbtn" class:on={creating} title={newLabel} aria-expanded={creating} onclick={() => (creating = !creating)}>{newButton}</button>
      <button class="x" onclick={onclose} aria-label={t("tracks.close")} title={t("tracks.close")}>
        <Icon name="collapse" />
      </button>
    </div>
    {#if creating}
      <form class="new" onsubmit={create}>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          type="text"
          bind:value={draft}
          placeholder={newPlaceholder}
          aria-label={newLabel}
          {maxlength}
          autofocus
          onkeydown={(e) => {
            if (e.key === "Escape") {
              creating = false;
              draft = "";
            }
          }}
        />
        <button class="btn" type="submit" title={newLabel} aria-label={newLabel}>+</button>
      </form>
    {/if}
    {#if note}
      <p class="note" role="status">{note}</p>
    {/if}
    <ListSearch bind:query placeholder={searchPlaceholder} {picked} ontoggle={toggleTag} onclear={() => onpick_tags([])} {tagCount} />
    <div class="rows">
      {#each [...pinned, ...shown] as item (item.id)}
        <!-- Like a track's row: tags and the small fact above, the name below, its colour on the left. -->
        <div
          class="row"
          class:on={item.id === current}
          class:menued={menued === item.id}
          style="border-left-color: {item.color || 'transparent'}"
          oncontextmenu={(e) => {
            e.preventDefault();
            if (item.menu !== false) onmenu(item.id, e.clientX, e.clientY);
          }}
          role="presentation"
        >
          <button class="pick" onclick={() => onpick(item.id)}>
            <span class="top">
              <span class="taglist" title={item.tags.join(", ")}>
                {#each item.tags as tag (tag)}
                  <span class="mono chip" style="color: {store.tagColor(tag)}">{tag}</span>
                {/each}
              </span>
              <span class="mono when" title={item.metaTitle ?? ""}>{item.meta}</span>
            </span>
            <span class="name">{item.name}</span>
          </button>
        </div>
      {/each}
      {#if !items.length}
        <p class="none">{empty}</p>
      {:else if filtering && !shown.length}
        <!-- Nothing the search or filter lets through: said as the tracks column says it. -->
        <div class="mono empty">{t("tracks.none")}</div>
      {/if}
    </div>
  </aside>
</div>

<style>
  .sidebox {
    flex-shrink: 0;
    display: flex;
    min-height: 0;
  }

  .list {
    position: relative;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
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

  /* The head's "+", as the tracks column's "+ TRACK". */
  .newbtn {
    height: 26px;
    padding: 0 9px;
  }

  .newbtn.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .empty {
    padding: 18px 16px;
    font-size: 11px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .x {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .x:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .new {
    display: flex;
    gap: 6px;
    padding: 10px 12px 0;
  }

  .new input {
    flex: 1;
    min-width: 0;
    height: 30px;
    padding: 0 9px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-size: 12px;
  }

  .new input:focus {
    border-color: var(--acc);
  }

  .new .btn {
    width: 30px;
    height: 30px;
    padding: 0;
    font-size: 15px;
  }

  .note {
    margin: 8px 12px 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--warn);
  }

  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  /* Two lines, as a track's row: tags and the small fact above, the name
     below. The left bar is the item's colour, only when one is chosen; the
     open one is told by its shade. */
  .row {
    min-height: 52px;
    display: flex;
    border-left: 2px solid transparent;
  }

  .row:hover,
  .row.on,
  .row.menued {
    background: var(--sel);
  }

  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 3px;
    padding: 8px 16px 8px 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    line-height: 1.3;
  }

  .taglist {
    flex: 1;
    min-width: 0;
    font-size: 9px;
    line-height: 1.3;
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

  .row.on .pick {
    color: var(--hi);
  }

  .name {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .when {
    flex-shrink: 0;
    font-size: 9px;
    letter-spacing: 0.04em;
    color: var(--lab);
  }

  .none {
    margin: 12px 16px;
    font-size: 12px;
    color: var(--lab);
  }
</style>
