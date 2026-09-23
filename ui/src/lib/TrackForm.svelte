<script lang="ts">
  import { store, agentLabel, type AgentId, type ConfigOption, type OptionConfig, type Track, type TrackPatch } from "./store.svelte";
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
  let workerAgent = $state<string>(seed?.worker_agent ?? "");
  let workerConfig = $state<OptionConfig>({ ...(seed?.worker_config ?? {}) });
  let busy = $state(false);
  let nameInput = $state<HTMLInputElement>();

  /** The agent lanes will run on. */
  const laneAgent = $derived(workerAgent || agent);
  const conductorOptions = $derived(store.optionsOf(agent));
  const workerOptions = $derived(store.optionsOf(laneAgent));
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

  async function browse() {
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
        worker_agent: workerAgent,
        worker_config: prune(workerConfig, workerOptions),
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

  /** Options in a fixed, readable order: mode first, then model, then the rest. */
  function ordered(options: ConfigOption[]): ConfigOption[] {
    const rank = (o: ConfigOption) => (o.category === "mode" ? 0 : o.category === "model" ? 1 : 2);
    return [...options].sort((a, b) => rank(a) - rank(b));
  }
</script>

<svelte:window onkeydown={onKey} />

<main>
  <div class="sheet">
    <div class="mlab">{editing ? t("newtrack.editTitle") : first ? t("newtrack.first") : t("newtrack.title")}</div>
    <div class="titlerow">
      <Mark size={28} ink="var(--hi)" />
      <h1 class="serif">{editing ? track?.name : first ? t("newtrack.first") : t("newtrack.title")}</h1>
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

      {#if ready.length === 0}
        <div class="field"><div class="mono none">{t("newtrack.noAgents")}</div></div>
      {:else}
        {@render role(t("newtrack.conductor"), t("newtrack.agent"), agent, (a) => (agent = a as AgentId), false, conductorOptions, conductorConfig, agent)}
        {@render role(t("newtrack.workers"), t("newtrack.workerAgent"), workerAgent, (a) => (workerAgent = a), true, workerOptions, workerConfig, laneAgent)}
      {/if}

      <div class="foot">
        {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
        <span class="grow"></span>
        {#if editing || !first}
          <button class="btn" type="button" onclick={cancel}>{t("newtrack.cancel")}</button>
        {/if}
        <button class="btn btn-acc" type="submit" disabled={!canSubmit}>
          {editing ? (busy ? t("newtrack.saving") : t("newtrack.save")) : busy ? t("newtrack.creating") : t("newtrack.create")}
        </button>
      </div>
    </form>
  </div>
</main>

<!-- One role: which agent, then that agent's options as selects. -->
{#snippet role(
  title: string,
  agentLabelText: string,
  chosen: string,
  pick: (id: string) => void,
  allowSame: boolean,
  options: ConfigOption[],
  config: OptionConfig,
  effectiveAgent: string,
)}
  <section class="role">
    <div class="mlab rolehead">{title}</div>

    <div class="field">
      <span class="mlab-sm">{agentLabelText}</span>
      <div class="agents" role="radiogroup" aria-label={agentLabelText}>
        {#if allowSame}
          <button type="button" class="opt" class:on={chosen === ""} role="radio" aria-checked={chosen === ""} onclick={() => pick("")}>
            <span class="dot"></span>
            <span class="mono">{t("newtrack.sameAsConductor")}</span>
          </button>
        {/if}
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
          onchange={(e) => (config[option.id] = (e.currentTarget as HTMLSelectElement).value)}
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

  .rolehead {
    padding-top: 26px;
    color: var(--hi);
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
    outline: none;
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
