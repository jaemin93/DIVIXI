<script lang="ts">
  import { plans } from "./plan.svelte";
  import { beforeChoices, type PlanStep } from "./planGraph";
  import { t } from "./i18n.svelte";

  /**
   * One step, open for editing: under the graph on a wide screen, under the
   * list on a phone. Each field is written when it is left (or, for the
   * checkboxes and selects, when it changes), as one edit the human can undo.
   * Editing a step the agent suggested keeps the suggestion, as on a design.
   */
  let { step }: { step: PlanStep } = $props();

  let title = $state("");
  let instruction = $state("");
  let inputs = $state("");
  let outputs = $state("");
  let doneWhen = $state("");
  /** The step and its version these fields were filled from. */
  let filled = $state("");

  // Refilled when another step is picked, or when this one changed elsewhere
  // (the agent, an undo) -- but never under a field being typed in.
  let typing = $state(false);
  $effect(() => {
    const key = `${step.id}:${JSON.stringify(step)}`;
    if (key === filled || typing) return;
    filled = key;
    title = step.title;
    instruction = step.instruction;
    inputs = step.inputs.join("\n");
    outputs = step.outputs.join("\n");
    doneWhen = step.done_when;
  });

  const lines = (v: string) =>
    v
      .split(/\n|,/)
      .map((x) => x.trim())
      .filter(Boolean);

  async function write(fields: Record<string, unknown>) {
    await plans.apply([{ op: "update_step", id: step.id, ...fields }]);
  }

  function leave(field: "title" | "instruction" | "inputs" | "outputs" | "done_when") {
    typing = false;
    if (field === "title") {
      if (!title.trim()) {
        title = step.title;
        return;
      }
      if (title.trim() !== step.title) void write({ title: title.trim() });
    } else if (field === "instruction") {
      if (instruction.trim() !== step.instruction) void write({ instruction: instruction.trim() });
    } else if (field === "done_when") {
      if (doneWhen.trim() !== step.done_when) void write({ done_when: doneWhen.trim() });
    } else {
      const next = lines(field === "inputs" ? inputs : outputs);
      if (JSON.stringify(next) !== JSON.stringify(step[field])) void write({ [field]: next });
    }
  }

  const choices = $derived(beforeChoices(plans.doc.steps, step.id));

  function toggleBefore(id: string, on: boolean) {
    const after = on ? [...step.after, id] : step.after.filter((a) => a !== id);
    void plans.apply([{ op: "set_after", id: step.id, after }]);
  }

  let armed = $state(false);
  function remove() {
    if (!armed) {
      armed = true;
      return;
    }
    armed = false;
    void plans.apply([{ op: "delete", ids: [step.id] }]);
    plans.pick("");
  }
</script>

<div class="drawer" aria-label={t("plan.step.label", { id: step.id })}>
  <div class="dh">
    <span class="mono sid">{step.id}</span>
    <span class="dt">{step.title}</span>
    <span class="grow"></span>
    <button class="btn sm" class:armed onclick={remove} onblur={() => (armed = false)}>
      {armed ? t("plan.step.deleteArm") : t("plan.step.delete")}
    </button>
    <button class="btn sm" onclick={() => plans.pick("")} aria-label={t("plan.step.close")} title={t("plan.step.close")}>×</button>
  </div>
  <div class="grid">
    <label class="f">
      <span class="flab">{t("plan.step.title")}</span>
      <input bind:value={title} onfocus={() => (typing = true)} onblur={() => leave("title")} />
    </label>
    <label class="f">
      <span class="flab">{t("plan.step.inputs")}</span>
      <textarea rows="2" bind:value={inputs} placeholder={t("plan.step.listHint")} onfocus={() => (typing = true)} onblur={() => leave("inputs")}></textarea>
    </label>
    <label class="f">
      <span class="flab">{t("plan.step.outputs")}</span>
      <textarea rows="2" bind:value={outputs} placeholder={t("plan.step.listHint")} onfocus={() => (typing = true)} onblur={() => leave("outputs")}></textarea>
    </label>
    <label class="f span2">
      <span class="flab">{t("plan.step.instruction")}</span>
      <textarea rows="3" bind:value={instruction} onfocus={() => (typing = true)} onblur={() => leave("instruction")}></textarea>
    </label>
    <div class="f">
      <span class="flab">{t("plan.step.before")}</span>
      <div class="deps" role="group" aria-label={t("plan.step.before")}>
        {#if !choices.length}
          <span class="none">{t("plan.step.beforeNone")}</span>
        {/if}
        {#each choices as c (c.id)}
          <label class="dep" class:closed={c.closed} title={c.closed ? t("plan.step.cycle") : ""}>
            <input type="checkbox" checked={c.on} disabled={c.closed} onchange={(e) => toggleBefore(c.id, (e.currentTarget as HTMLInputElement).checked)} />
            <span class="mono">{c.id}</span>
            <span class="dname">{c.title}</span>
          </label>
        {/each}
      </div>
    </div>
    <label class="f span2">
      <span class="flab">{t("plan.step.doneWhen")}</span>
      <input bind:value={doneWhen} onfocus={() => (typing = true)} onblur={() => leave("done_when")} />
    </label>
    <div class="f">
      <span class="flab">{t("plan.step.onFail")}</span>
      <select value={step.on_fail} onchange={(e) => void write({ on_fail: (e.currentTarget as HTMLSelectElement).value })}>
        <option value="stop">{t("plan.step.onFailStop")}</option>
        <option value="continue">{t("plan.step.onFailContinue")}</option>
      </select>
      <label class="ask">
        <input type="checkbox" checked={step.approval === "ask"} onchange={(e) => void write({ approval: (e.currentTarget as HTMLInputElement).checked ? "ask" : "none" })} />
        {t("plan.step.approval")}
      </label>
    </div>
  </div>
</div>

<style>
  .drawer {
    flex-shrink: 0;
    border-top: 1px solid var(--line);
    background: var(--bg);
    padding: 10px 18px 14px;
    max-height: 46vh;
    overflow-y: auto;
  }

  .dh {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }

  .sid {
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--dim);
  }

  .dt {
    font-weight: 600;
    color: var(--hi);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
  }

  .btn {
    height: 24px;
    padding: 0 9px;
    font-size: 12px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .btn:hover {
    color: var(--hi);
    border-color: var(--acc);
  }

  .btn.armed {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px 14px;
  }

  .span2 {
    grid-column: span 2;
  }

  .f {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .flab {
    font-size: 11px;
    font-weight: 600;
    color: var(--lab);
  }

  input:not([type="checkbox"]),
  textarea,
  select {
    padding: 6px 9px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--hi);
    font: inherit;
    font-size: 12.5px;
    min-width: 0;
  }

  textarea {
    line-height: 1.6;
    resize: vertical;
  }

  .deps {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 5px 8px;
    background: var(--inp);
    border: 1px solid var(--line);
    max-height: 96px;
    overflow-y: auto;
    font-size: 12px;
  }

  .dep {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--txt);
    min-width: 0;
  }

  .dep .mono {
    font-size: 10.5px;
    color: var(--dim);
  }

  .dname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dep.closed {
    color: var(--lab);
    text-decoration: line-through;
  }

  .none {
    color: var(--lab);
  }

  .ask {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 6px;
    font-size: 12px;
    color: var(--txt);
  }

  @media (max-width: 640px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .span2 {
      grid-column: auto;
    }

    .drawer {
      max-height: none;
      padding: 10px 14px 18px;
    }
  }
</style>
