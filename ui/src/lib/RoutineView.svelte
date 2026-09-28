<script lang="ts">
  import { store, agentLabel, type Run } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import Working from "./Working.svelte";
  import ReportCard from "./ReportCard.svelte";
  import { t } from "./i18n.svelte";
  import { whenFull } from "./time";

  /**
   * One routine: what it tells its worker, and every time it has run.
   *
   * The instruction is shown in full and is editable, and that is the point
   * of this screen. A conductor writing a routine has the whole
   * conversation in front of it and can save an instruction that leans on
   * it — "the file we just looked at" — which reads as nothing three weeks
   * later. No prompt rule catches that reliably. The human seeing the text
   * they will actually be re-running, and being able to fix it, is what
   * does.
   */
  const routine = $derived(store.routines.find((r) => r.id === store.routine));
  const track = $derived(store.tracks.find((tr) => tr.id === routine?.track));
  const live = $derived(routine ? store.routineLive(routine) : false);

  /** The draft, and the saved text it was taken from, so an edit made while
   *  a run is in flight is not thrown away by a refresh. */
  let draft = $state("");
  let taken = $state("");
  let editing = $state(false);
  let armed = $state(false);

  $effect(() => {
    const text = routine?.instruction ?? "";
    if (text !== taken) {
      taken = text;
      if (!editing) draft = text;
    }
  });

  // Moving to another routine drops an unsaved edit rather than carrying it
  // across: the text belongs to the routine it was typed under.
  $effect(() => {
    store.routine;
    editing = false;
    armed = false;
  });

  const dirty = $derived(editing && draft.trim() !== taken.trim() && draft.trim().length > 0);

  async function save() {
    if (!routine || !dirty) {
      editing = false;
      return;
    }
    await store.saveRoutine(routine.id, { instruction: draft.trim() });
    editing = false;
  }

  function cancel() {
    draft = taken;
    editing = false;
  }

  async function remove() {
    if (!routine) return;
    if (!armed) {
      armed = true;
      return;
    }
    await store.deleteRoutine(routine.id);
  }

  /** A run's standing in a word. A run the human stopped is recorded
   *  `done` with a stop reason, so that is read before the status: "it
   *  finished" and "it was cut short" are not the same thing to someone
   *  deciding whether to run the routine again. */
  function statusWord(run: Run): string {
    if (run.status === "running" || run.status === "connecting") return t("routines.running");
    if (run.status === "failed") return t("routines.status.failed");
    if (run.stopReason) return t("routines.status.stopped");
    return t("routines.status.done");
  }
</script>

