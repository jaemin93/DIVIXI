<script lang="ts">
  import { local } from "./ipc.svelte";
  import FolderPicker from "./FolderPicker.svelte";
  import { store, agentLabel, type AgentId, type ConfigOption, type OptionConfig, type Track, type TrackPatch, WORKER_FOLDERS, type WorkerFolder } from "./store.svelte";
  import Mark from "./Mark.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Making or changing a track: a name, one line of intent, the folder it
   * works in, and for the conductor and for workers an agent plus whatever
   * session options that agent advertises (mode, model, effort, …). With no
   * `track` this creates one, and is the first screen when nothing exists.
   */
  let { track }: { track?: Track } = $props();

  const editing = $derived(!!track);
  const first = $derived(store.tracks.length === 0);
  const ready = $derived(store.readyAgents);

  // The form seeds from the track as it was when opened; App remounts the
  // form (keyed by track id) when another track is edited.
  // svelte-ignore state_referenced_locally
  const seed = track;
  let name = $state(seed?.name ?? "");
  let intent = $state(seed?.intent ?? "");
  let cwd = $state(seed?.cwd ?? "");
  let agent = $state<AgentId>((seed?.agent as AgentId) ?? store.agent);
  let conductorConfig = $state<OptionConfig>({ ...(seed?.conductor_config ?? {}) });
  /**
   * The worker starts on the conductor's agent and options and keeps
   * following them until it is set apart: the first change on its tab
   * copies what the conductor has and goes from there. A track whose
   * worker was never set apart saves it that way, and workers then open
   * with the conductor's options.
   */
  const followed = !seed?.worker_agent && Object.keys(seed?.worker_config ?? {}).length === 0;
  let workerApart = $state(!followed);
  let workerAgent = $state<string>(seed?.worker_agent || (seed?.agent ?? ""));
  let workerConfig = $state<OptionConfig>({ ...(seed?.worker_config ?? {}) });
  let tab = $state<"conductor" | "worker">("conductor");
  let workerFolder = $state<WorkerFolder>(seed?.worker_folder ?? "subfolder");

  function setApart() {
    if (workerApart) return;
    workerAgent = agent;
    workerConfig = { ...conductorConfig };
    workerApart = true;
  }
  let busy = $state(false);
  let nameInput = $state<HTMLInputElement>();

  /** The agent and options workers will run on, as the worker tab shows them. */
  const shownWorkerAgent = $derived(workerApart ? workerAgent || agent : agent);
  const shownWorkerConfig = $derived(workerApart ? workerConfig : conductorConfig);
  const conductorOptions = $derived(store.optionsOf(agent));
  const workerOptions = $derived(store.optionsOf(shownWorkerAgent));

  /** "Claude Code · Opus" for a role's tab. */
  function roleSummary(id: string, options: ConfigOption[], config: OptionConfig): string {
    const model = options.find((o) => o.category === "model");
    const name = model ? store.choiceName(model, store.effective(id, config, model)) : "";
    return name ? `${agentLabel(id)} · ${name}` : agentLabel(id);
  }
  const canSubmit = $derived(!!name.trim() && ready.some((a) => a.kind === agent) && !busy);

  // Default folder for a new track: the repository the app was launched from.
  $effect(() => {
    if (editing) return;
    if (store.info === null) store.loadInfo();
    else if (!cwd && store.info.workspace) cwd = store.info.workspace;
  });

  $effect(() => {
    if (!ready.some((a) => a.kind === agent) && ready.length) agent = ready[0].kind;
  });

  $effect(() => {
    nameInput?.focus();
  });

  /** On a remote instance, its own folders in a picker the app draws. */
  let remotePick = $state(false);

  async function browse() {
    if (!local) {
      remotePick = true;
      return;
    }
    const picked = await store.pickFolder(cwd);
    if (picked) cwd = picked;
  }

  /** Keep only choices the agent still offers, so a stale id never travels. */
  function prune(config: OptionConfig, options: ConfigOption[]): OptionConfig {
    const out: OptionConfig = {};
    for (const o of options) {
      const v = config[o.id];
      if (v && o.choices.some((c) => c.id === v)) out[o.id] = v;
    }
    return out;
  }

  async function submit(e: Event) {
    e.preventDefault();
    if (!canSubmit) return;
    busy = true;
    try {
      const patch: TrackPatch = {
        name: name.trim(),
        intent: intent.trim(),
        cwd,
        agent,
        conductor_config: prune(conductorConfig, conductorOptions),
        worker_agent: workerApart ? shownWorkerAgent : "",
        worker_config: workerApart ? prune(workerConfig, workerOptions) : {},
        worker_folder: workerFolder,
      };
      const ok = track ? await store.updateTrack(track.id, patch) : await store.createTrack(patch);
      if (ok && track) store.view = "track";
    } finally {
      busy = false;
    }
  }

  function cancel() {
    if (first && !editing) return;
    store.view = "track";
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") cancel();
  }

  /** Label for an option's "leave it" choice. */
  function defaultLabel(role: string, option: ConfigOption): string {
    if (option.category === "mode") {
      const mode = store.autonomousModeOf(role) || option.current;
      return t("newtrack.autoMode", { mode: store.choiceName(option, mode) });
    }
    return t("newtrack.agentDefault", { value: store.choiceName(option, option.current) });
  }

  /** The heading of a new track's sheet: the first one is greeted differently. */
  const newTitle = $derived(first ? t("newtrack.first") : t("newtrack.title"));
  const submitLabel = $derived.by(() => {
    if (editing) return busy ? t("newtrack.saving") : t("newtrack.save");
    return busy ? t("newtrack.creating") : t("newtrack.create");
  });

  /** Options in a fixed, readable order: mode first, then model, then the rest. */
  function ordered(options: ConfigOption[]): ConfigOption[] {
    const ranks: Record<string, number> = { mode: 0, model: 1 };
    const rank = (o: ConfigOption) => ranks[o.category] ?? 2;
    return [...options].sort((a, b) => rank(a) - rank(b));
  }
