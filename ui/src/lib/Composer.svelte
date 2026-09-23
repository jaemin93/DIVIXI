<script lang="ts">
  import { store } from "./store.svelte";
  import AgentPicker from "./AgentPicker.svelte";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import { t } from "./i18n.svelte";

  let draft = $state("");
  let contextOpen = $state(false);

  function submit(e: Event) {
    e.preventDefault();
    const text = draft;
    draft = "";
    store.send(text);
  }

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
    <AgentPicker />

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
    color: var(--lab);
    width: 40px;
    flex-shrink: 0;
    font-size: 11px;
  }
</style>
