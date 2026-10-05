<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import SplitHandle from "./SplitHandle.svelte";
  import Icon from "./Icon.svelte";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import { store } from "./store.svelte";
  import { kb } from "./knowledge.svelte";
  import { t } from "./i18n.svelte";
  import { MAX_NAME, cleanName, deleteBlock, nameProblem, type DeleteBlock, type KLibrary, type NameProblem } from "./libraries";

  /**
   * The knowledge page's libraries, as the designs column lists designs: all
   * of them on top, then each library with its number of documents, and a
   * box to make one. A library is renamed and deleted from its right-click
   * menu, as a design is (or in place: double-click, the pencil, the ×):
   * General is never deleted, nor one with documents until they are moved or
   * removed, which the menu or the column says.
   */

  /** Slides like the tracks and designs columns; not for those who asked for less motion. */
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const side = { axis: "x" as const, duration: reduced ? 0 : 200, easing: cubicOut };

  let newName = $state("");
  /** The last thing to say about a name or a delete; cleared by the next try. */
  let note = $state("");
  let renaming = $state<number | null>(null);
  let draft = $state("");
  let confirmDelete = $state<number | null>(null);
  /** The right-click menu: the designs list's, without tags and colours. */
  let menu = $state<{ id: number; x: number; y: number } | null>(null);
  const menuLibrary = $derived.by(() => {
    const m = menu;
    return m ? kb.libraries.find((l) => l.id === m.id) : undefined;
  });

  /** From the menu: the same rules as in place; what is wrong is said in the column. */
  async function renameTo(l: KLibrary, raw: string) {
    const name = cleanName(raw);
    if (!name || name === kb.libraryName(l)) return;
    const p = nameProblem(name, kb.libraries, l.id);
    note = p ? nameNote(p) : await kb.renameLibrary(l.id, name);
  }

  /** From the menu: why not (shown in the menu), or "" once it is gone. */
  async function deleteFromMenu(l: KLibrary): Promise<string> {
    const block = deleteBlock(l);
    if (block) return blockNote(block, l);
    return kb.deleteLibrary(l.id);
  }

  function nameNote(p: NameProblem): string {
    if (p === "long") return t("kb.lib.nameLong", { n: MAX_NAME });
    if (p === "taken") return t("kb.lib.nameTaken");
    return "";
  }

  function blockNote(b: DeleteBlock, l: KLibrary): string {
    if (b === "general") return t("kb.lib.generalKept");
    if (b === "notEmpty") return t("kb.lib.notEmpty", { n: l.sources });
    return "";
  }

  async function create(e: Event) {
    e.preventDefault();
    const p = nameProblem(newName, kb.libraries);
    if (p === "empty") return;
    if (p) {
      note = nameNote(p);
      return;
    }
    const why = await kb.createLibrary(cleanName(newName));
    note = why;
    if (!why) newName = "";
  }

  function startRename(l: KLibrary) {
    confirmDelete = null;
    note = "";
    draft = kb.libraryName(l);
    renaming = l.id;
  }

  async function commitRename(l: KLibrary) {
    if (renaming !== l.id) return;
    renaming = null;
    const name = cleanName(draft);
    if (!name || name === kb.libraryName(l)) return;
    const p = nameProblem(name, kb.libraries, l.id);
    if (p) {
      note = nameNote(p);
      return;
    }
    note = await kb.renameLibrary(l.id, name);
  }

  async function askDelete(l: KLibrary) {
    note = "";
    const block = deleteBlock(l);
    if (block) {
      note = blockNote(block, l);
      return;
    }
    if (confirmDelete !== l.id) {
      confirmDelete = l.id;
      return;
    }
    confirmDelete = null;
    note = await kb.deleteLibrary(l.id);
  }
</script>

