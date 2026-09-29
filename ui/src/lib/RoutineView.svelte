<script lang="ts">
  import { store, agentLabel, NEW_ROUTINE, type RoutineRun, type OptionConfig } from "./store.svelte";
  import { local } from "./ipc.svelte";
  import FolderPicker from "./FolderPicker.svelte";
  import Icon from "./Icon.svelte";
  import Working from "./Working.svelte";
  import Markdown from "./Markdown.svelte";
  import { t } from "./i18n.svelte";
  import { whenFull } from "./time";

  /**
   * One routine: what it runs, where, on what — and every time it has run.
   *
   * The same screen writes a new one and edits a saved one, because they are
   * the same four answers. A routine runs alone, so all four are its own:
   * there is no track lending it a folder or an agent, and the form says so
   * by asking for them outright.
   *
   * The instruction is shown in full and is editable, and that is the point.
   * A conductor writing a routine has the whole conversation in front of it
   * and can save something that leans on it — "the file we just looked at" —
   * which reads as nothing three weeks later. No prompt rule catches that
   * reliably. The human seeing the text they will actually be re-running,
   * and being able to fix it, is what does.
   */
  const making = $derived(store.routine === NEW_ROUTINE);
  const saved = $derived(store.routines.find((r) => r.id === store.routine));
  const live = $derived(!making && !!saved && store.routineLive(saved.id));

  let name = $state("");
  let instruction = $state("");
  let cwd = $state("");
  let agent = $state("");
  let config = $state<OptionConfig>({});
  /** The routine these fields were filled from, so a reload does not stamp
   *  on what is being typed. */
  let filledFrom = $state("");
  let armed = $state(false);
  let failed = $state("");

  $effect(() => {
    const key = making ? NEW_ROUTINE : (saved?.id ?? "");
    if (key === filledFrom) return;
    filledFrom = key;
    armed = false;
    failed = "";
    if (making) {
      // A blank form still starts somewhere sensible: the agent the human
      // is most likely to want, and nothing else guessed for them.
      name = "";
      instruction = "";
      cwd = "";
      agent = store.readyAgents[0]?.kind ?? "";
      config = {};
    } else if (saved) {
      name = saved.name;
      instruction = saved.instruction;
      cwd = saved.cwd;
      agent = saved.agent;
      config = { ...saved.config };
    }
  });

  const options = $derived(store.optionsOf(agent));
  /** Shown only when this agent advertised models; agents that do not say
   *  get no empty select. */
  const modelOption = $derived(options.find((o) => o.category === "model"));

  const complete = $derived(!!name.trim() && !!instruction.trim() && !!cwd.trim() && !!agent);
  const dirty = $derived(
    making ||
      (!!saved &&
        (name.trim() !== saved.name ||
          instruction.trim() !== saved.instruction ||
          cwd.trim() !== saved.cwd ||
          agent !== saved.agent ||
          JSON.stringify(config) !== JSON.stringify(saved.config))),
  );

  /** On a remote instance, its own folders in a picker the app draws. */
  let remotePick = $state(false);

  async function browse() {
    // The system picker only knows this PC's folders, and the bridge refuses
    // it outright ("pick_folder is not available from another device"). A
    // routine on a remote instance works in that instance's folders, so it
    // browses them the same way a track does.
    if (!local) {
      remotePick = true;
      return;
    }
    const picked = await store.pickFolder(cwd);
    if (picked) cwd = picked;
  }

  async function save() {
    if (!complete) return;
    failed = "";
    const patch = { name: name.trim(), instruction: instruction.trim(), cwd: cwd.trim(), agent, config };
    const ok = making ? await store.createRoutine(patch) : await store.saveRoutine(saved!.id, patch);
    if (!ok) failed = store.lastError;
    else filledFrom = "";
  }

  async function remove() {
    if (!saved) return;
    if (!armed) {
      armed = true;
      return;
    }
    await store.deleteRoutine(saved.id);
  }

</script>

