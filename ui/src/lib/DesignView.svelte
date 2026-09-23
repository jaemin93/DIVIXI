<script lang="ts">
  import { store } from "./store.svelte";
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import SplitHandle from "./SplitHandle.svelte";
  import Icon from "./Icon.svelte";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import { whenFull } from "./time";
  import Board from "./Board.svelte";
  import Timeline from "./Timeline.svelte";
  import Composer from "./Composer.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * A design (an artifact): the list of designs, the board, and beside it
   * the conversation with its agent — the same timeline and composer as a track's
   * conductor (files, agent and model, context, stop). The header counts
   * what the design has become (goals, constraints, open questions); a
   * design is attached to a track's conductor from the track's composer.
   */
  let renaming = $state(false);

  /** The list slides like the tracks column; not for those who asked for less motion. */
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const side = { axis: "x" as const, duration: reduced ? 0 : 200, easing: cubicOut };
  let titleDraft = $state("");
  let newTitle = $state("");
  /** The right-click menu: the same as a track's, less its session. */
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  const menuArtifact = $derived.by(() => {
    const m = menu;
    return m ? store.designs.find((x) => x.id === m.id) : undefined;
  });

  const d = $derived(store.currentArtifact);
  const notes = $derived(store.designDoc.nodes.filter((n) => n.kind === "note"));
  const count = (tag: string) => notes.filter((n) => n.tag === tag && n.text.trim()).length;
  const goals = $derived(count("goal"));
  const constraints = $derived(count("constraint"));
  const questions = $derived(count("question"));

  async function create(e: Event) {
    e.preventDefault();
    await store.createDesign(newTitle.trim() || t("design.untitled"));
    newTitle = "";
  }

  function rename() {
    if (!d) return;
    titleDraft = d.title;
    renaming = true;
  }

  async function commitRename() {
    renaming = false;
    if (d && titleDraft.trim() && titleDraft.trim() !== d.title) await store.updateArtifact(d.id, { title: titleDraft.trim() });
  }

</script>

