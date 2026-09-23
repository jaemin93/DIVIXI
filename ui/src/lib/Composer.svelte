<script lang="ts">
  import { store, agentLabel } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import { t } from "./i18n.svelte";

  let draft = $state("");
  let agentOpen = $state(false);
  let contextOpen = $state(false);
  let modelOpen = $state(false);
  let filter = $state("");

  function submit(e: Event) {
    e.preventDefault();
    const text = draft;
    draft = "";
    store.send(text);
  }

  const choices = $derived(
    (store.modelOption?.choices ?? []).filter(
      (c) => !filter.trim() || `${c.id} ${c.name}`.toLowerCase().includes(filter.trim().toLowerCase()),
    ),
  );

  const ctx = $derived(store.context);
  const pct = $derived(ctx && ctx.size > 0 ? Math.min(100, (ctx.used / ctx.size) * 100) : 0);

  function k(n: number): string {
    return n >= 1000 ? `${(n / 1000).toFixed(n >= 100000 ? 0 : 1)}k` : String(n);
  }

  function shortPath(p: string | undefined): string {
    if (!p) return "";
    const parts = p.split(/[\\/]/).filter(Boolean);
    return parts.length > 3 ? `…${p.includes("\\") ? "\\" : "/"}${parts.slice(-2).join(p.includes("\\") ? "\\" : "/")}` : p;
  }

</script>

