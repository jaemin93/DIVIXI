<script lang="ts">
  import { store, agentLabel, type Routine } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import { shownChoice } from "./agentOptions";

  /**
   * The routine's run agent, in its settings: one line with what runs it
   * now, and a press opens the agent and model under it. A change is saved
   * at once and taken from the next run. Not the planning agent (the plan's,
   * in the right panel): that one is chosen there and beside [Plan].
   */
  let { routine }: { routine: Routine } = $props();

  let open = $state(false);

  const modelOption = $derived(store.optionsOf(routine.agent).find((o) => o.category === "model"));
  const modelName = $derived(modelOption ? store.choiceName(modelOption, store.effective(routine.agent, routine.config, modelOption)) : "");

  function setAgent(a: string) {
    if (a === routine.agent) return;
    // Options belong to the agent they were picked for.
    void store.saveRoutine(routine.id, { agent: a, config: {} });
  }

  function setModel(v: string) {
    if (!modelOption) return;
    const { [modelOption.id]: _old, ...rest } = routine.config;
    void store.saveRoutine(routine.id, { config: v ? { ...rest, [modelOption.id]: v } : rest });
  }
</script>

<button class="line" aria-expanded={open} onclick={() => (open = !open)}>
  {t("plan.runsOn")} <b>{agentLabel(routine.agent)}{#if modelName} · {modelName}{/if}</b> <span class="caret">{open ? "▾" : "▸"}</span>
</button>
{#if open}
  <div class="edit">
    <label>
      <span class="flab">{t("picker.agent")}</span>
      <select value={routine.agent} onchange={(e) => setAgent((e.currentTarget as HTMLSelectElement).value)}>
        {#each store.readyAgents as a (a.kind)}
          <option value={a.kind}>{agentLabel(a.kind)}</option>
        {/each}
        {#if !store.readyAgents.some((a) => a.kind === routine.agent)}
          <option value={routine.agent}>{agentLabel(routine.agent)}</option>
        {/if}
      </select>
    </label>
    {#if modelOption}
      <label>
        <span class="flab">{modelOption.name}</span>
        <select value={shownChoice(modelOption, routine.config[modelOption.id])} onchange={(e) => setModel((e.currentTarget as HTMLSelectElement).value)}>
          {#each modelOption.choices as c (c.id)}
            <option value={c.id}>{c.name}</option>
          {/each}
        </select>
      </label>
    {/if}
    <span class="note">{t("routines.nextRun")}</span>
  </div>
{/if}

<style>
  .line {
    background: none;
    border: 0;
    padding: 0;
    color: var(--lab);
    font-size: 12px;
    text-align: left;
  }

  .line b {
    color: var(--txt);
    font-weight: 500;
  }

  .line:hover,
  .line:hover b {
    color: var(--hi);
  }

  .caret {
    font-size: 10px;
  }

  /* Under the line: in the settings strip it takes a row of its own. */
  .edit {
    flex-basis: 100%;
    order: 99;
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 10px 14px;
    padding: 6px 0 4px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 180px;
  }

  .flab {
    font-size: 11px;
    font-weight: 600;
    color: var(--lab);
  }

  select {
    padding: 6px 10px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--hi);
    font: inherit;
    font-size: 13px;
  }

  .note {
    font-size: 11px;
    color: var(--lab);
    padding-bottom: 8px;
  }
</style>