{#if !routine}
  <section class="blank"><p>{t("routines.pickOne")}</p></section>
{:else}
  <section>
    <header>
      <div class="who">
        <h1>{routine.name}</h1>
        <p class="meta">
          {t("routines.track")}: {track?.name ?? t("routines.trackGone")}
          · {t("routines.worker")}: <code>{routine.worker}</code>
          · {t("routines.agent")}: <code>{agentLabel(routine.agent || track?.worker_agent || track?.agent || "")}</code>
        </p>
        <!-- Said where the button is, not only in the docs: running this
             opens the worker anew, and a conversation the conductor was
             keeping with a worker of the same name ends there. -->
        <p class="warn">{t("routines.freshNote", { worker: routine.worker })}</p>
      </div>
      <div class="acts">
        <button class="del" class:armed onclick={remove} onblur={() => (armed = false)}>
          {armed ? t("routines.deleteArm") : t("routines.delete")}
        </button>
        <button class="run" disabled={live} onclick={() => routine && store.runRoutine(routine.id)}>
          {#if live}<Working size={12} />{:else}<Icon name="play" size={12} />{/if}
          {live ? t("routines.running") : t("routines.runNow")}
        </button>
      </div>
    </header>

    <div class="body">
      <div class="block">
        <div class="blabel">
          <span>{t("routines.instruction")}</span>
          {#if !editing}
            <button class="edit" onclick={() => (editing = true)}>{t("routines.edit")}</button>
          {/if}
        </div>
        {#if editing}
          <textarea bind:value={draft} rows="8" aria-label={t("routines.instruction")}></textarea>
          <p class="hint">{t("routines.instructionHint")}</p>
          <!-- The instruction reaches the worker as written, so an agent
               that takes slash commands takes them from here too. Worth a
               line: nothing else in the app says a routine can call a
               skill the agent already has. -->
          <p class="hint">{t("routines.skillHint")}</p>
          <div class="erow">
            <button class="ghost" onclick={cancel}>{t("routines.cancel")}</button>
            <button class="save" disabled={!dirty} onclick={save}>{t("routines.save")}</button>
          </div>
        {:else}
          <!-- Shown whole, never clipped: an instruction the human cannot
               read in full is one they cannot judge before running it. -->
          <pre class="shown">{routine.instruction}</pre>
          <p class="hint">{t("routines.instructionHint")}</p>
        {/if}
      </div>

      <div class="block">
        <div class="blabel"><span>{t("routines.history")}</span></div>
        {#if !store.routineRuns.length}
          <p class="hint">{t("routines.noRuns")}</p>
        {:else}
          <ul class="runs">
            {#each store.routineRuns as run (run.id)}
              <li>
                <button class="rrow" onclick={() => store.openRoutineRun(run)}>
                  <span class="rwhen">{whenFull(run.startedAt, store.lang)}</span>
                  <span class="rstat" class:bad={run.status === "failed"}>{statusWord(run)}</span>
                  <span class="rid mono">{run.id}</span>
                </button>
                <ReportCard run={run.id} worker={routine.worker} compact={true} />
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
  </section>
{/if}

<style>
  section {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  .blank {
    align-items: center;
    justify-content: center;
    color: var(--lab);
    font-size: 13px;
  }

  header {
    flex-shrink: 0;
    display: flex;
    align-items: flex-start;
    gap: 16px;
    padding: 14px 20px;
    border-bottom: 1px solid var(--line);
  }

  .who {
    flex: 1;
    min-width: 0;
  }

  h1 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--hi);
  }

  .meta {
    margin: 4px 0 0;
    font-size: 11px;
    color: var(--lab);
  }

  .meta code {
    font-size: 11px;
  }

  .warn {
    margin: 6px 0 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--lab);
  }

  .acts {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }

  /* The one irreversible action is the only filled button, as the rest of
     the app does it: weight says what the press costs. */
  .run {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 14px;
    background: var(--acc);
    border: 1px solid var(--acc);
    color: var(--accon);
    font-weight: 600;
  }

  .run:disabled {
    opacity: 0.5;
  }

  .del,
  .edit,
  .ghost,
  .save {
    height: 30px;
    padding: 0 12px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .del:hover,
  .edit:hover,
  .ghost:hover {
    color: var(--hi);
    border-color: var(--acc);
  }

  .del.armed {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .save:not(:disabled) {
    color: var(--hi);
    border-color: var(--acc);
  }

  .save:disabled {
    opacity: 0.45;
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px 20px 28px;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .blabel {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    font-weight: 600;
    color: var(--lab);
  }

  .blabel .edit {
    height: 22px;
    padding: 0 8px;
    font-size: 11px;
  }

  .shown {
    margin: 0;
    padding: 12px 14px;
    background: var(--card);
    border: 1px solid var(--line);
    font-size: 13px;
    line-height: 1.65;
    color: var(--hi);
    white-space: pre-wrap;
    word-break: break-word;
  }

  textarea {
    padding: 12px 14px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--hi);
    font: inherit;
    font-size: 13px;
    line-height: 1.65;
    resize: vertical;
  }

  .hint {
    margin: 0;
    font-size: 11px;
    line-height: 1.6;
    color: var(--lab);
  }

  .erow {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }

  .runs {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .rrow {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 6px 0;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--lab);
    font-size: 12px;
  }

  .rrow:hover .rwhen {
    color: var(--hi);
  }

  .rwhen {
    color: var(--txt);
  }

  .rstat.bad {
    color: var(--deltx);
  }

  .rid {
    margin-left: auto;
    font-size: 11px;
    opacity: 0.6;
  }
</style>
