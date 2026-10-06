<script lang="ts">
  import { store } from "./store.svelte";
  import ItemColumn from "./ItemColumn.svelte";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import { whenFull } from "./time";
  import Board from "./Board.svelte";
  import PanelToggle from "./PanelToggle.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * A design (an artifact): the list of designs and the board. The
   * conversation with its agent is a column of the app's layout on the right
   * (AgentPanel), opened from the panel toggle in the head as a track opens
   * its working folder. The header counts
   * what the design has become (goals, constraints, open questions); a
   * design is attached to a track's conductor from the track's composer.
   */
  let renaming = $state(false);
  let titleDraft = $state("");
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

  async function create(name: string) {
    await store.createDesign(name.trim() || t("design.untitled"));
  }

  /** Every design, most recently touched first, as the column's rows. */
  const rows = $derived(
    store.designs.map((d) => ({
      id: d.id,
      name: d.title,
      tags: d.tags,
      color: d.color,
      meta: whenLabel(d.updated_at, store.now, store.lang),
      metaTitle: whenFull(d.updated_at, store.lang),
    })),
  );

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
    <ItemColumn
      title={t("design.title")}
      width={store.designListWidth}
      widthLabel={t("design.listWidth")}
      onwidth={(px, persist) => store.setDesignListWidth(px, persist)}
      onclose={() => store.setDesignList(false)}
      newLabel={t("design.new")}
      newButton={t("design.newButton")}
      newPlaceholder={t("design.newPh")}
      searchPlaceholder={t("design.search")}
      oncreate={create}
      items={rows}
      picked={store.designTags}
      onpick_tags={(tags) => store.setDesignTags(tags)}
      current={store.artifact}
      menued={menu?.id ?? null}
      onpick={(id) => store.openArtifact(id)}
      onmenu={(id, x, y) => (menu = { id, x, y })}
      empty={t("design.none")}
    />
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
        <div class="hrow">
          <div class="mlab">DESIGN / {d.id}</div>
          <span class="grow"></span>
          <PanelToggle on={store.designChatOpen} title={t("panel.agent")} onclick={() => store.setDesignChat(!store.designChatOpen)} />
        </div>
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
            <!-- A double-click for the pointer, as before; Enter or Space
                 for the keyboard, which had no way in at all. A single
                 click still does nothing, deliberately: the title is read
                 far more often than it is changed. -->
            <button
              class="serif title"
              ondblclick={rename}
              onkeydown={(e) => {
                if (e.key !== "Enter" && e.key !== " ") return;
                e.preventDefault();
                rename();
              }}
              title={t("design.renameHint")}
              aria-label={t("design.renameTitle", { title: d.title })}>{d.title}</button
            >
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
          <!-- One board per design: its selection, edit and arrow-in-progress never carry over. -->
          {#key store.artifact}
            <Board bind:selected={store.designSelected} />
          {/key}
          {#if store.designDoc.changes.length}
            <!-- The agent's board changes, all at once; one by one on the items. -->
            <div class="review">
              <span class="mono">{t("design.pending", { n: store.designDoc.changes.length })}</span>
              <button type="button" class="btn sm" onclick={() => store.designReview(store.designDoc.changes.map((c) => c.id), false)}>{t("design.revertAll")}</button>
              <button type="button" class="btn sm keep" onclick={() => store.designReview(store.designDoc.changes.map((c) => c.id), true)}>{t("design.keepAll")}</button>
            </div>
          {/if}
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
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
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
    /* Top and right as a track's head, so the panel toggle sits where it does there. */
    padding: 14px 10px 14px 24px;
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

  /* The head's first row: the label, and the panel toggle at the right as on a track. */
  .hrow {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .review .keep {
    border-color: var(--acc);
    color: var(--hi);
  }

</style>
