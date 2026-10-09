<script module lang="ts">
  /** One row of a list column. */
  export type ColumnItem = {
    id: string;
    name: string;
    tags: string[];
    color: string;
    /** The small fact on the right of the upper line (a design's time); none, nothing there. */
    meta?: string;
    metaTitle?: string;
  };

  /**
   * One way the "+" makes an item, when it has more than one: `name` opens
   * the name box; `file` picks a file (this PC's own dialog when `onfile` is
   * called with none; a browser's picker otherwise, with `accept`).
   */
  export type NewOption =
    | { id: string; label: string; kind: "name" }
    | { id: string; label: string; kind: "file"; accept: string; onfile: (file?: File) => void };
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
  import { phoneWidth } from "./phone";
  import { overWeb } from "./ipc.svelte";
  import { foldsOnPick } from "./uiMemory";

  /**
   * A page's list column, made like the tracks column: the title, a "+"
   * button and the fold button over a rule; the search row and its tag
   * filter (ListSearch, the tracks column's own); a row per item, tags and a
   * small fact above, the name below, its colour as the bar on the left, the
   * open one told by its shade. The "+" opens a name box under the head (a
   * design or a library needs a name only; a track opens its own form), or,
   * when there is more than one way to make one, a menu of them.
   * Right-clicking a row asks the page for its menu (rename, tags, colour,
   * delete), which the page draws. The designs page and the knowledge
   * page's libraries both use it, so the three columns read as one.
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
    options = [],
    note = "",
    items,
    picked,
    onpick_tags,
    newAsked = false,
    onnewasked,
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
    /** More than one way to make one: the "+" opens a menu of them. Without, it opens the name box. */
    options?: NewOption[];
    /** A line under the box: what went wrong with a name, say. */
    note?: string;
    items: ColumnItem[];
    /** The tags the column is narrowed to, kept as the tracks column keeps its filter. */
    picked: string[];
    onpick_tags: (tags: string[]) => void;
    /** The page asks for the name box (nothing to put a document in yet); told back once it is open. */
    newAsked?: boolean;
    onnewasked?: () => void;
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
  /** The "+" menu, when there are options. */
  let choosing = $state(false);
  let menuEl = $state<HTMLDivElement>();
  let fileInput = $state<HTMLInputElement>();
  /** The file option a browser's picker is open for. */
  let picking = $state<Extract<NewOption, { kind: "file" }> | null>(null);

  // Asked from the page: the name box, open (the column may have just been unfolded for it).
  $effect(() => {
    if (!newAsked) return;
    choosing = false;
    creating = true;
    onnewasked?.();
  });

  function plus() {
    // An open name box: the "+" puts it away, as it always did.
    if (creating) creating = false;
    else if (options.length) choosing = !choosing;
    else creating = !creating;
  }

  function choose(o: NewOption) {
    choosing = false;
    if (o.kind === "name") {
      creating = true;
    } else if (overWeb && fileInput) {
      // In the click itself: a browser opens its picker only for a person's press.
      picking = o;
      fileInput.accept = o.accept;
      fileInput.click();
    } else {
      creating = false;
      o.onfile();
    }
  }

  function onDocClick(e: MouseEvent) {
    if (choosing && menuEl && !e.composedPath().includes(menuEl)) choosing = false;
  }
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

  /** How many of this column's rows carry a tag. */
  function tagCount(tag: string): number {
    return items.filter((i) => i.tags.includes(tag)).length;
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={(e) => e.key === "Escape" && (choosing = false)} />

<div class="sidebox" transition:slide={side}>
  <aside class="list" style="width: {width}px" aria-label={title}>
    <SplitHandle edge="right" {width} {min} {max} {reset} label={widthLabel} onchange={onwidth} />
    <!-- As the tracks column: the title, "+", then folding the column away, over a rule. -->
    <div class="head">
      <span class="mlab">{title}</span>
      <span class="grow"></span>
      <div class="plus" bind:this={menuEl}>
        <button
          class="btn newbtn"
          class:on={creating || choosing}
          title={newLabel}
          aria-haspopup={options.length ? "menu" : undefined}
          aria-expanded={options.length ? choosing : creating}
          onclick={plus}>{newButton}</button
        >
        {#if choosing}
          <div class="newmenu" role="menu" aria-label={newLabel}>
            {#each options as o (o.id)}
              <button class="opt" role="menuitem" onclick={() => choose(o)}>{o.label}</button>
            {/each}
          </div>
        {/if}
      </div>
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
    {#if overWeb && options.some((o) => o.kind === "file")}
      <input
        class="pickfile"
        type="file"
        bind:this={fileInput}
        onchange={(e) => {
          const f = e.currentTarget.files?.[0];
          e.currentTarget.value = "";
          if (f) picking?.onfile(f);
          picking = null;
        }}
      />
    {/if}
    {#if note}
      <p class="note" role="status">{note}</p>
    {/if}
    <ListSearch bind:query placeholder={searchPlaceholder} {picked} ontoggle={toggleTag} onclear={() => onpick_tags([])} {tagCount} />
    <div class="rows">
      {#each shown as item (item.id)}
        <!-- A track's row without its dot and fold toggle: tags and the small fact above, the name below, its colour on the left. -->
        <div
          class="row"
          class:on={item.id === current}
          class:menued={menued === item.id}
          style="border-left-color: {item.color || 'transparent'}"
          oncontextmenu={(e) => {
            e.preventDefault();
            onmenu(item.id, e.clientX, e.clientY);
          }}
          role="presentation"
        >
          <button
            class="pick"
            onclick={() => {
              onpick(item.id);
              // A phone: done with once a row is picked, as the track list is.
              if (foldsOnPick(overWeb, phoneWidth(innerWidth))) onclose();
            }}
          >
            <span class="top">
              <span class="taglist" title={item.tags.join(", ")}>
                {#each item.tags as tag (tag)}
                  <span class="mono chip" style="color: {store.tagColor(tag)}">{tag}</span>
                {/each}
              </span>
              {#if item.meta}<span class="mono when" title={item.metaTitle ?? ""}>{item.meta}</span>{/if}
            </span>
            <span class="main"><span class="name">{item.name}</span></span>
          </button>
        </div>
      {/each}
      {#if !items.length}
        <div class="mono empty">{empty}</div>
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

  /* The "+" and, with options, its menu under it. */
  .plus {
    position: relative;
  }

  .newmenu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 30;
    min-width: 150px;
    padding: 4px 0;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  .opt {
    display: block;
    width: 100%;
    height: 34px;
    padding: 0 14px;
    background: none;
    border: 0;
    text-align: left;
    font-size: 13px;
    color: var(--dim);
  }

  .opt:hover,
  .opt:focus-visible {
    color: var(--hi);
    background: var(--sel);
  }

  .pickfile {
    display: none;
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

  /* The tracks column's list (TrackList .list), its rows' measures and
     colours, so the three columns read as one. */
  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0 12px;
  }

  /* Two lines, as a track's row: tags and the small fact above, the name
     below on a 36px line. The left bar is the item's colour, only when one
     is chosen; the open one is told by its shade and brighter text, as a
     track is. */
  .row {
    min-height: 52px;
    display: flex;
    align-items: flex-end;
    border-left: 2px solid transparent;
    color: var(--dim);
  }

  .row:hover,
  .row.on,
  .row.menued {
    color: var(--hi);
    background: var(--sel);
  }

  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 0 16px 0 14px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 13px;
    color: inherit;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0 0 0;
    margin-bottom: -4px;
    overflow: hidden;
    white-space: nowrap;
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

  .main {
    height: 36px;
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .when {
    flex-shrink: 0;
    font-size: 9px;
    letter-spacing: 0.04em;
    color: var(--lab);
  }
</style>
