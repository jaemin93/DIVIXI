<script lang="ts">
  import { TRACK_COLORS } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The right-click menu of an artifact in its list (a design; knowledge
   * later): rename, tags, colour, delete. The artifacts' own menu, apart
   * from the tracks column's: a fixed sheet at the pointer, kept inside the
   * window, closed by a click elsewhere or Escape.
   */
  let {
    x,
    y,
    name,
    color,
    deleteNote,
    busy = false,
    busyNote = "",
    onclose,
    onrename,
    ontags,
    oncolor,
    onexport,
    exportLabel = "",
    exportTitle = "",
    ondelete,
  }: {
    x: number;
    y: number;
    name: string;
    color: string;
    deleteNote: string;
    /** Deleting is refused while this is set. */
    busy?: boolean;
    busyNote?: string;
    onclose: () => void;
    onrename: (name: string) => void | Promise<void>;
    /** Without it (a library has no tags), the menu has no Tags item. */
    ontags?: () => void;
    /** Without it (a library has no colour), the menu has no colours. */
    oncolor?: (color: string) => void | Promise<void>;
    /** Without it (a library is not written out), the menu has no Export item. */
    onexport?: () => void;
    exportLabel?: string;
    exportTitle?: string;
    /** Resolves to why it was refused, or "" when it is gone. */
    ondelete: () => Promise<string>;
  } = $props();

  let mode = $state<"" | "rename" | "confirm">("");
  let draft = $state("");
  let deleteError = $state("");
  let el = $state<HTMLDivElement>();
  let input = $state<HTMLInputElement>();
  // svelte-ignore state_referenced_locally
  let pos = $state({ x, y });

  /** Keep the sheet inside the window once it has a size. */
  $effect(() => {
    if (!el) return;
    void mode;
    const r = el.getBoundingClientRect();
    const nx = Math.max(8, Math.min(x, window.innerWidth - r.width - 8));
    const ny = Math.max(8, Math.min(y, window.innerHeight - r.height - 8));
    if (nx !== pos.x || ny !== pos.y) pos = { x: nx, y: ny };
  });

  $effect(() => {
    if (mode === "rename") input?.focus();
  });

  /** A click is inside when the sheet was on its path at dispatch, so a
   *  button the click itself replaced (Delete → its confirm step) counts. */
  function onDocClick(e: MouseEvent) {
    if (el && !e.composedPath().includes(el)) onclose();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }

  function startRename() {
    draft = name;
    mode = "rename";
  }

  async function commit(e: Event) {
    e.preventDefault();
    if (draft.trim() && draft.trim() !== name) await onrename(draft.trim());
    onclose();
  }

  async function pick(c: string) {
    await oncolor?.(c);
    onclose();
  }

  async function remove() {
    deleteError = "";
    const refused = await ondelete();
    if (refused) deleteError = refused;
    else onclose();
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={onKey} />

<div class="menu" bind:this={el} style="left: {pos.x}px; top: {pos.y}px" role="menu" aria-label={name}>
  {#if mode === "rename"}
    <form class="inline" onsubmit={commit}>
      <span class="mlab-sm">{t("track.rename")}</span>
      <input type="text" bind:this={input} bind:value={draft} maxlength="80" />
      <div class="acts">
        <span class="grow"></span>
        <button class="btn sm" type="button" onclick={() => (mode = "")}>{t("track.cancel")}</button>
        <button class="btn sm btn-acc" type="submit" disabled={!draft.trim()}>{t("newtrack.save")}</button>
      </div>
    </form>
  {:else}
    <button class="item" role="menuitem" onclick={startRename}>{t("track.rename")}</button>
    {#if ontags}
      <button class="item" role="menuitem" onclick={() => { ontags(); onclose(); }}>{t("track.tags")}</button>
    {/if}
    {#if onexport}
      <button class="item" role="menuitem" title={exportTitle} onclick={() => { onexport(); onclose(); }}>{exportLabel}</button>
    {/if}
    <div class="rule"></div>
    {#if oncolor}
      <div class="colors" role="group" aria-label={t("track.color")}>
        <button class="sw none" class:on={!color} title={t("track.noColor")} onclick={() => pick("")}></button>
        {#each TRACK_COLORS as c (c)}
          <button class="sw" class:on={color === c} style="background: {c}" title={c} onclick={() => pick(c)}></button>
        {/each}
      </div>
      <div class="rule"></div>
    {/if}
    {#if mode === "confirm"}
      <div class="mono note">{busy ? busyNote : deleteNote}</div>
      {#if deleteError}<div class="mono note err">{deleteError}</div>{/if}
      <div class="acts pad">
        <button class="btn sm" type="button" onclick={() => (mode = "")}>{t("track.cancel")}</button>
        <span class="grow"></span>
        <button class="btn sm danger" type="button" disabled={busy} onclick={remove}>{t("track.confirmDelete")}</button>
      </div>
    {:else}
      <button class="item danger" role="menuitem" onclick={() => (mode = "confirm")}>{t("track.delete")}</button>
    {/if}
  {/if}
</div>

<style>
  /* A fixed sheet at the pointer, hairline and square. */
  .menu {
    position: fixed;
    z-index: 40;
    min-width: 210px;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0;
  }

  .item {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    padding: 0 14px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 12px;
    color: var(--dim);
  }

  .item:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .item.danger {
    color: var(--acct);
  }

  .item:disabled {
    color: var(--lab);
    cursor: default;
  }

  .rule {
    height: 1px;
    background: var(--line);
    margin: 4px 0;
  }

  .colors {
    display: flex;
    gap: 7px;
    padding: 6px 14px;
  }

  .sw {
    width: 16px;
    height: 16px;
    border: 1px solid transparent;
    padding: 0;
  }

  .sw.none {
    background: transparent;
    border-color: var(--lines);
    background-image: linear-gradient(135deg, transparent 46%, var(--acct) 46%, var(--acct) 54%, transparent 54%);
  }

  .sw.on {
    outline: 1px solid var(--hi);
    outline-offset: 2px;
  }

  .note {
    padding: 6px 14px 2px;
    font-size: 10px;
    color: var(--dim);
    max-width: 260px;
  }

  .note.err {
    color: var(--acct);
  }

  .inline {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px 10px;
    width: 260px;
  }

  .inline input {
    height: 30px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
    padding: 0 9px;
  }

  .inline input:focus {
    border-color: var(--acc);
  }

  .acts {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .acts.pad {
    padding: 4px 12px 8px;
  }

  .grow {
    flex: 1;
  }

  .btn.sm {
    height: 24px;
    padding: 0 8px;
  }

  .btn.danger:not(:disabled) {
    color: var(--acct);
    border-color: var(--acct);
  }
</style>
