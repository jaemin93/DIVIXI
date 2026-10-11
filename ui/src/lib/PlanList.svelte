<script lang="ts">
  import { plans } from "./plan.svelte";
  import PlanCard from "./PlanCard.svelte";
  import { shownSteps, suggestions, waves, type StepResult } from "./planGraph";
  import { t } from "./i18n.svelte";

  /**
   * The plan as a list: wave by wave, in the order a run takes them, the
   * steps of one wave side by side (stacked on a phone). The way to read a
   * plan on a phone, and on a wide screen when the graph is more than is
   * wanted. What a step waits for is changed in its drawer, from a
   * checklist that will not offer a cycle.
   */
  let { results }: { results: Map<string, StepResult> } = $props();

  const shown = $derived(shownSteps(plans.doc));
  const sug = $derived(suggestions(plans.doc));
  const byId = $derived(new Map(shown.map((s) => [s.step.id, s])));
  const groups = $derived(waves(plans.doc.steps));
  /** Steps the agent removed have no wave; they are listed after the rest. */
  const gone = $derived(shown.filter((s) => s.removed));
</script>

<div class="list" role="list" aria-label={t("plan.listLabel")}>
  {#each groups as wave, i (i)}
    <section class="wave" role="listitem">
      <div class="wlab mono">
        {t("plan.wave", { n: i + 1 })}{#if wave.length > 1}<span class="side"> · {t("plan.waveTogether")}</span>{/if}
      </div>
      <div class="row">
        {#each wave as id (id)}
          {@const s = byId.get(id)}
          {#if s}
            <PlanCard step={s.step} suggestion={sug.get(id)} result={results.get(id)} onpointerdown={(e) => plans.pick(id, e.shiftKey)} />
          {/if}
        {/each}
      </div>
    </section>
  {/each}
  {#if gone.length}
    <section class="wave" role="listitem">
      <div class="wlab mono">{t("plan.removedByAgent")}</div>
      <div class="row">
        {#each gone as s (s.step.id)}
          <PlanCard step={s.step} suggestion={sug.get(s.step.id)} removed />
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px 18px 20px;
    background: var(--kg-paper);
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .wlab {
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--lab);
    margin-bottom: 6px;
    text-transform: uppercase;
  }

  .side {
    text-transform: none;
    letter-spacing: 0.04em;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }

  .row :global(.card) {
    width: 260px;
  }

  @media (max-width: 640px) {
    .list {
      padding: 12px;
    }

    .row {
      flex-direction: column;
    }

    .row :global(.card) {
      width: auto;
    }
  }
</style>
