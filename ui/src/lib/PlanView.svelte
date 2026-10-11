<script lang="ts">
  import { plans } from "./plan.svelte";
  import { store } from "./store.svelte";
  import PlanGraph from "./PlanGraph.svelte";
  import PlanStepEditor from "./PlanStepEditor.svelte";
  import { differsFromLast, openNeedsConfirm, resultsOf, sameVersion, saveTarget } from "./planGraph";
  import { whenFull } from "./time";
  import Working from "./Working.svelte";
  import { t } from "./i18n.svelte";

  /**
   * A routine's plan, open beside its form: the graph, the bands that want
   * the human (the instruction moved on since the plan was made, the agent's
   * suggestions, an instruction it proposed), the saved versions, and the
   * picked step's drawer. Its agent is the robot panel on the right.
   */
  const doc = $derived(plans.doc);
  const pending = $derived(doc.changes.length);
  const picked = $derived(doc.steps.find((s) => s.id === plans.picked));
  const versions = $derived([...(doc.saved ?? [])].reverse());

  /** The run whose step results the cards show. */
  const run = $derived(
    plans.resultRun ? store.routineRuns.find((r) => r.id === plans.resultRun) : store.routineRuns.find((r) => Array.isArray(r.steps) && r.steps.length),
  );
  const results = $derived(resultsOf(run?.steps));

  async function addStep() {
    const after = plans.picked ? [plans.picked] : [];
    const out = await plans.apply([{ op: "add_step", title: t("plan.newStep"), instruction: "", after }]);
    const id = out[0]?.id;
    if (typeof id === "string") plans.pick(id);
  }

  // ----- saving, and the versions -----
  let saving = $state(false);
  let note = $state("");
  let versionsOpen = $state(false);

  /** What "save over" writes into, and whether it would change anything. */
  const target = $derived(saveTarget(doc));
  const canOver = $derived(target !== undefined && plans.unsaved);
  const canNew = $derived(differsFromLast(doc));
  const saveBlocked = $derived(pending > 0 ? t("plan.saveSettleFirst", { n: pending }) : !canNew && !canOver ? t("plan.saveNothing") : "");

  /** The saved version the working plan is now (marked in the list). */
  const same = $derived(sameVersion(doc));
  /** The version waiting on "open it anyway?", if one is. */
  let confirmN = $state<number | null>(null);

  /** Open a version as the working plan: asking first when that would lose changes kept nowhere. */
  async function openVersion(n: number, sure = false) {
    if (n === same) {
      versionsOpen = false;
      return;
    }
    if (!sure && openNeedsConfirm(doc, n)) {
      confirmN = n;
      return;
    }
    confirmN = null;
    versionsOpen = false;
    await plans.openVersion(n);
  }

  async function save(over?: number) {
    if (saveBlocked) return;
    if (await plans.save(note, over)) {
      note = "";
      saving = false;
    }
  }
</script>

