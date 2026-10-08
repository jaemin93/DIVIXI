<script lang="ts">
  import ItemColumn from "./ItemColumn.svelte";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import { store } from "./store.svelte";
  import { kb } from "./knowledge.svelte";
  import { t } from "./i18n.svelte";
  import { MAX_NAME, cleanName, deleteBlock, libraryRow, nameProblem, type DeleteBlock, type KLibrary, type NameProblem } from "./libraries";

  /**
   * The knowledge page's libraries, in the same column as the designs
   * (ItemColumn): each library, its tags above its name, its colour as the
   * bar; one is always the one shown (General at first). A library is renamed, tagged, coloured and deleted from its
   * right-click menu, as a design is (ArtifactMenu); General is never
   * deleted, nor a library with documents, which the menu says.
   */

  /** What to say about a name; cleared by the next try. */
  let note = $state("");
  let menu = $state<{ id: number; x: number; y: number } | null>(null);
  const menuLibrary = $derived.by(() => {
    const m = menu;
    return m ? kb.libraries.find((l) => l.id === m.id) : undefined;
  });

  const rows = $derived(kb.libraries.map((l) => libraryRow(l, kb.libraryName(l))));

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

  /** From the column's box. Resolves to why not (the box keeps its text). */
  async function create(raw: string): Promise<string> {
    const p = nameProblem(raw, kb.libraries);
    if (p === "empty") return "empty";
    if (p) return (note = nameNote(p));
    return (note = await kb.createLibrary(cleanName(raw)));
  }

  /** From the menu's rename: the same rules as the box; what is wrong is said under it. */
  async function renameTo(l: KLibrary, raw: string) {
    const name = cleanName(raw);
    if (!name || name === kb.libraryName(l)) return;
    const p = nameProblem(name, kb.libraries, l.id);
    note = p ? nameNote(p) : await kb.renameLibrary(l.id, name);
  }
</script>

<ItemColumn
  title={t("kb.lib.title")}
  width={store.kbListWidth}
  widthLabel={t("kb.lib.listWidth")}
  onwidth={(px, persist) => store.setKbListWidth(px, persist)}
  onclose={() => store.setKbList(false)}
  newLabel={t("kb.lib.new")}
  newButton={t("kb.lib.newButton")}
  newPlaceholder={t("kb.lib.newPh")}
  searchPlaceholder={t("kb.lib.search")}
  maxlength={MAX_NAME}
  oncreate={create}
  {note}
  items={rows}
  picked={store.kbTags}
  onpick_tags={(tags) => store.setKbTags(tags)}
  current={kb.library === null ? null : String(kb.library)}
  menued={menu ? String(menu.id) : null}
  onpick={(id) => kb.showLibrary(Number(id))}
  onmenu={(id, x, y) => {
    note = "";
    menu = { id: Number(id), x, y };
  }}
  empty={t("kb.lib.none")}
/>

{#if menu && menuLibrary}
  {@const ml = menuLibrary}
  {@const block = deleteBlock(ml)}
  <!-- The designs' menu, as it is: rename, tags, colour, delete (refused, and said, for General and a library with documents). -->
  <ArtifactMenu
    x={menu.x}
    y={menu.y}
    name={kb.libraryName(ml)}
    color={ml.color}
    busy={!!block}
    busyNote={blockNote(block, ml)}
    deleteNote={t("kb.lib.deleteNote")}
    onclose={() => (menu = null)}
    onrename={(name) => renameTo(ml, name)}
    ontags={() => (store.tagDialog = `lib:${ml.id}`)}
    oncolor={(color) => kb.updateLibrary(ml.id, { color })}
    ondelete={() => kb.deleteLibrary(ml.id)}
  />
{/if}
