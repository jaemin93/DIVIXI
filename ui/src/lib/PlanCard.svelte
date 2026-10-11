<script lang="ts">
  import { plans } from "./plan.svelte";
  import { store } from "./store.svelte";
  import { changedFields, RESULT_GLYPH, type PlanChange, type PlanStep, type RunState, type StepResult, type SuggestionKind } from "./planGraph";
  import Working from "./Working.svelte";
  import { t, type Key } from "./i18n.svelte";

  /**
   * One step as a card, on the graph and in the list alike: its id, who made
   * it, its title and the start of its instruction, what it waits for and
   * leaves behind. A step the agent suggested is dashed, says what the
   * suggestion did, and carries its own keep and revert; one the agent
   * removed stays, struck through, until that is settled.
   */
  let {
    step,
    suggestion,
    removed = false,
    result,
    run,
    told = false,
    pinned = step.x !== undefined,
    style = "",
    onpointerdown,
    onhandle,
  }: {
    step: PlanStep;
    suggestion?: { change: PlanChange; kind: SuggestionKind };
    removed?: boolean;
    result?: StepResult;
    /** Where the step is in a run in flight. */
    run?: RunState;
    /** Whether the run's agent has said anything yet (until then, nothing is dimmed). */
    told?: boolean;
    /** Sitting where a person put it (a pin out of order is let go, and not marked). */
    pinned?: boolean;
    style?: string;
    onpointerdown?: (e: PointerEvent) => void;
    onhandle?: (e: PointerEvent) => void;
  } = $props();

  const picked = $derived(store.planSelected.includes(step.id));
  /** What the agent's suggestion did to this step, in words. */
  const what = $derived.by(() => {
    if (!suggestion) return "";
    if (suggestion.kind !== "changed") return t(`plan.sug.${suggestion.kind}` as Key);
    return t("plan.sug.changed", { what: changedFields(suggestion.change).map((f) => t(`plan.field.${f}` as Key)).join(", ") });
  });
  const ended = $derived(run === "done" || run === "failed" || run === "skipped" ? run : result?.status);
  const glyph = $derived(ended ? RESULT_GLYPH[ended] : "");

  function key(e: KeyboardEvent) {
    if (removed) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      plans.pick(step.id, e.shiftKey);
    }
  }
</script>

<!-- The card is pressed and dragged as a whole; the part that is picked (by
     keyboard too) is its face, and the suggestion's buttons sit beside it,
     never inside another button. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="card"
  class:sug={!!suggestion}
  class:removed
  class:picked
  class:waiting={run === "waiting"}
  class:running={run === "running"}
  {style}
  data-step={step.id}
  onpointerdown={(e) => {
    if ((e.target as HTMLElement).closest("button")) return;
    onpointerdown?.(e);
  }}
>
  <div class="face" role="button" tabindex={removed ? -1 : 0} aria-pressed={picked} aria-label={`${step.id} ${step.title}`} onkeydown={key}>
  <div class="top mono">
    <span>{step.id}</span>
    <span>{step.by === "agent" ? t("plan.by.agent") : t("plan.by.human")}</span>
    {#if pinned}<span class="pin" title={t("plan.pinned")}>📌</span>{/if}
    <span class="grow"></span>
    {#if run === "running"}
      <Working size={11} />
    {:else if glyph}
      <span class="res {ended}" title={result?.note || t(`plan.result.${ended}` as Key)}>{glyph}</span>
    {/if}
  </div>
  <div class="ttl">{step.title}</div>
  {#if step.instruction}<div class="ins">{step.instruction}</div>{/if}
  <div class="io mono">
    {#if step.after.length}<span>← {step.after.join(", ")}</span>{/if}
    {#each step.outputs as o (o)}<span>→ {o}</span>{/each}
  </div>
  {#if suggestion}<div class="what">{what}</div>{/if}
  </div>
  {#if suggestion}
    <div class="rev">
      <button type="button" onclick={() => plans.review([suggestion.change.id], false)}>{t("plan.sug.revert")}</button>
      <button type="button" onclick={() => plans.review([suggestion.change.id], true)}>{t("plan.sug.keep")}</button>
    </div>
  {/if}
  {#if onhandle && !removed}
    <span class="handle" role="presentation" title={t("plan.connectHint")} onpointerdown={(e) => { e.stopPropagation(); onhandle(e); }}></span>
  {/if}
</div>

<style>
  .card {
    position: relative;
    width: 220px;
    min-height: 104px;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 8px 10px 9px;
    cursor: pointer;
    user-select: none;
    touch-action: none;
  }

  .card:hover {
    border-color: var(--dim);
  }

  .card.picked {
    border-color: var(--hi);
    box-shadow: 0 0 0 1px var(--hi);
  }

  .card.sug {
    border-style: dashed;
    border-color: var(--acc);
    background: var(--accbg);
  }

  .card.removed {
    opacity: 0.6;
  }

  /* A run in flight: the step under way stands out, the ones still to come recede. */
  .card.running {
    border-color: var(--acc);
  }

  .card.waiting {
    opacity: 0.45;
  }

  .card.removed .ttl,
  .card.removed .ins {
    text-decoration: line-through;
  }

  .face {
    outline: none;
  }

  .card:has(.face:focus-visible) {
    outline: 1px solid var(--focus);
    outline-offset: 1px;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10px;
    color: var(--lab);
    letter-spacing: 0.06em;
  }

  .grow {
    flex: 1;
  }

  .pin {
    font-size: 9px;
  }


  .res {
    font-size: 12px;
  }

  .res.done {
    color: var(--ok);
  }

  .res.failed {
    color: var(--deltx);
  }

  .res.skipped {
    color: var(--lab);
  }

  .ttl {
    margin-top: 4px;
    font-weight: 600;
    color: var(--hi);
    font-size: 13px;
    overflow-wrap: anywhere;
  }

  .ins {
    margin-top: 3px;
    font-size: 11.5px;
    color: var(--body);
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .io {
    margin-top: 6px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .io span {
    font-size: 9.5px;
    color: var(--dim);
    border: 1px solid var(--line);
    padding: 0 4px;
  }

  .what {
    margin-top: 6px;
    font-size: 11px;
    color: var(--acct);
  }

  .rev {
    display: flex;
    margin: 8px -10px -9px;
  }

  .rev button {
    flex: 1;
    height: 24px;
    font-size: 11px;
    background: var(--card);
    border: 0;
    border-top: 1px dashed var(--acc);
    color: var(--txt);
  }

  .rev button + button {
    border-left: 1px dashed var(--acc);
  }

  .rev button:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .handle {
    position: absolute;
    right: -6px;
    top: 50%;
    width: 11px;
    height: 11px;
    margin-top: -5px;
    background: var(--bg);
    border: 1px solid var(--dim);
    border-radius: 50%;
    cursor: crosshair;
  }

  .handle:hover {
    border-color: var(--acc);
    background: var(--accbg);
  }
</style>