<div class="plan">
  <div class="tools" role="toolbar" aria-label={t("plan.tools")}>
    <button type="button" class="tb" onclick={addStep}>+ {t("plan.addStep")}</button>
    <button type="button" class="tb" onclick={() => plans.autoLayout()} title={t("plan.autoLayoutHint")}>{t("plan.autoLayout")}</button>
    <button type="button" class="tb" onclick={() => plans.undo()} title={t("plan.undo")} aria-label={t("plan.undo")}>↶</button>
    <span class="grow"></span>
    {#if plans.unsaved}
      <span class="dirty" title={t("plan.unsavedHint")}>● {t("plan.unsavedBadge")}</span>
    {/if}
    <div class="pop">
      <button
        type="button"
        class="tb save"
        class:ready={!saveBlocked}
        disabled={!!saveBlocked}
        title={saveBlocked || t("plan.saveHint")}
        aria-expanded={saving}
        onclick={() => {
          saving = !saving;
          versionsOpen = false;
        }}>{t("plan.save")}</button
      >
      {#if saving && !saveBlocked}
        <div class="menu" role="dialog" aria-label={t("plan.save")}>
          <label class="f">
            <span class="flab">{t("plan.saveNote")}</span>
            <!-- svelte-ignore a11y_autofocus -->
            <input bind:value={note} maxlength="200" autofocus onkeydown={(e) => e.key === "Enter" && canNew && save()} />
          </label>
          <div class="macts">
            <button type="button" class="tb" onclick={() => (saving = false)}>{t("plan.cancel")}</button>
            {#if target !== undefined}
              <button type="button" class="tb" disabled={!canOver} onclick={() => save(target)}>{t("plan.saveOver", { n: target })}</button>
            {/if}
            <button type="button" class="tb ready" disabled={!canNew} onclick={() => save()}>{t("plan.saveNew", { n: (plans.lastSaved ?? 0) + 1 })}</button>
          </div>
        </div>
      {/if}
    </div>
    <div class="pop">
      <button
        type="button"
        class="tb"
        aria-expanded={versionsOpen}
        onclick={() => {
          versionsOpen = !versionsOpen;
          saving = false;
        }}>{plans.lastSaved ? t("plan.versionNow", { n: plans.lastSaved }) : t("plan.neverSaved")} ▾</button
      >
      {#if versionsOpen}
        <div class="menu" role="menu">
          {#if !versions.length}
            <p class="mnote">{t("plan.noVersions")}</p>
          {:else}
            {#if pending}<p class="mnote warn">{t("plan.saveSettleFirst", { n: pending })}</p>{/if}
            {#each versions as v (v.n)}
              {#if confirmN === v.n}
                <div class="ver confirm" role="alertdialog" aria-label={t("plan.versionOpenConfirm")}>
                  <span>{t("plan.versionOpenConfirm")}</span>
                  <div class="macts">
                    <button type="button" class="tb" onclick={() => (confirmN = null)}>{t("plan.cancel")}</button>
                    <!-- svelte-ignore a11y_autofocus -->
                    <button type="button" class="tb ready" autofocus onclick={() => openVersion(v.n, true)}>{t("plan.versionOpenYes")}</button>
                  </div>
                </div>
              {:else}
                <button
                  type="button"
                  class="ver"
                  class:current={v.n === same}
                  role="menuitemradio"
                  aria-checked={v.n === same}
                  disabled={!!pending}
                  onclick={() => openVersion(v.n)}
                >
                  <b>v{v.n}</b>
                  <span>{whenFull(v.at, store.lang)} · {t("plan.versionSteps", { n: v.steps.length })}</span>
                  {#if v.edited_at}<span>{t("plan.savedOver", { when: whenFull(v.edited_at, store.lang) })}</span>{/if}
                  {#if v.note}<span class="vnote">{v.note}</span>{/if}
                  {#if v.n === plans.lastSaved}<span class="vrun">{t("plan.versionUsed")}</span>{/if}
                </button>
              {/if}
            {/each}
          {/if}
        </div>
      {/if}
    </div>
  </div>

  {#if plans.live}
    <div class="band live" role="status">
      <Working size={11} />
      <span>{plans.live.plan ? t("plan.running", { n: plans.live.plan.version }) : t("plan.runningNoPlan")}</span>
    </div>
  {/if}
  {#if plans.drafting}
    <div class="band draft" role="status">
      <Working size={11} />
      <span>{t("plan.drafting")}</span>
    </div>
  {/if}
  {#if plans.changed}
    <div class="band warn">
      <span>⚠ {t("plan.changedBand")}</span>
      <span class="grow"></span>
      <button type="button" class="tb" onclick={() => plans.restamp()}>{t("plan.changedIgnore")}</button>
    </div>
  {/if}
  {#if pending}
    <div class="band sug">
      <span class="mono">{t("plan.pending", { n: pending })}</span>
      <span class="grow"></span>
      <button type="button" class="tb" onclick={() => plans.review(doc.changes.map((c) => c.id), false)}>{t("plan.revertAll")}</button>
      <button type="button" class="tb" onclick={() => plans.review(doc.changes.map((c) => c.id), true)}>{t("plan.keepAll")}</button>
    </div>
  {/if}
  {#if doc.proposal}
    <div class="band prop">
      <div class="ptext">
        <span class="mono plab">{t("plan.proposal")}</span>
        <p>{doc.proposal.text}</p>
      </div>
      <div class="pacts">
        <button type="button" class="tb" onclick={() => plans.apply([{ op: "dismiss_proposal" }])}>{t("plan.proposalDismiss")}</button>
        <button type="button" class="tb ready" onclick={() => plans.takeProposal()}>{t("plan.proposalTake")}</button>
      </div>
    </div>
  {/if}
  {#if !plans.live && run && results.size}
    <div class="band res">
      <span>{t("plan.resultsOf", { when: whenFull(run.started_at, store.lang) })}</span>
      {#if plans.resultRun}<button type="button" class="link" onclick={() => (plans.resultRun = "")}>{t("plan.resultsLatest")}</button>{/if}
    </div>
  {/if}

  {#if !doc.steps.length && !pending}
    <div class="empty">
      <p class="et">{plans.drafting ? t("plan.drafting") : t("plan.emptyTitle")}</p>
      <button type="button" class="tb" onclick={addStep}>+ {t("plan.addStep")}</button>
    </div>
  {:else}
    <PlanGraph {results} />
  {/if}

  {#if picked}
    {#key picked.id}
      <PlanStepEditor step={picked} />
    {/key}
  {/if}
</div>

<style>
  .plan {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
  }

  .tools {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 18px;
    border-bottom: 1px solid var(--line);
    flex-wrap: wrap;
  }

  .grow {
    flex: 1;
  }

  .tb {
    height: 26px;
    padding: 0 10px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--lab);
    font-size: 12px;
  }

  .tb:hover:not(:disabled) {
    color: var(--hi);
    border-color: var(--acc);
  }

  .tb:disabled {
    opacity: 0.45;
  }

  .tb.ready {
    color: var(--hi);
    border-color: var(--acc);
  }

  .tb.save.ready {
    background: var(--accbg);
  }


  .dirty {
    font-size: 11.5px;
    color: var(--warn);
  }

  .pop {
    position: relative;
  }

  .menu {
    position: absolute;
    right: 0;
    top: 30px;
    z-index: 20;
    width: 320px;
    max-height: 60vh;
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .f {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .flab {
    font-size: 11px;
    font-weight: 600;
    color: var(--lab);
  }

  .f input {
    padding: 6px 9px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--hi);
    font: inherit;
    font-size: 12.5px;
  }

  .macts {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .mnote {
    margin: 2px 2px 4px;
    font-size: 11.5px;
    color: var(--lab);
  }

  .mnote.warn {
    color: var(--warn);
  }

  /* A version is a button: hover, focus and the one the working plan is now. */
  .ver {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    text-align: left;
    padding: 7px 8px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    border-top: 1px solid var(--lineq);
    font-size: 11.5px;
    color: var(--lab);
  }

  button.ver:hover:not(:disabled),
  button.ver:focus-visible {
    background: var(--sel);
    color: var(--txt);
  }

  button.ver:focus-visible {
    outline: 1px solid var(--focus);
    outline-offset: -1px;
  }

  .ver.current {
    border-left-color: var(--acc);
    background: var(--sel);
    color: var(--txt);
  }

  .ver:disabled {
    opacity: 0.5;
  }

  .ver b {
    color: var(--hi);
    font-size: 12.5px;
  }

  .ver.confirm {
    color: var(--warn);
    gap: 6px;
  }



  .vnote {
    color: var(--txt);
    overflow-wrap: anywhere;
  }

  .vrun {
    color: var(--oktx);
  }

  .band {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 18px;
    font-size: 12px;
    border-bottom: 1px solid var(--line);
    flex-wrap: wrap;
  }

  .band.warn {
    background: var(--warnbg);
    color: var(--warn);
    border-color: var(--warnln);
  }

  .band.sug {
    background: var(--accbg);
    color: var(--txt);
    border-color: var(--accln);
  }

  .band.prop {
    align-items: flex-start;
    background: var(--okbg);
    border-color: var(--okln);
  }

  .band.live {
    color: var(--txt);
    background: var(--accbg);
    border-color: var(--accln);
  }

  .band.draft {
    color: var(--txt);
    background: var(--sel);
  }

  .band.res {
    color: var(--lab);
  }

  .ptext {
    flex: 1;
    min-width: 0;
  }

  .plab {
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--oktx);
  }

  .ptext p {
    margin: 4px 0 0;
    white-space: pre-wrap;
    color: var(--txt);
    line-height: 1.6;
    max-height: 9em;
    overflow-y: auto;
  }

  .pacts {
    display: flex;
    gap: 6px;
  }

  .link {
    background: none;
    border: 0;
    color: var(--acct);
    font-size: 12px;
    padding: 0;
  }

  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 30px 18px;
    text-align: center;
    background: var(--kg-paper);
  }

  .et {
    margin: 0;
    font-family: var(--serif);
    font-size: 17px;
    color: var(--hi);
  }

  @media (max-width: 640px) {
    .tools,
    .band {
      padding-left: 12px;
      padding-right: 12px;
    }

    .menu {
      position: fixed;
      left: 12px;
      right: 12px;
      top: auto;
      width: auto;
    }
  }
</style>