</script>

<svelte:window onkeydown={onKey} />

<main>
  <div class="sheet">
    <div class="mlab">{editing ? t("newtrack.editTitle") : newTitle}</div>
    <div class="titlerow">
      <Mark size={28} />
      <h1 class="serif">{editing ? track?.name : newTitle}</h1>
    </div>
    <p class="blurb">{editing ? t("newtrack.editBlurb") : t("newtrack.blurb")}</p>

    <form onsubmit={submit}>
      <label class="field">
        <span class="mlab-sm">{t("newtrack.name")}</span>
        <input type="text" bind:this={nameInput} bind:value={name} placeholder={t("newtrack.namePh")} maxlength="80" />
      </label>

      <label class="field">
        <span class="mlab-sm">{t("newtrack.intent")}</span>
        <input type="text" bind:value={intent} placeholder={t("newtrack.intentPh")} maxlength="200" />
      </label>

      <div class="field">
        <span class="mlab-sm">{t("newtrack.folder")}</span>
        <div class="folder">
          <Icon name="folder" size={14} />
          <input class="mono" type="text" bind:value={cwd} spellcheck="false" />
          <button class="btn" type="button" onclick={browse}>{t("newtrack.browse")}</button>
        </div>
      </div>

      <!-- Where workers work: a folder each (their results side by side), a git checkout each (code, merged by the human), or all in the track folder. -->
      <div class="field">
        <span class="mlab-sm">{t("newtrack.workerFolder")}</span>
        <div class="wfolders" role="radiogroup" aria-label={t("newtrack.workerFolder")}>
          {#each WORKER_FOLDERS as f (f)}
            <button type="button" class="wf" class:on={workerFolder === f} role="radio" aria-checked={workerFolder === f} onclick={() => (workerFolder = f)}>
              <span class="wft">{t(`newtrack.wf.${f}`)}</span>
              <span class="wfd">{t(`newtrack.wf.${f}.desc`)}</span>
            </button>
          {/each}
        </div>
      </div>

      {#if ready.length === 0}
        <div class="field"><div class="mono none">{t("newtrack.noAgents")}</div></div>
      {:else}
        <!-- Who runs what: one role at a time, each tab saying what it is set to. -->
        <div class="roles" role="tablist">
          <button type="button" class="roletab" class:on={tab === "conductor"} role="tab" aria-selected={tab === "conductor"} onclick={() => (tab = "conductor")}>
            <span class="mlab-sm">{t("newtrack.conductor")}</span>
            <span class="mono sum">{roleSummary(agent, conductorOptions, conductorConfig)}</span>
          </button>
          <button type="button" class="roletab" class:on={tab === "worker"} role="tab" aria-selected={tab === "worker"} onclick={() => (tab = "worker")}>
            <span class="mlab-sm">{t("newtrack.workers")}</span>
            <span class="mono sum">{roleSummary(shownWorkerAgent, workerOptions, shownWorkerConfig)}</span>
          </button>
        </div>
        {#if tab === "conductor"}
          {@render role(t("newtrack.conductorNote"), t("newtrack.agent"), agent, (a) => (agent = a as AgentId), conductorOptions, conductorConfig, (id, v) => (conductorConfig[id] = v), agent)}
        {:else}
          {@render role(
            t("newtrack.workerNote"),
            t("newtrack.workerAgent"),
            shownWorkerAgent,
            (a) => {
              setApart();
              if (a !== workerAgent) workerConfig = {};
              workerAgent = a;
            },
            workerOptions,
            shownWorkerConfig,
            (id, v) => {
              setApart();
              workerConfig[id] = v;
            },
            shownWorkerAgent,
          )}
        {/if}
      {/if}

      <div class="foot">
        {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
        <span class="grow"></span>
        {#if editing || !first}
          <button class="btn" type="button" onclick={cancel}>{t("newtrack.cancel")}</button>
        {/if}
        <button class="btn btn-acc" type="submit" disabled={!canSubmit}>
          {submitLabel}
        </button>
      </div>
    </form>
  </div>
</main>

<!-- One role: which agent, then that agent's options as selects. -->
{#snippet role(
  note: string,
  agentLabelText: string,
  chosen: string,
  pick: (id: string) => void,
  options: ConfigOption[],
  config: OptionConfig,
  setOption: (id: string, value: string) => void,
  effectiveAgent: string,
)}
  <section class="role" role="tabpanel">
    <p class="rolenote">{note}</p>

    <div class="field">
      <span class="mlab-sm">{agentLabelText}</span>
      <div class="agents" role="radiogroup" aria-label={agentLabelText}>
        {#each ready as a (a.kind)}
          <button type="button" class="opt" class:on={chosen === a.kind} role="radio" aria-checked={chosen === a.kind} onclick={() => pick(a.kind)}>
            <span class="dot"></span>
            <span class="mono">{agentLabel(a.kind)}</span>
          </button>
        {/each}
      </div>
    </div>

    {#if options.length === 0}
      <div class="field"><div class="mono none">{t("newtrack.noOptions")}</div></div>
    {/if}
    {#each ordered(options) as option (option.id)}
      <label class="field">
        <span class="mlab-sm">{option.name} <span class="mono cat">{option.category}</span></span>
        <select
          value={config[option.id] ?? ""}
          onchange={(e) => setOption(option.id, (e.currentTarget as HTMLSelectElement).value)}
        >
          <option value="">{defaultLabel(effectiveAgent, option)}</option>
          {#each option.choices as c (c.id)}
            <option value={c.id}>{c.name}{c.group ? ` · ${c.group}` : ""}{c.id !== c.name ? ` (${c.id.replace(/^.*[#/]/, "")})` : ""}</option>
          {/each}
        </select>
        {#if option.category === "mode"}
          {@const current = config[option.id]}
          {@const desc = option.choices.find((c) => c.id === current)?.description}
          <span class="note">{desc || t("newtrack.modeNote")}</span>
        {:else if option.description}
          <span class="note">{option.description}</span>
        {/if}
      </label>
    {/each}
  </section>
{/snippet}

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
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    background-size: 22px 22px;
  }

  .sheet {
    width: min(680px, 100%);
    margin: 0 auto;
    padding: 48px 34px 40px;
  }

  .titlerow {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 10px;
  }

  h1 {
    margin: 0;
    font-weight: 400;
    font-size: 36px;
    line-height: 1.1;
    color: var(--hi);
  }

  .blurb {
    margin: 12px 0 0;
    font-size: 13px;
    line-height: 1.6;
    color: var(--dim);
    max-width: 600px;
  }

  form {
    margin-top: 30px;
    border-top: 1px solid var(--line);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 18px 0;
    border-bottom: 1px solid var(--lineq);
  }

  /* The role switch: two tabs, each with what the role is set to. */
  .roles {
    display: flex;
    margin-top: 26px;
    border: 1px solid var(--line);
  }

  .roletab {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 5px;
    padding: 11px 14px;
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    text-align: left;
    color: var(--dim);
  }

  .roletab + .roletab {
    border-left: 1px solid var(--line);
  }

  .roletab:hover {
    background: var(--sel);
  }

  .roletab.on {
    background: var(--sel);
    border-bottom-color: var(--acc);
    color: var(--hi);
  }

  .roletab.on .mlab-sm {
    color: var(--hi);
  }

  .sum {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }

  .rolenote {
    margin: 16px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--dim);
  }

  .cat {
    margin-left: 8px;
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--lab);
  }

  input,
  select {
    height: 36px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 13px;
    padding: 0 12px;
  }

  select {
    padding-right: 28px;
    appearance: none;
    background-image: linear-gradient(45deg, transparent 50%, var(--lab) 50%), linear-gradient(135deg, var(--lab) 50%, transparent 50%);
    background-position: calc(100% - 16px) 15px, calc(100% - 11px) 15px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  input.mono {
    font-family: var(--mono);
    font-size: 12px;
  }

  input:focus,
  select:focus {
    border-color: var(--acc);
  }

  .note {
    font-size: 11px;
    line-height: 1.55;
    color: var(--lab);
  }

  .wfolders {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .wf {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 9px 11px;
    text-align: left;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    cursor: pointer;
  }

  .wf.on {
    border-color: var(--acc);
    background: var(--accbg);
  }

  .wft {
    font-size: 13px;
    color: var(--hi);
  }

  .wfd {
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--dim);
  }

  .folder {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--lab);
  }

  .folder input {
    flex: 1;
    min-width: 0;
  }

  .folder .btn {
    height: 36px;
  }

  .agents {
    display: flex;
    flex-wrap: wrap;
    border: 1px solid var(--line);
  }

  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 14px;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    color: var(--dim);
    font-size: 12px;
  }

  .opt:last-child {
    border-right: 0;
  }

  .opt:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .opt.on {
    color: var(--hi);
    background: var(--sel);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--idle);
  }

  .opt.on .dot {
    background: var(--ok);
  }

  .none {
    font-size: 11px;
    color: var(--lab);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-top: 22px;
  }

  .grow {
    flex: 1;
  }

  .err {
    font-size: 11px;
    color: var(--acct);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 360px;
  }
</style>