<div class="sidebox" transition:slide={side}>
  <aside class="list" style="width: {store.kbListWidth}px" aria-label={t("kb.lib.title")}>
    <SplitHandle edge="right" width={store.kbListWidth} min={180} max={420} reset={220} label={t("kb.lib.listWidth")} onchange={(px, persist) => store.setKbListWidth(px, persist)} />
    <!-- As the designs column: the title, then folding the column away. -->
    <div class="head">
      <span class="mlab">{t("kb.lib.title")}</span>
      <span class="grow"></span>
      <button class="x" onclick={() => store.setKbList(false)} aria-label={t("tracks.close")} title={t("tracks.close")}>
        <Icon name="collapse" />
      </button>
    </div>
    <form class="new" onsubmit={create}>
      <input type="text" bind:value={newName} placeholder={t("kb.lib.newPh")} aria-label={t("kb.lib.new")} maxlength={MAX_NAME + 20} />
      <button class="btn" type="submit" title={t("kb.lib.new")} aria-label={t("kb.lib.new")}>+</button>
    </form>
    {#if note}
      <p class="lnote" role="status">{note}</p>
    {/if}
    <div class="rows" role="listbox" aria-label={t("kb.lib.title")}>
      <div class="row" class:on={kb.library === null}>
        <button class="pick" role="option" aria-selected={kb.library === null} onclick={() => kb.showLibrary(null)}>
          <span class="name">{t("kb.lib.all")}</span>
          <span class="mono count">{kb.sources.length}</span>
        </button>
      </div>
      <div class="rule" aria-hidden="true"></div>
      {#each kb.libraries as l (l.id)}
        <div
          class="row"
          class:on={kb.library === l.id}
          class:asking={confirmDelete === l.id}
          class:menued={menu?.id === l.id}
          oncontextmenu={(e) => {
            e.preventDefault();
            confirmDelete = null;
            note = "";
            menu = { id: l.id, x: e.clientX, y: e.clientY };
          }}
          role="presentation"
        >
          {#if renaming === l.id}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="rename"
              bind:value={draft}
              aria-label={t("kb.lib.rename")}
              maxlength={MAX_NAME + 20}
              autofocus
              onblur={() => commitRename(l)}
              onkeydown={(e) => {
                if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
                else if (e.key === "Escape") renaming = null;
              }}
            />
          {:else}
            <button
              class="pick"
              role="option"
              aria-selected={kb.library === l.id}
              title={t("kb.lib.renameHint")}
              onclick={() => kb.showLibrary(l.id)}
              ondblclick={() => startRename(l)}
            >
              <span class="name">{kb.libraryName(l)}</span>
              <span class="mono count">{l.sources}</span>
            </button>
            {#if confirmDelete === l.id}
              <button class="btn sm danger" onclick={() => askDelete(l)}>{t("kb.lib.deleteConfirm")}</button>
              <button class="tool" onclick={() => (confirmDelete = null)} aria-label={t("kb.lib.cancel")} title={t("kb.lib.cancel")}>↩</button>
            {:else}
              <span class="tools">
                <button class="tool" onclick={() => startRename(l)} aria-label={t("kb.lib.rename")} title={t("kb.lib.rename")}>✎</button>
                <button class="tool del" onclick={() => askDelete(l)} aria-label={t("kb.lib.delete")} title={t("kb.lib.delete")}>×</button>
              </span>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  </aside>
</div>

{#if menu && menuLibrary}
  {@const ml = menuLibrary}
  <ArtifactMenu
    x={menu.x}
    y={menu.y}
    name={kb.libraryName(ml)}
    color=""
    deleteNote={deleteBlock(ml) ? blockNote(deleteBlock(ml), ml) : t("kb.lib.deleteNote")}
    onclose={() => (menu = null)}
    onrename={(name) => renameTo(ml, name)}
    ondelete={() => deleteFromMenu(ml)}
  />
{/if}

<style>
  /* As the designs column (DesignView .list), so the two read as one family. */
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

  .new input,
  .rename {
    flex: 1;
    min-width: 0;
    height: 30px;
    padding: 0 9px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-size: 12px;
  }

  .new input:focus,
  .rename {
    border-color: var(--acc);
  }

  .rename {
    margin: 5px 10px 5px 12px;
  }

  .new .btn {
    width: 30px;
    height: 30px;
    padding: 0;
    font-size: 15px;
  }

  .lnote {
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

  .rule {
    height: 1px;
    margin: 4px 16px;
    background: var(--line);
  }

  /* One line each: the name, its documents; the open one told by its shade, as a design's row. */
  .row {
    min-height: 40px;
    display: flex;
    align-items: center;
    border-left: 2px solid transparent;
  }

  .row:hover,
  .row.on,
  .row.asking,
  .row.menued {
    background: var(--sel);
  }

  .row.on {
    border-left-color: var(--acc);
  }

  .pick {
    flex: 1;
    min-width: 0;
    align-self: stretch;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .row.on .pick {
    color: var(--hi);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .count {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--lab);
  }

  /* Rename and delete, on the row under the pointer (or the keyboard). */
  .tools {
    display: flex;
    padding-right: 6px;
    opacity: 0;
  }

  .row:hover .tools,
  .row:focus-within .tools {
    opacity: 1;
  }

  .tool {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 12px;
  }

  .tool:hover {
    color: var(--hi);
  }

  .tool.del:hover {
    color: var(--deltx);
  }

  .btn.sm {
    height: 24px;
    padding: 0 8px;
    font-size: 11px;
    white-space: nowrap;
  }

  .btn.danger {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .row.asking .tool {
    margin-right: 6px;
  }
</style>