<div class="composer">
  <form onsubmit={submit}>
    <input
      type="text"
      bind:value={draft}
      disabled={store.busy}
      placeholder={store.busy ? t("composer.busy") : t("composer.placeholder")}
      aria-label={t("composer.placeholder")}
    />
    <button class="btn send" type="submit" disabled={store.busy || !draft.trim()} aria-label={t("composer.send")}>→</button>
  </form>

  <div class="status">
    <!-- agent -->
    <Popover bind:open={agentOpen} width={260}>
      {#snippet trigger()}
        <button class="chip" onclick={() => (agentOpen = !agentOpen)} title={t("composer.agentTitle")} aria-haspopup="listbox" aria-expanded={agentOpen}>
          <Icon name="bot" size={14} />
          <span class="mono">{agentLabel(store.agent)}</span>
        </button>
      {/snippet}
      <div class="mlab ph">{t("composer.agent")}</div>
      <div class="list" role="listbox" aria-label={t("composer.agent")}>
        {#each store.agents ?? [] as a (a.kind)}
          <button
            class="opt"
            class:on={store.agent === a.kind}
            role="option"
            aria-selected={store.agent === a.kind}
            disabled={a.readiness !== "ready"}
            onclick={() => {
              store.setTrackAgent(a.kind);
              agentOpen = false;
            }}
          >
            <span class="dot" style="background: {a.readiness === 'ready' ? 'var(--ok)' : 'var(--idle)'}"></span>
            <span class="mono id">{agentLabel(a.kind)}</span>
            <span class="grow"></span>
            {#if store.agent === a.kind}<span class="mono check">✓</span>{/if}
            {#if a.readiness !== "ready"}<span class="mono dim">{a.readiness.replace("_", " ")}</span>{/if}
          </button>
        {/each}
      </div>
    </Popover>

    <span class="chip static" title={store.currentTrack?.cwd}>
      <Icon name="folder" size={14} />
      <span class="mono path">{shortPath(store.currentTrack?.cwd)}</span>
    </span>

    <span class="grow"></span>

    {#if store.lastError}<span class="mono err" title={store.lastError}>{store.lastError}</span>{/if}

    <!-- context -->
    <Popover bind:open={contextOpen} align="right" width={280}>
      {#snippet trigger()}
        <button class="ctx" onclick={() => (contextOpen = !contextOpen)} title={t("composer.contextTitle")} aria-expanded={contextOpen}>
          <span class="bar"><span class="fill" style="width: {pct}%"></span></span>
        </button>
      {/snippet}
      <div class="mlab ph">{t("composer.context")}</div>
      <div class="kv mono">
        {#if ctx}
          <div class="row"><span class="dim">{t("composer.used")}</span><span>{t("composer.tokens", { used: k(ctx.used), size: k(ctx.size) })}</span></div>
          <div class="row"><span class="dim">{t("composer.ratio")}</span><span>{pct.toFixed(1)}%</span></div>
          {#if ctx.cost != null}
            <div class="row"><span class="dim">{t("composer.cost")}</span><span>{ctx.cost.toFixed(4)} {ctx.currency ?? ""}</span></div>
          {/if}
          <div class="row"><span class="dim">{t("composer.source")}</span><span>{store.activeRun ? t("composer.liveRun") : t("composer.lastRun")}</span></div>
        {:else}
          <div class="row dim">{t("composer.noRuns")}</div>
        {/if}
      </div>
    </Popover>

    <!-- model -->
    <Popover bind:open={modelOpen} align="right" width={360}>
      {#snippet trigger()}
        <button class="chip" onclick={() => (modelOpen = !modelOpen)} title={t("composer.modelTitle")} aria-haspopup="listbox" aria-expanded={modelOpen}>
          <span class="mono">{store.modelName || "default"}</span>
        </button>
      {/snippet}
      <input class="filter" type="text" bind:value={filter} placeholder={t("composer.filter")} aria-label={t("composer.modelFilter")} />
      <div class="list" role="listbox" aria-label={t("composer.model")}>
        {#if !store.modelOption}
          <div class="mono dim ph">{t("composer.noModels")}</div>
        {/if}
        {#each choices as c (c.id)}
          <button
            class="opt model"
            class:on={store.modelId === c.id}
            role="option"
            aria-selected={store.modelId === c.id}
            onclick={() => {
              store.setModel(store.agent, c.id);
              modelOpen = false;
            }}
          >
            <span class="line">
              <span class="mono id">{c.id}</span>
              {#if store.modelId === c.id}<span class="mono check">✓</span>{/if}
              <span class="grow"></span>
              {#if c.group}<span class="mono dim">{c.group}</span>{/if}
            </span>
            <span class="desc">{c.name}{c.description ? ` · ${c.description}` : ""}</span>
          </button>
        {/each}
      </div>
      {#if store.modelOption}
        <div class="foot mono">
          <span class="dim">{store.modelOption.name}</span>
          <span class="grow"></span>
          {#if store.models[store.agent]}
            <button class="link" onclick={() => store.setModel(store.agent, "")}>{t("composer.defaultModel")}</button>
          {/if}
        </div>
      {/if}
    </Popover>
  </div>
</div>

<style>
  .composer {
    flex-shrink: 0;
    border-top: 1px solid var(--line);
    background: var(--rail);
    padding: 16px 34px 10px;
  }

  form {
    display: flex;
    gap: 9px;
  }

  input {
    flex: 1;
    height: 44px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 14px;
    padding: 0 14px;
    outline: none;
  }

  input:focus {
    border-color: var(--acc);
  }

  .send {
    width: 44px;
    height: 44px;
    padding: 0;
    font-size: 15px;
    letter-spacing: 0;
    border-color: var(--accln);
    color: var(--acct);
  }

  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 32px;
    margin-top: 6px;
  }

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

  .chip:not(.static):hover {
    color: var(--hi);
    background: var(--sel);
  }

  .chip.static {
    color: var(--lab);
  }

  .path {
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
  }

  .err {
    font-size: 10px;
    color: var(--acct);
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Context: a hairline track, accent fill; no numbers until asked. */
  .ctx {
    height: 26px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    background: transparent;
    border: 0;
  }

  .ctx:hover {
    background: var(--sel);
  }

  .bar {
    width: 72px;
    height: 3px;
    background: var(--lines);
    display: block;
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--acc);
    transition: width 200ms linear;
  }

  .ph {
    padding: 12px 14px 8px;
  }

  .list {
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding-bottom: 6px;
  }

  .opt {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
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

  .opt.model {
    flex-direction: column;
    align-items: stretch;
    gap: 3px;
  }

  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .id {
    font-size: 12px;
    color: inherit;
  }

  .check {
    color: var(--acct);
    font-size: 11px;
  }

  .desc {
    font-size: 11px;
    color: var(--lab);
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

  .filter {
    margin: 10px 14px 6px;
    height: 30px;
    font-size: 12px;
  }

  .kv {
    display: flex;
    flex-direction: column;
    padding: 0 14px 12px;
    gap: 6px;
    font-size: 11px;
    color: var(--txt);
  }

  .row {
    display: flex;
    gap: 12px;
  }

  .row .dim {
    width: 40px;
    flex-shrink: 0;
    font-size: 11px;
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
