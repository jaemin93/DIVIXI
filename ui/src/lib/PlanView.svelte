<script lang="ts">
  import { plans } from "./plan.svelte";
  import { store, artifactKey } from "./store.svelte";
  import PlanGraph from "./PlanGraph.svelte";
  import PlanList from "./PlanList.svelte";
  import PlanStepEditor from "./PlanStepEditor.svelte";
  import { resultsOf } from "./planGraph";
  import { whenFull } from "./time";
  import { t } from "./i18n.svelte";

  /**
   * A routine's plan, open beside its form: the graph or the list, the bands
   * that want the human (the instruction moved on since the plan was made,
   * the agent's suggestions, an instruction the agent proposed), and the
   * picked step's drawer. Its agent is the panel on the right.
   */
  const doc = $derived(plans.doc);
  const pending = $derived(doc.changes.length);
  const picked = $derived(doc.steps.find((s) => s.id === plans.picked));
  /** The plan's agent has a turn in flight (its own conversation, whatever else is open). */
  const busy = $derived(store.runs.some((r) => r.track === artifactKey(plans.id) && (r.status === "running" || r.status === "connecting")));

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

  let confirmOver = $state(false);
  async function startOver() {
    if (!confirmOver) {
      confirmOver = true;
      return;
    }
    confirmOver = false;
    plans.menu = false;
    await plans.startOver();
  }

  async function merge() {
    plans.menu = false;
    await plans.merge();
  }
</script>

<div class="plan">
  <div class="tools" role="toolbar" aria-label={t("plan.tools")}>
    {#if !plans.narrow}
      <div class="seg" role="group" aria-label={t("plan.showLabel")}>
        <button type="button" class:on={plans.show === "graph"} aria-pressed={plans.show === "graph"} onclick={() => plans.setShow("graph")}>{t("plan.showGraph")}</button>
        <button type="button" class:on={plans.show === "list"} aria-pressed={plans.show === "list"} onclick={() => plans.setShow("list")}>{t("plan.showList")}</button>
      </div>
    {/if}
    <button type="button" class="tb" onclick={addStep}>+ {t("plan.addStep")}</button>
    {#if plans.show === "graph"}
      <button type="button" class="tb" onclick={() => plans.apply([{ op: "auto_layout" }])} title={t("plan.autoLayoutHint")}>{t("plan.autoLayout")}</button>
    {/if}
    <button type="button" class="tb" onclick={() => plans.undo()} title={t("plan.undo")} aria-label={t("plan.undo")}>↶</button>
    <span class="grow"></span>
    <label class="use" title={t("plan.useInRunHint")}>
      <input type="checkbox" checked={doc.use_in_run} onchange={(e) => plans.apply([{ op: "use_in_run", on: (e.currentTarget as HTMLInputElement).checked }])} />
      {t("plan.useInRun")}
    </label>
    <div class="menuwrap">
      <button type="button" class="tb" aria-expanded={plans.menu} onclick={() => (plans.menu = !plans.menu)}>{t("plan.again")} ▾</button>
      {#if plans.menu}
        <div class="menu" role="menu">
          {#if pending}
            <p class="mnote">{t("plan.againSettleFirst", { n: pending })}</p>
          {/if}
          <button type="button" role="menuitem" disabled={!!pending || busy} onclick={merge}>
            <b>{t("plan.againMerge")}</b>
            <span>{t("plan.againMergeHint")}</span>
          </button>
          <button type="button" role="menuitem" class:armed={confirmOver} disabled={!!pending || busy} onclick={startOver} onblur={() => (confirmOver = false)}>
            <b>{confirmOver ? t("plan.againOverArm") : t("plan.againOver")}</b>
            <span>{t("plan.againOverHint")}</span>
          </button>
          {#if doc.history?.length}
            <div class="hist">
              <span class="mnote">{t("plan.history")}</span>
              {#each doc.history as h, i (h.at)}
                <button type="button" role="menuitem" disabled={!!pending} onclick={() => { plans.menu = false; void plans.restore(i); }}>
                  {t("plan.historyItem", { when: whenFull(h.at, store.lang), n: h.steps.length })}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>

  {#if plans.changed}
    <div class="band warn">
      <span>⚠ {t("plan.changedBand")}</span>
      <span class="grow"></span>
      <button type="button" class="tb" disabled={!!pending || busy} onclick={merge}>{t("plan.againMerge")}</button>
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
        <button type="button" class="tb keep" onclick={() => plans.takeProposal()}>{t("plan.proposalTake")}</button>
      </div>
    </div>
  {/if}
  {#if run && results.size}
    <div class="band res">
      <span>{t("plan.resultsOf", { when: whenFull(run.started_at, store.lang) })}</span>
      {#if plans.resultRun}<button type="button" class="link" onclick={() => (plans.resultRun = "")}>{t("plan.resultsLatest")}</button>{/if}
    </div>
  {/if}

  {#if !doc.steps.length && !pending}
    <div class="empty">
      <p class="et">{busy ? t("plan.drafting") : t("plan.emptyTitle")}</p>
      <p class="eh">{t("plan.emptyHint")}</p>
      <div class="eacts">
        <button type="button" class="tb keep" disabled={busy} onclick={() => plans.draft()}>{t("plan.askDraft")}</button>
        <button type="button" class="tb" onclick={addStep}>+ {t("plan.addStep")}</button>
      </div>
    </div>
  {:else if plans.show === "graph"}
    <PlanGraph {results} />
  {:else}
    <PlanList {results} />
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

  .seg {
    display: flex;
    border: 1px solid var(--line);
  }

  .seg button {
    height: 26px;
    padding: 0 11px;
    background: none;
    border: 0;
    color: var(--lab);
    font-size: 12px;
  }

  .seg button + button {
    border-left: 1px solid var(--line);
  }

  .seg button.on {
    color: var(--hi);
    background: var(--sel);
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

  .tb.keep {
    color: var(--hi);
    border-color: var(--acc);
  }

  .use {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--txt);
  }

  .menuwrap {
    position: relative;
  }

  .menu {
    position: absolute;
    right: 0;
    top: 30px;
    z-index: 20;
    width: 300px;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .menu button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: left;
    padding: 7px 9px;
    background: transparent;
    border: 1px solid transparent;
    color: var(--txt);
  }

  .menu button:hover:not(:disabled) {
    border-color: var(--acc);
  }

  .menu button:disabled {
    opacity: 0.45;
  }

  .menu button.armed b {
    color: var(--deltx);
  }

  .menu b {
    font-weight: 600;
    font-size: 12.5px;
  }

  .menu span {
    font-size: 11px;
    color: var(--lab);
  }

  .mnote {
    margin: 2px 4px 4px;
    font-size: 11px;
    color: var(--warn);
  }

  .hist {
    border-top: 1px solid var(--line);
    padding-top: 4px;
    display: flex;
    flex-direction: column;
  }

  .hist .mnote {
    color: var(--lab);
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
    gap: 6px;
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

  .eh {
    margin: 0;
    max-width: 440px;
    font-size: 12px;
    line-height: 1.6;
    color: var(--lab);
  }

  .eacts {
    margin-top: 8px;
    display: flex;
    gap: 8px;
  }

  @media (max-width: 640px) {
    .tools,
    .band {
      padding-left: 12px;
      padding-right: 12px;
    }

    .menu {
      width: min(300px, calc(100vw - 24px));
    }
  }
</style>
