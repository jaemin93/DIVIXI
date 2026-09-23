<script lang="ts">
  import { store } from "./store.svelte";
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import SplitHandle from "./SplitHandle.svelte";
  import Board from "./Board.svelte";
  import Timeline from "./Timeline.svelte";
  import Composer from "./Composer.svelte";
  import { briefOf } from "./ink";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * A draft: the list of drafts, the board, and under it the conversation
   * with the draft's agent — the same timeline and composer as a track's
   * conductor (files, agent and model, context, stop). The header counts
   * what the draft has become (goals, constraints, open questions) and
   * carries it into a new track once it has a goal.
   */
  let renaming = $state(false);

  /** The list slides like the tracks column; not for those who asked for less motion. */
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const side = { axis: "x" as const, duration: reduced ? 0 : 200, easing: cubicOut };
  let titleDraft = $state("");
  let newTitle = $state("");
  let confirmDelete = $state("");

  const d = $derived(store.currentDraft);
  const notes = $derived(store.draftDoc.nodes.filter((n) => n.kind === "note"));
  const count = (tag: string) => notes.filter((n) => n.tag === tag && n.text.trim()).length;
  const goals = $derived(count("goal"));
  const constraints = $derived(count("constraint"));
  const questions = $derived(count("question"));

  async function create(e: Event) {
    e.preventDefault();
    await store.createDraft(newTitle.trim() || t("draft.untitled"));
    newTitle = "";
  }

  function rename() {
    if (!d) return;
    titleDraft = d.title;
    renaming = true;
  }

  async function commitRename() {
    renaming = false;
    if (d && titleDraft.trim() && titleDraft.trim() !== d.title) await store.updateDraft(d.id, { title: titleDraft.trim() });
  }

  function promote() {
    if (!d) return;
    const labels = {
      goal: t("draft.tag.goal"),
      constraint: t("draft.tag.constraint"),
      question: t("draft.tag.question"),
      idea: t("draft.tag.idea"),
      note: t("draft.notes"),
    };
    const goal = notes.find((n) => n.tag === "goal" && n.text.trim())?.text.trim().split("\n")[0] ?? "";
    store.promoteDraft(`${t("draft.briefLead")}\n\n${briefOf(d.title, store.draftDoc, labels)}`, goal);
  }
</script>

<div class="drafts">
  <!-- Every draft, most recently touched first; folds like the tracks column. -->
  {#if store.draftListOpen}
  <div class="sidebox" transition:slide={side}>
  <aside class="list">
    <div class="head"><span class="mlab">{t("draft.title")}</span></div>
    <form class="new" onsubmit={create}>
      <input type="text" bind:value={newTitle} placeholder={t("draft.newPh")} aria-label={t("draft.new")} maxlength="80" />
      <button class="btn" type="submit" title={t("draft.new")} aria-label={t("draft.new")}>+</button>
    </form>
    <div class="rows">
      {#each store.drafts as item (item.id)}
        <div class="row" class:on={item.id === store.draft}>
          <button class="pick" onclick={() => store.openDraft(item.id)}>
            <span class="name">{item.title}</span>
            <span class="mono when">{whenLabel(item.updated_at, store.now, store.lang)}</span>
          </button>
          {#if confirmDelete === item.id}
            <button class="del sure mono" onclick={() => { confirmDelete = ""; void store.deleteDraft(item.id); }}>{t("draft.deleteSure")}</button>
          {:else}
            <button class="del" onclick={() => (confirmDelete = item.id)} title={t("draft.delete")} aria-label={t("draft.delete")}>×</button>
          {/if}
        </div>
      {/each}
      {#if !store.drafts.length}
        <p class="none">{t("draft.none")}</p>
      {/if}
    </div>
  </aside>
  </div>
  {/if}

  {#if d}
    <section class="main">
      <header>
        <div class="mlab">DRAFT / {d.id}</div>
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
            <button class="serif title" ondblclick={rename} title={t("draft.renameHint")}>{d.title}</button>
          {/if}
          <span class="grow"></span>
          <!-- What the draft has become. -->
          <div class="meter mono" title={t("draft.meterHint")}>
            <span><b style="color: #46c46a">{goals}</b> {t("draft.tag.goal")}</span>
            <span><b style="color: var(--acct)">{constraints}</b> {t("draft.tag.constraint")}</span>
            <span><b style="color: var(--warn)">{questions}</b> {t("draft.tag.question")}</span>
          </div>
          <button class="btn btn-acc" disabled={goals === 0} onclick={promote} title={goals === 0 ? t("draft.promoteNeedsGoal") : t("draft.promoteHint")}>
            {t("draft.promote")}
          </button>
        </div>
      </header>
      <div class="work">
        <div class="boardwrap">
          <Board bind:selected={store.draftSelected} />
          {#if store.draftDoc.changes.length}
            <!-- The agent's board changes, all at once; one by one on the items. -->
            <div class="review">
              <span class="mono">{t("draft.pending", { n: store.draftDoc.changes.length })}</span>
              <button type="button" class="btn sm" onclick={() => store.draftReview(store.draftDoc.changes.map((c) => c.id), false)}>{t("draft.revertAll")}</button>
              <button type="button" class="btn sm keep" onclick={() => store.draftReview(store.draftDoc.changes.map((c) => c.id), true)}>{t("draft.keepAll")}</button>
            </div>
          {/if}
        </div>
        <!-- The conversation beside the board, as on a track. -->
        <div class="talk" style="width: {store.draftChatWidth}px">
          <SplitHandle
            edge="left"
            width={store.draftChatWidth}
            min={340}
            max={760}
            reset={460}
            label={t("draft.chatResize")}
            onchange={(px, persist) => store.setDraftChatWidth(px, persist)}
          />
          <Timeline />
          <Composer />
        </div>
      </div>
    </section>
  {:else}
    <section class="main blank">
      <p class="serif">{t("draft.startTitle")}</p>
      <p class="hint">{t("draft.startHint")}</p>
    </section>
  {/if}
</div>

<style>
  .drafts {
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
    display: flex;
    align-items: center;
    padding: 0 16px;
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

  .row {
    display: flex;
    align-items: center;
    border-left: 2px solid transparent;
  }

  .row:hover,
  .row.on {
    background: var(--sel);
  }

  .row.on {
    border-left-color: var(--acc);
  }

  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    padding: 9px 6px 9px 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
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
    font-size: 9px;
    color: var(--lab);
  }

  .del {
    margin-right: 8px;
    height: 24px;
    min-width: 24px;
    padding: 0 6px;
    background: transparent;
    border: 0;
    color: var(--lab);
    opacity: 0;
  }

  .row:hover .del,
  .del.sure {
    opacity: 1;
  }

  .del:hover {
    color: var(--acct);
  }

  .del.sure {
    font-size: 10px;
    color: var(--acct);
    border: 1px solid var(--accln);
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