<section>
  <header>
    <div class="who">
      <h1>{making ? t("routines.newTitle") : (saved?.name ?? "")}</h1>
      <p class="meta">{t("routines.standalone")}</p>
    </div>
    {#if !making && saved}
      <div class="acts">
        <button class="del" class:armed onclick={remove} onblur={() => (armed = false)}>
          {armed ? t("routines.deleteArm") : t("routines.delete")}
        </button>
        {#if live}
          <button class="run" onclick={() => store.cancelRoutine(saved.id)}>
            <Working size={12} /> {t("routines.stop")}
          </button>
        {:else}
          <button class="run" disabled={dirty} title={dirty ? t("routines.saveFirst") : ""} onclick={() => store.runRoutine(saved.id)}>
            <Icon name="play" size={12} /> {t("routines.runNow")}
          </button>
        {/if}
      </div>
    {/if}
  </header>

  <div class="body">
    <!-- Where and on what, above the words: the same order the form asks a
         person to think in, and it is never hidden behind a menu. -->
    <div class="grid">
      <label class="f wide">
        <span class="flab">{t("routines.name")}</span>
        <input bind:value={name} placeholder={t("routines.namePlaceholder")} />
      </label>

      <div class="f wide">
        <span class="flab">{t("routines.folder")}</span>
        <div class="frow">
          <input bind:value={cwd} placeholder={t("routines.folderPlaceholder")} spellcheck="false" />
          <button class="ghost" onclick={browse} title={t("routines.browse")} aria-label={t("routines.browse")}>
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>

      <label class="f">
        <span class="flab">{t("routines.agent")}</span>
        <select bind:value={agent}>
          {#if !store.readyAgents.length}<option value="">{t("routines.noAgents")}</option>{/if}
          {#each store.readyAgents as a (a.kind)}
            <option value={a.kind}>{agentLabel(a.kind)}</option>
          {/each}
        </select>
      </label>

      {#if modelOption}
        <label class="f">
          <span class="flab">{modelOption.name}</span>
          <select
            value={config[modelOption.id] ?? ""}
            onchange={(e) => {
              const v = (e.currentTarget as HTMLSelectElement).value;
              const next = { ...config };
              if (v) next[modelOption.id] = v;
              else delete next[modelOption.id];
              config = next;
            }}
          >
            <option value="">{t("routines.agentDefault", { what: store.choiceName(modelOption, modelOption.current) })}</option>
            {#each modelOption.choices as c (c.id)}
              <option value={c.id}>{c.name}</option>
            {/each}
          </select>
        </label>
      {/if}
    </div>

    <div class="block">
      <span class="flab">{t("routines.instruction")}</span>
      <textarea bind:value={instruction} rows="9" placeholder={t("routines.instructionPlaceholder")}></textarea>
    </div>

    <div class="erow">
      {#if failed}<span class="bad">{failed}</span>{/if}
      <button class="save" disabled={!complete || !dirty} onclick={save}>
        {making ? t("routines.create") : t("routines.save")}
      </button>
    </div>

    {#if !making && saved}
      <div class="block">
        <span class="flab">{t("routines.history")}</span>
        {#if !store.routineRuns.length}
          <p class="hint">{t("routines.noRuns")}</p>
        {:else}
          <ul class="runs">
            {#each store.routineRuns as run (run.id)}
              <li>
                <div class="rrow">
                  <span class="rwhen">{whenFull(run.started_at, store.lang)}</span>
                  <span class="rstat" class:bad={run.status === "failed"}>{t(`routines.status.${run.status}`)}</span>
                  {#if run.tools}<span class="rtools">{t("routines.tools", { n: run.tools })}</span>{/if}
                  <span class="rid mono">{run.id}</span>
                </div>
                {#if run.error}
                  <p class="rerr">{run.error}</p>
                {:else if run.report?.summary}
                  <p class="rsum">{run.report.summary}</p>
                {/if}
                {#if run.output}
                  <details class="rout">
                    <summary>{t("routines.fullReply")}</summary>
                    <Markdown source={run.output} />
                  </details>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </div>
</section>

{#if remotePick}
  <FolderPicker
    start={cwd}
    onpick={(p) => {
      cwd = p;
      remotePick = false;
    }}
    onclose={() => (remotePick = false)}
  />
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
  .ghost,
  .save {
    height: 30px;
    padding: 0 12px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .del:hover,
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
    gap: 18px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
    gap: 12px 16px;
  }

  .f {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }

  .f.wide {
    grid-column: 1 / -1;
  }

  .frow {
    display: flex;
    gap: 8px;
  }

  .frow input {
    flex: 1;
    min-width: 0;
  }

  .flab {
    font-size: 11px;
    font-weight: 600;
    color: var(--lab);
  }

  input,
  select,
  textarea {
    padding: 7px 10px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--hi);
    font: inherit;
    font-size: 13px;
    min-width: 0;
  }

  textarea {
    line-height: 1.65;
    resize: vertical;
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .hint {
    margin: 0;
    font-size: 11px;
    line-height: 1.6;
    color: var(--lab);
  }

  .erow {
    display: flex;
    gap: 10px;
    align-items: center;
    justify-content: flex-end;
  }

  .bad {
    font-size: 12px;
    color: var(--deltx);
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
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 4px 0 0;
    color: var(--lab);
    font-size: 12px;
  }

  .rwhen {
    color: var(--txt);
  }

  .rstat.bad {
    color: var(--deltx);
  }

  .rtools {
    font-size: 11px;
  }

  .rid {
    margin-left: auto;
    font-size: 11px;
    opacity: 0.6;
  }

  .rsum,
  .rerr {
    margin: 2px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--txt);
  }

  .rerr {
    color: var(--deltx);
  }

  .rout {
    margin-top: 4px;
    font-size: 12px;
  }

  .rout summary {
    cursor: pointer;
    color: var(--lab);
    font-size: 11px;
  }
</style>
