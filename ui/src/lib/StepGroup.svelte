<script lang="ts">
  import { t, type Key } from "./i18n.svelte";
  import { detail, isOpen, running, span, summarize, took, verbKey, visible, type Step } from "./steps";

  /**
   * One run of steps in a turn: a list that grows while the agent works,
   * with the newest step marked on the rule, and one summary line once the
   * answer has started. Either can be turned into the other by hand.
   *
   * A turn restored from the store comes with a count and no steps; opening
   * it asks for them (`onopen`), and until they arrive it says so.
   */
  let {
    steps,
    live,
    active,
    count,
    initial,
    onopen,
  }: {
    steps: Step[];
    /** The turn is still going. */
    live: boolean;
    /** These are the steps being added to right now. */
    active: boolean;
    /** Calls known before the steps themselves are (a restored turn). */
    count?: number;
    /** Open from the start: the human already asked to see this turn. */
    initial?: boolean;
    onopen?: () => void;
  } = $props();

  // Undefined until the human chooses; then theirs, whatever the turn does.
  // Read from `initial` only once, on purpose: it says where this starts.
  // svelte-ignore state_referenced_locally
  let chosen = $state<boolean | undefined>(initial);
  let all = $state(false);

  const open = $derived(isOpen(active, chosen));
  const sum = $derived(summarize(steps));
  const tools = $derived(steps.length ? sum.tools : (count ?? 0));
  const view = $derived(visible(steps, all));
  const id = $props.id();

  function toggle() {
    chosen = !open;
    if (chosen) onopen?.();
  }

  /** "4 steps · 1 failed · 12.0s", or what it was if it called nothing. */
  const said = $derived.by(() => {
    const parts = [{ text: tools === 0 ? t("steps.thoughts") : tools === 1 ? t("steps.countOne") : t("steps.count", { n: tools }), bad: false }];
    if (sum.failed) parts.push({ text: t("steps.failed", { n: sum.failed }), bad: true });
    if (sum.ms !== undefined && sum.ms >= 1000) parts.push({ text: span(sum.ms), bad: false });
    return parts;
  });
</script>

<div class="steps" class:open>
  <button
    class="head"
    type="button"
    aria-expanded={open}
    aria-controls={id}
    title={open ? t("steps.fold") : t("steps.unfold")}
    onclick={toggle}
  >
    <span class="mlab">{active ? t("steps.working") : t("steps.worked")}</span>
    {#if !active}
      <span class="said">
        {#each said as part, i (i)}{#if i}{" · "}{/if}<span class:bad={part.bad}>{part.text}</span>{/each}
      </span>
    {/if}
    <svg class="chev" width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M1.5 3 4 5.5 6.5 3" /></svg>
  </button>

  {#if open}
    <ol class="list" {id}>
      {#if view.hidden}
        <li class="more">
          <button type="button" class="mono" onclick={() => (all = true)}>{t("steps.earlier", { n: view.hidden })}</button>
        </li>
      {/if}
      {#if steps.length === 0}
        <li class="step"><span class="what">{t("steps.loading")}</span></li>
      {/if}
      {#each view.shown as step, i (view.hidden + i)}
        {@const current = active && i === view.shown.length - 1}
        <li class="step" class:current class:thought={step.kind === "thought"}>
          {#if current}<span class="tick pulse" aria-hidden="true"></span>{/if}
          <span class="verb">{t(verbKey(step, live) as Key)}</span>
          {#if step.kind === "thought"}
            <span class="what">{step.text}</span>
          {:else}
            {@const ms = took(step.tool)}
            <span class="what" title={step.tool.title}>{detail(step.tool)}</span>
            {#if step.tool.status === "failed"}
              <span class="aux mono bad">{t("steps.failedAux")}</span>
            {:else if ms !== undefined && !(live && running(step.tool))}
              <span class="aux mono">{span(ms)}</span>
            {/if}
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .steps {
    margin: 2px 0;
  }

  /* The same gap as between two paragraphs of the turn. */
  :global(.ctext) + .steps {
    margin-top: 8px;
  }

  .steps:not(:last-child) {
    margin-bottom: 8px;
  }

  /* The label line, open or folded: the house micro-label, then what it
     comes to in the quieter body face. */
  .head {
    display: inline-flex;
    align-items: baseline;
    gap: 9px;
    max-width: 100%;
    padding: 2px 0;
    background: transparent;
    border: 0;
    color: var(--lab);
    cursor: pointer;
    text-align: left;
  }

  .head:hover,
  .head:hover .mlab {
    color: var(--hi);
  }

  .head .mlab {
    color: var(--body);
  }

  .said {
    font-family: var(--sans);
    font-size: calc(var(--chat-fs) - 2px);
    color: var(--dim);
  }

  .said .bad {
    color: var(--acct);
  }

  .chev {
    align-self: center;
    transition: transform 0.15s ease;
  }

  .open .chev {
    transform: rotate(180deg);
  }

  /* The rule down the left; the newest step sits on it. */
  .list {
    list-style: none;
    margin: 6px 0 2px 3px;
    padding: 0 0 0 14px;
    border-left: 1px solid var(--lines);
  }

  .step {
    position: relative;
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-height: 24px;
    padding: 2px 0;
    font-family: var(--sans);
    font-size: calc(var(--chat-fs) - 1px);
    line-height: 1.55;
  }

  .tick {
    position: absolute;
    left: -18px;
    top: calc(2px + 0.775em - 3.5px);
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }

  .verb {
    flex-shrink: 0;
    font-weight: 600;
    color: var(--txt);
  }

  .what {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--dim);
  }

  /* A thought is prose, not a name: a few lines of it, then it trails off. */
  .thought .what {
    white-space: normal;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    font-style: italic;
  }

  .aux {
    flex-shrink: 0;
    margin-left: 12px;
    font-size: 10px;
    letter-spacing: 0.06em;
    color: var(--lab);
  }

  .aux.bad {
    color: var(--acct);
  }

  .more button {
    padding: 2px 0 4px;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 10px;
    letter-spacing: 0.1em;
    cursor: pointer;
  }

  .more button:hover {
    color: var(--hi);
  }
</style>
