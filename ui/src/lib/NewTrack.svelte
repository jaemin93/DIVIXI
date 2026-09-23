<script lang="ts">
  import { store, agentLabel, type AgentId } from "./store.svelte";
  import Mark from "./Mark.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Making a track: a name, one line of intent, the folder it works in and
   * the agent its conductor runs on. This is also the first screen when
   * nothing exists yet.
   */
  let name = $state("");
  let intent = $state("");
  let cwd = $state("");
  let agent = $state<AgentId>(store.agent);
  let creating = $state(false);
  let nameInput = $state<HTMLInputElement>();

  const first = $derived(store.tracks.length === 0);
  const ready = $derived(store.readyAgents);
  const canCreate = $derived(!!name.trim() && ready.some((a) => a.kind === agent) && !creating);

  // Default folder: the repository the app was launched from.
  $effect(() => {
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

  async function submit(e: Event) {
    e.preventDefault();
    if (!canCreate) return;
    creating = true;
    try {
      const ok = await store.createTrack(name, intent, cwd, agent);
      if (ok) {
        name = "";
        intent = "";
      }
    } finally {
      creating = false;
    }
  }

  function cancel() {
    if (first) return;
    store.view = "track";
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") cancel();
  }
</script>

<svelte:window onkeydown={onKey} />

<main>
  <div class="sheet">
    <div class="mlab">{first ? t("newtrack.first") : t("newtrack.title")}</div>
    <div class="titlerow">
      <Mark size={28} ink="var(--hi)" />
      <h1 class="serif">{first ? t("newtrack.first") : t("newtrack.title")}</h1>
    </div>
    <p class="blurb">{t("newtrack.blurb")}</p>

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

      <div class="field">
        <span class="mlab-sm">{t("newtrack.agent")}</span>
        {#if ready.length === 0}
          <div class="mono none">{t("newtrack.noAgents")}</div>
        {:else}
          <div class="agents" role="radiogroup" aria-label={t("newtrack.agent")}>
            {#each ready as a (a.kind)}
              <button type="button" class="opt" class:on={agent === a.kind} role="radio" aria-checked={agent === a.kind} onclick={() => (agent = a.kind)}>
                <span class="dot"></span>
                <span class="mono">{agentLabel(a.kind)}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <div class="foot">
        {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
        <span class="grow"></span>
        {#if !first}
          <button class="btn" type="button" onclick={cancel}>{t("newtrack.cancel")}</button>
        {/if}
        <button class="btn btn-acc" type="submit" disabled={!canCreate}>{creating ? t("newtrack.creating") : t("newtrack.create")}</button>
      </div>
    </form>
  </div>
</main>

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
    width: min(640px, 100%);
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
    max-width: 560px;
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

  input {
    height: 36px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 13px;
    padding: 0 12px;
    outline: none;
  }

  input.mono {
    font-family: var(--mono);
    font-size: 12px;
  }

  input:focus {
    border-color: var(--acc);
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
