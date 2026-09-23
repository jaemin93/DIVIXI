<script lang="ts">
  import { store, agentLabel, laneAgentOf, type Role } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import { t } from "./i18n.svelte";

  /**
   * One chip, one panel, for who runs what: the conductor's agent and
   * model, and the workers'. A role switch at the top, agents on the left,
   * the chosen agent's models on the right, the track's full settings a
   * click away. Kiro Crew's agent menu, folded around our two roles.
   */
  let open = $state(false);
  let role = $state<Role>("conductor");
  let filter = $state("");

  const track = $derived(store.currentTrack);
  const agent = $derived(store.roleAgent(role));
  const config = $derived(store.roleConfig(role));
  const options = $derived(store.optionsOf(agent));
  const modelOption = $derived(options.find((o) => o.category === "model"));
  const modeOption = $derived(options.find((o) => o.category === "mode"));
  /** Workers follow the conductor unless the track names an agent for them. */
  const sameAsConductor = $derived(role === "worker" && !track?.worker_agent);

  const modelInEffect = $derived(modelOption ? store.effective(agent, config, modelOption) : "");
  const chosenModel = $derived(modelOption ? (config[modelOption.id] ?? "") : "");
  const modeInEffect = $derived(modeOption ? store.effective(agent, config, modeOption) : store.autonomousModeOf(agent));

  const models = $derived(
    (modelOption?.choices ?? []).filter((c) => {
      const q = filter.trim().toLowerCase();
      return !q || `${c.id} ${c.name} ${c.group ?? ""}`.toLowerCase().includes(q);
    }),
  );

  /** Mode ids like `…/session-modes#autopilot` read as their last segment. */
  function modeKey(id: string): string {
    return id.replace(/^.*[#/]/, "");
  }

  /** What the chip says: the conductor, and the workers when they differ. */
  const summary = $derived.by(() => {
    if (!track) return { conductor: agentLabel(store.agent), model: "", workers: "" };
    const cOpts = store.optionsOf(track.agent);
    const cModel = cOpts.find((o) => o.category === "model");
    const model = cModel ? store.choiceName(cModel, store.effective(track.agent, track.conductor_config, cModel)) : "";
    const wAgent = laneAgentOf(track);
    const wOpts = store.optionsOf(wAgent);
    const wModel = wOpts.find((o) => o.category === "model");
    const wModelName = wModel ? store.choiceName(wModel, store.effective(wAgent, track.worker_config, wModel)) : "";
    const differs = wAgent !== track.agent || (wModel && wModelName !== model);
    return {
      conductor: agentLabel(track.agent),
      model,
      workers: differs ? `${agentLabel(wAgent)}${wModelName ? ` · ${wModelName}` : ""}` : "",
    };
  });

  function pickAgent(id: string) {
    void store.setRoleAgent(role, id);
  }

  function pickModel(id: string) {
    if (modelOption) void store.setRoleOption(role, modelOption.id, id);
  }

  function openSettings() {
    open = false;
    store.view = "edit-track";
  }
</script>

<Popover bind:open width={520}>
  {#snippet trigger()}
    <button class="chip" onclick={() => (open = !open)} title={t("picker.title")} aria-haspopup="dialog" aria-expanded={open}>
      <Icon name="bot" size={14} />
      <!-- One line, cut with an ellipsis when the column is narrow; never wrapped. -->
      <span class="label mono">
        {summary.conductor}{#if summary.model}<span class="dim">{summary.model}</span>{/if}{#if summary.workers}<span class="sep">/</span><span class="dim">{t("picker.worker")}</span><span>{summary.workers}</span>{/if}
      </span>
    </button>
  {/snippet}

  <div class="head">
    <div class="roles" role="tablist">
      <button class="role mono" class:on={role === "conductor"} role="tab" aria-selected={role === "conductor"} onclick={() => (role = "conductor")}>{t("picker.conductor")}</button>
      <button class="role mono" class:on={role === "worker"} role="tab" aria-selected={role === "worker"} onclick={() => (role = "worker")}>{t("picker.worker")}</button>
    </div>
    <span class="grow"></span>
    {#if modeInEffect}
      <span class="mono modetag" title={t("composer.modeTitle")}>{t("picker.mode")} · {modeKey(modeInEffect)}</span>
    {/if}
  </div>

  <input class="filter" type="text" bind:value={filter} placeholder={t("composer.filter")} aria-label={t("composer.modelFilter")} />

  <div class="cols">
    <!-- agents -->
    <div class="col agents">
      <div class="mlab-sm colhead">{t("picker.agent")}</div>
      {#if role === "worker"}
        <button class="opt" class:on={sameAsConductor} onclick={() => pickAgent("")}>
          <span class="dot" style="background: {sameAsConductor ? 'var(--ok)' : 'var(--idle)'}"></span>
          <span class="col2">
            <span class="mono id">{t("picker.same")}</span>
            <span class="desc">{t("picker.sameNote")}</span>
          </span>
          {#if sameAsConductor}<span class="mono check">✓</span>{/if}
        </button>
      {/if}
      {#each store.agents ?? [] as a (a.kind)}
        {@const on = role === "worker" ? !sameAsConductor && agent === a.kind : agent === a.kind}
        <button class="opt" class:on disabled={a.readiness !== "ready"} onclick={() => pickAgent(a.kind)}>
          <span class="dot" style="background: {a.readiness === 'ready' ? 'var(--ok)' : 'var(--idle)'}"></span>
          <span class="col2">
            <span class="mono id">{agentLabel(a.kind)}</span>
            <span class="desc">{a.readiness === "ready" ? modeKey(store.autonomousModeOf(a.kind)) || a.name : a.readiness.replace("_", " ")}</span>
          </span>
          {#if on}<span class="mono check">✓</span>{/if}
        </button>
      {/each}
    </div>

    <!-- models of the chosen agent -->
    <div class="col models">
      <div class="mlab-sm colhead">{t("picker.model")} <span class="mono agentname">{agentLabel(agent)}</span></div>
      {#if !modelOption}
        <div class="mono none">{t("picker.noModels")}</div>
      {:else}
        <button class="opt" class:on={!chosenModel} disabled={sameAsConductor} onclick={() => pickModel("")}>
          <span class="col2">
            <span class="mono id">{t("picker.default")}</span>
            <span class="desc">{store.choiceName(modelOption, modelOption.current)}</span>
          </span>
          {#if !chosenModel}<span class="mono check">✓</span>{/if}
        </button>
        {#each models as c (c.id)}
          <button class="opt" class:on={chosenModel === c.id} disabled={sameAsConductor} onclick={() => pickModel(c.id)}>
            <span class="col2">
              <span class="mono id">{c.name}</span>
              <span class="desc">{c.id}{c.group ? ` · ${c.group}` : ""}{c.description ? ` · ${c.description}` : ""}</span>
            </span>
            {#if chosenModel === c.id}<span class="mono check">✓</span>{:else if modelInEffect === c.id}<span class="mono dim">now</span>{/if}
          </button>
        {/each}
      {/if}
    </div>
  </div>

  <div class="foot mono">
    <span class="dim">{t("picker.applies")}</span>
    <span class="grow"></span>
    <button class="link" onclick={openSettings}>{t("picker.settings")}</button>
  </div>
</Popover>

<style>
  .chip {
    height: 26px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 0 8px;
    background: transparent;
    border: 0;
    color: var(--dim);
    font-size: 10px;
    letter-spacing: 0.12em;
  }

  .chip:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .chip {
    min-width: 0;
    flex-shrink: 1;
    white-space: nowrap;
  }

  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .label > span {
    margin-left: 7px;
  }

  .chip .dim,
  .sep {
    color: var(--lab);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px 0;
  }

  .roles {
    display: flex;
    border: 1px solid var(--line);
  }

  .role {
    height: 24px;
    padding: 0 12px;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    color: var(--dim);
    font-size: 10px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .role:last-child {
    border-right: 0;
  }

  .role.on {
    color: var(--hi);
    background: var(--sel);
  }

  .modetag {
    font-size: 9px;
    letter-spacing: 0.12em;
    color: var(--lab);
    border: 1px solid var(--line);
    padding: 2px 6px;
  }

  .grow {
    flex: 1;
  }

  .filter {
    margin: 10px 14px 6px;
    height: 30px;
    font-size: 12px;
    width: calc(100% - 28px);
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    padding: 0 10px;
    outline: none;
  }

  .filter:focus {
    border-color: var(--acc);
  }

  .cols {
    display: grid;
    grid-template-columns: 200px 1fr;
    border-top: 1px solid var(--line);
    max-height: 320px;
    min-height: 0;
  }

  .col {
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding-bottom: 6px;
  }

  .agents {
    border-right: 1px solid var(--line);
  }

  .colhead {
    padding: 10px 14px 4px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .agentname {
    font-size: 9px;
    letter-spacing: 0.1em;
    color: var(--lab);
    text-transform: none;
  }

  .opt {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 14px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    color: var(--dim);
    font-size: 12px;
  }

  .opt:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .opt.on {
    color: var(--hi);
    border-left-color: var(--acc);
  }

  .opt:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .col2 {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .id {
    font-size: 12px;
    color: inherit;
  }

  .desc {
    font-size: 10px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .check {
    color: var(--acct);
    font-size: 11px;
  }

  .dim {
    color: var(--lab);
    font-size: 10px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .none {
    padding: 8px 14px;
    font-size: 10px;
    color: var(--lab);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px 10px;
    border-top: 1px solid var(--line);
    font-size: 10px;
  }

  .link {
    background: transparent;
    border: 0;
    color: var(--acct);
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.1em;
    padding: 0;
  }
</style>
