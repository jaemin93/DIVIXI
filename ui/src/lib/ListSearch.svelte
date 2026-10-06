<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /**
   * A list column's search row, after the tracks column's (and Kiro Crew's):
   * a box that narrows the list as you type, and a filter button whose menu
   * picks tags (AND-ed) from the app's one tag vocabulary. The tracks column
   * adds its own sections above the tags (running, active, recent, sort,
   * fold); the designs and libraries columns have the tags alone. The menu
   * closes on a click elsewhere or Esc. The menu's look lives here, for all
   * three columns.
   */
  let {
    query = $bindable(""),
    placeholder,
    picked,
    ontoggle,
    onclear,
    tagCount,
    extraOn = false,
    title = t("tracks.filterTitle"),
    sections,
  }: {
    query?: string;
    placeholder: string;
    /** The tags picked to narrow the list. */
    picked: string[];
    ontoggle: (tag: string) => void;
    /** Take every filter off (the column's own, and the tags). */
    onclear: () => void;
    /** How many of this column's things carry a tag. */
    tagCount: (tag: string) => number;
    /** A filter of the column's own is on (the tracks column's running, active, recent). */
    extraOn?: boolean;
    /** What the button and the menu are called. */
    title?: string;
    /** The column's own sections, above the tags, each ending with a rule. */
    sections?: Snippet;
  } = $props();

  let open = $state(false);
  let box = $state<HTMLDivElement>();
  const active = $derived(extraOn || picked.length > 0);
  const label = $derived(active ? `${title} · ${t("tracks.filterOn")}` : title);

  function onDocClick(e: MouseEvent) {
    if (open && box && !e.composedPath().includes(box)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") open = false;
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={onKey} />

<div class="searchrow" bind:this={box}>
  <input class="search" type="text" bind:value={query} {placeholder} aria-label={placeholder} />
  <button class="fbtn" class:on={active || open} class:set={active} onclick={() => (open = !open)} title={label} aria-label={label} aria-haspopup="menu" aria-expanded={open}>
    <Icon name="filter" size={14} />
  </button>

  {#if open}
    <!-- The filter menu, after Kiro Crew's: the column's own sections, then which tags. -->
    <div class="fmenu" role="menu" aria-label={title}>
      {#if sections}
        <div class="fhead fheadrow">
          <span class="mlab-sm">{t("tracks.filterTitle")}</span>
          <span class="grow"></span>
          {#if active}<button class="mono fclear" onclick={onclear}>{t("tracks.clearShort")}</button>{/if}
        </div>
        {@render sections()}
      {/if}
      <div class="fhead fheadrow">
        <span class="mlab-sm">{t("tracks.tagsTitle")}</span>
        {#if store.tagPool.length > 1}<span class="fhint">{t("tracks.tagsAll")}</span>{/if}
        {#if !sections}
          <span class="grow"></span>
          {#if active}<button class="mono fclear" onclick={onclear}>{t("tracks.clearShort")}</button>{/if}
        {/if}
      </div>
      <div class="tags">
        {#each store.tagPool as tag (tag.name)}
          {@const on = picked.includes(tag.name)}
          <!-- A filter that is on says so three ways: the accent bar, bold text and a ✓. -->
          <button class="fitem narrow" class:on role="menuitemcheckbox" aria-checked={on} onclick={() => ontoggle(tag.name)}>
            <span class="box" class:on style="border-color: {tag.color}; background: {on ? tag.color : 'transparent'}"></span>
            <span class="mono">{tag.name}</span>
            <span class="grow"></span>
            {#if on}<span class="mono check" aria-hidden="true">✓</span>{/if}
            <span class="mono count">{tagCount(tag.name)}</span>
          </button>
        {/each}
        {#if store.tagPool.length === 0}
          <div class="fitem static dim">{t("tracks.noTagsYet")}</div>
        {/if}
      </div>
      {#if active}
        <div class="rule"></div>
        <button class="fitem" onclick={onclear}>{t("tracks.clear")}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .searchrow {
    position: relative;
    display: flex;
    gap: 6px;
    margin: 10px 12px 6px;
  }

  .search {
    flex: 1;
    min-width: 0;
    height: 32px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
    padding: 0 10px;
  }

  .search:focus {
    border-color: var(--acc);
  }

  .fbtn {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .fbtn:hover,
  .fbtn.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  /* A filter is on: the button is tinted. */
  .fbtn.set {
    background: var(--accbg);
  }

  /* The filter menu hangs under the search row. Its rows are styled here for
     the column's own sections too (passed in, so :global within the menu). */
  .fmenu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    z-index: 30;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0 6px;
  }

  .tags {
    max-height: 30vh;
    overflow-y: auto;
  }

  .grow {
    flex: 1;
  }

  .fmenu :global(.rule) {
    height: 1px;
    background: var(--line);
    margin: 4px 0;
  }

  /* A row with a submenu: hovering opens it to the right, as Kiro does. */
  .fmenu :global(.fitem.sub) {
    position: relative;
  }

  .fmenu :global(.arrow) {
    color: var(--lab);
    font-size: 12px;
  }

  .fmenu :global(.submenu) {
    display: none;
    position: absolute;
    top: -5px;
    left: calc(100% - 6px);
    min-width: 200px;
    z-index: 31;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0 6px;
    color: var(--dim);
  }

  .fmenu :global(.fitem.sub:hover),
  .fmenu :global(.fitem.sub:focus-within) {
    color: var(--hi);
    background: var(--sel);
  }

  .fmenu :global(.fitem.sub:hover > .submenu),
  .fmenu :global(.fitem.sub:focus-within > .submenu) {
    display: block;
  }

  .fmenu :global(.subhead) {
    padding: 8px 14px 6px;
    font-size: 11px;
    color: var(--lab);
    white-space: nowrap;
  }

  .fmenu :global(.fhead) {
    padding: 10px 14px 4px;
  }

  .fmenu :global(.fheadrow) {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .fhint {
    font-size: 10px;
    color: var(--lab);
  }

  .fclear {
    padding: 0;
    background: transparent;
    border: 0;
    font-size: 10px;
    color: var(--lab);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .fclear:hover {
    color: var(--hi);
  }

  .fmenu :global(.fitem) {
    width: 100%;
    min-height: 30px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 4px 14px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 12px;
    color: var(--dim);
  }

  .fmenu :global(.fitem:not(.static):hover) {
    color: var(--hi);
    background: var(--sel);
  }

  .fmenu :global(.fitem.on) {
    color: var(--hi);
  }

  /* A filter that narrows the list, when on: the instance menu's accent bar
     on the left, a tinted row and bold text, besides the ✓ at the right. */
  .fmenu :global(.fitem.narrow) {
    border-left: 2px solid transparent;
    padding-left: 12px;
  }

  .fmenu :global(.fitem.narrow.on) {
    border-left-color: var(--acc);
    background: var(--accbg);
    font-weight: 600;
  }

  .fmenu :global(.fitem.narrow.on:hover) {
    background: var(--sel);
  }

  .fmenu :global(.fitem.dim) {
    color: var(--lab);
    font-size: 11px;
  }

  .fmenu :global(.fitem .check) {
    color: var(--acct);
    font-size: 11px;
  }

  .fmenu :global(.fitem .dot) {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .box {
    width: 10px;
    height: 10px;
    border: 1px solid;
    flex-shrink: 0;
  }

  .count {
    font-size: 10px;
    color: var(--lab);
  }
</style>
