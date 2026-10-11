<script lang="ts">
  import { plans } from "./plan.svelte";
  import { beforeChoices, editedFields, type PlanStep } from "./planGraph";
  import { t } from "./i18n.svelte";

  /**
   * One step's fields, in the drawer under the graph (PlanDrawer). Each field is written when it is left (or, for the
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

  async function write(fields: Record<string, unknown>) {
    await plans.apply([{ op: "update_step", id: step.id, ...fields }]);
  }

  /**
   * Write what the boxes hold, the changed fields only. Called a moment
   * after typing stops, and when a box is left: so "Unsaved" shows while
   * typing, not only once the box is left.
   */
  let timer: ReturnType<typeof setTimeout> | null = null;
  function flush() {
    if (timer) clearTimeout(timer);
    timer = null;
    const fields = editedFields(step, { title, instruction, inputs, outputs, done_when: doneWhen });
    if (Object.keys(fields).length) void write(fields);
  }

  function typed() {
    typing = true;
    if (timer) clearTimeout(timer);
    timer = setTimeout(flush, 500);
  }

  function leave() {
    typing = false;
    if (!title.trim()) title = step.title;
    flush();
  }

  const choices = $derived(beforeChoices(plans.doc.steps, step.id));

  function toggleBefore(id: string, on: boolean) {
    const after = on ? [...step.after, id] : step.after.filter((a) => a !== id);
    void plans.apply([{ op: "set_after", id: step.id, after }]);
  }
</script>

<div class="fields" role="group" aria-label={t("plan.step.label", { id: step.id })}>
  <div class="grid">
    <label class="f">
      <span class="flab">{t("plan.step.title")}</span>
      <input bind:value={title} oninput={typed} onblur={leave} />
    </label>
    <label class="f">
      <span class="flab">{t("plan.step.inputs")}</span>
      <textarea rows="2" bind:value={inputs} placeholder={t("plan.step.listHint")} oninput={typed} onblur={leave}></textarea>
    </label>
    <label class="f">
      <span class="flab">{t("plan.step.outputs")}</span>
      <textarea rows="2" bind:value={outputs} placeholder={t("plan.step.listHint")} oninput={typed} onblur={leave}></textarea>
    </label>
    <label class="f span2">
      <span class="flab">{t("plan.step.instruction")}</span>
      <textarea rows="3" bind:value={instruction} oninput={typed} onblur={leave}></textarea>
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
      <input bind:value={doneWhen} oninput={typed} onblur={leave} />
    </label>
    <div class="f">
      <span class="flab">{t("plan.step.onFail")}</span>
      <select value={step.on_fail} onchange={(e) => void write({ on_fail: (e.currentTarget as HTMLSelectElement).value })}>
        <option value="stop">{t("plan.step.onFailStop")}</option>
        <option value="continue">{t("plan.step.onFailContinue")}</option>
      </select>
    </div>
  </div>
</div>

<style>
  /* The drawer scrolls; the fields only keep their margins. */
  .fields {
    padding: 2px 18px 18px;
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


  @media (max-width: 640px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .span2 {
      grid-column: auto;
    }

    .fields {
      padding: 2px 14px 24px;
    }
  }
</style>