<div class="designs">
  <!-- Every design, most recently touched first; folds like the tracks column. -->
  {#if store.designListOpen}
  <div class="sidebox" transition:slide={side}>
  <aside class="list">
    <!-- As the tracks column: the title, then folding the column away. -->
    <div class="head">
      <span class="mlab">{t("design.title")}</span>
      <span class="grow"></span>
      <button class="x" onclick={() => store.setDesignList(false)} aria-label={t("tracks.close")} title={t("tracks.close")}>
        <Icon name="collapse" />
      </button>
    </div>
    <form class="new" onsubmit={create}>
      <input type="text" bind:value={newTitle} placeholder={t("design.newPh")} aria-label={t("design.new")} maxlength="80" />
      <button class="btn" type="submit" title={t("design.new")} aria-label={t("design.new")}>+</button>
    </form>
    <div class="rows">
      {#each store.designs as item (item.id)}
        <!-- Like a track's row: tags and time above, the name below, its colour on the left. -->
        <div
          class="row"
          class:on={item.id === store.artifact}
          class:menued={menu?.id === item.id}
          style="border-left-color: {item.color || 'transparent'}"
          oncontextmenu={(e) => {
            e.preventDefault();
            menu = { id: item.id, x: e.clientX, y: e.clientY };
          }}
          role="presentation"
        >
          <button class="pick" onclick={() => store.openArtifact(item.id)}>
            <span class="top">
              <span class="taglist" title={item.tags.join(", ")}>
                {#each item.tags as tag (tag)}
                  <span class="mono chip" style="color: {store.tagColor(tag)}">{tag}</span>
                {/each}
              </span>
              <span class="mono when" title={whenFull(item.updated_at, store.lang)}>{whenLabel(item.updated_at, store.now, store.lang)}</span>
            </span>
            <span class="name">{item.title}</span>
          </button>
        </div>
      {/each}
      {#if !store.designs.length}
        <p class="none">{t("design.none")}</p>
      {/if}
    </div>
  </aside>
  </div>
  {/if}

  {#if menu && menuArtifact}
    {@const md = menuArtifact}
    <ArtifactMenu
      x={menu.x}
      y={menu.y}
      name={md.title}
      color={md.color}
      busy={store.artifact === md.id && store.artifactBusy}
      busyNote={t("design.busyNote")}
      deleteNote={t("design.deleteNote")}
      onclose={() => (menu = null)}
      onrename={(title) => store.updateArtifact(md.id, { title })}
      ontags={() => (store.tagDialog = md.id)}
      oncolor={(color) => store.updateArtifact(md.id, { color })}
      ondelete={() => store.deleteArtifact(md.id)}
    />
  {/if}

  {#if d}
    <section class="main">
      <header>
        <div class="mlab">DESIGN / {d.id}</div>
        <div class="titlerow">
          {#if renaming}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="serif rename"
              bind:value={titleDraft}
              onblur={commitRename}
              onkeydown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
              autofocus
            />
          {:else}
            <button class="serif title" ondblclick={rename} title={t("design.renameHint")}>{d.title}</button>
          {/if}
          <span class="grow"></span>
          <!-- What the design has become. -->
          <div class="meter mono" title={t("design.meterHint")}>
            <span><b style="color: #46c46a">{goals}</b> {t("design.tag.goal")}</span>
            <span><b style="color: var(--acct)">{constraints}</b> {t("design.tag.constraint")}</span>
            <span><b style="color: var(--warn)">{questions}</b> {t("design.tag.question")}</span>
          </div>
        </div>
      </header>
      <div class="work">
        <div class="boardwrap">
          <Board bind:selected={store.designSelected} />
          {#if store.designDoc.changes.length}
            <!-- The agent's board changes, all at once; one by one on the items. -->
            <div class="review">
              <span class="mono">{t("design.pending", { n: store.designDoc.changes.length })}</span>
              <button type="button" class="btn sm" onclick={() => store.designReview(store.designDoc.changes.map((c) => c.id), false)}>{t("design.revertAll")}</button>
              <button type="button" class="btn sm keep" onclick={() => store.designReview(store.designDoc.changes.map((c) => c.id), true)}>{t("design.keepAll")}</button>
            </div>
          {/if}
        </div>
        <!-- The conversation beside the board, as on a track. -->
        <div class="talk" style="width: {store.artifactChatWidth}px">
          <SplitHandle
            edge="left"
            width={store.artifactChatWidth}
            min={340}
            max={760}
            reset={460}
            label={t("design.chatResize")}
            onchange={(px, persist) => store.setArtifactChatWidth(px, persist)}
          />
          <Timeline />
          <Composer />
        </div>
      </div>
    </section>
  {:else}
    <section class="main blank">
      <p class="serif">{t("design.startTitle")}</p>
      <p class="hint">{t("design.startHint")}</p>
    </section>
  {/if}
</div>

<style>
  .designs {
    flex: 1;
    min-width: 0;
    display: flex;
  }

  .list {
    width: 240px;
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
    outline: none;
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

  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  /* Two lines, as a track's row: tags and time above, the name below. The
     left bar is the design's colour, only when one is chosen; the open one is
     told by its shade. */
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

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .main.blank {
    align-items: center;
    justify-content: center;
    gap: 8px;
  }

  .blank p {
    margin: 0;
  }

  .blank .serif {
    font-size: 24px;
    color: var(--dim);
  }

  .blank .hint {
    font-size: 13px;
    color: var(--lab);
  }

  header {
    flex-shrink: 0;
    padding: 18px 24px 14px;
    border-bottom: 1px solid var(--line);
  }

  .titlerow {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 6px;
  }

  .title,
  .rename {
    font-size: 26px;
    font-weight: 400;
    color: var(--hi);
    background: transparent;
    border: 0;
    padding: 0;
    text-align: left;
    min-width: 0;
  }

  .rename {
    border-bottom: 1px solid var(--acc);
    outline: none;
    font-family: var(--serif);
  }

  .grow {
    flex: 1;
  }

  .meter {
    display: flex;
    gap: 14px;
    font-size: 11px;
    color: var(--lab);
  }

  .meter b {
    font-weight: 600;
    margin-right: 3px;
  }

  .work {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .sidebox {
    flex-shrink: 0;
    display: flex;
    min-height: 0;
  }

  .boardwrap {
    position: relative;
    flex: 1;
    min-width: 200px;
    display: flex;
  }

  .review {
    position: absolute;
    right: 14px;
    bottom: 14px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px 6px 12px;
    background: var(--accbg);
    border: 1px solid var(--accln);
    font-size: 10px;
    letter-spacing: 0.08em;
    color: var(--acct);
  }

  .review .btn.sm {
    height: 26px;
    padding: 0 10px;
  }

  .review .keep {
    border-color: var(--acc);
    color: var(--hi);
  }

  /* The conversation: the track's timeline and composer, beside the board. */
  .talk {
    position: relative;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--line);
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    background-size: 22px 22px;
  }

  /* The track's composer and timeline are laid out for a wide column. */
  .talk :global(.composer) {
    padding-left: 16px;
    padding-right: 16px;
  }

  .talk :global(.scroll) {
    padding-left: 18px;
    padding-right: 18px;
  }
</style>
