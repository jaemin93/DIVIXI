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
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /**
   * A page's list column, made like the tracks column: the title and the
   * fold button, a box to make one, and a row per item, tags and a small
   * fact above, the name below, its colour as the bar on the left; the open
   * one told by its shade. Right-clicking a row asks the page for its menu
   * (rename, tags, colour, delete), which the page draws. The designs page
   * and the knowledge page's libraries both use it, so the two read as one.
   */

  let {
    title,
    width,
    min = 200,
    max = 480,
    reset = 240,
    widthLabel,
    onwidth,
    onclose,
    newLabel,
    newPlaceholder,
    maxlength = 80,
    oncreate,
    note = "",
    items,
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
    newPlaceholder: string;
    maxlength?: number;
    /** Make one from the box. Resolves to why not (the box keeps its text), or nothing. */
    oncreate: (name: string) => Promise<string | void> | string | void;
    /** A line under the box: what went wrong with a name, say. */
    note?: string;
    items: ColumnItem[];
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

  async function create(e: Event) {
    e.preventDefault();
    const why = await oncreate(draft);
    if (!why) draft = "";
  }
</script>

<div class="sidebox" transition:slide={side}>
  <aside class="list" style="width: {width}px" aria-label={title}>
    <SplitHandle edge="right" {width} {min} {max} {reset} label={widthLabel} onchange={onwidth} />
    <!-- As the tracks column: the title, then folding the column away. -->
    <div class="head">
      <span class="mlab">{title}</span>
      <span class="grow"></span>
      <button class="x" onclick={onclose} aria-label={t("tracks.close")} title={t("tracks.close")}>
        <Icon name="collapse" />
      </button>
    </div>
    <form class="new" onsubmit={create}>
      <input type="text" bind:value={draft} placeholder={newPlaceholder} aria-label={newLabel} {maxlength} />
      <button class="btn" type="submit" title={newLabel} aria-label={newLabel}>+</button>
    </form>
    {#if note}
      <p class="note" role="status">{note}</p>
    {/if}
    <div class="rows">
      {#each items as item (item.id)}
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
    padding: 0 12px 10px;
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
    margin: -2px 12px 10px;
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
