<script lang="ts">
  import { store, agentLabel, type AgentStatus, type Readiness } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /** Status chip text and colour. Colour says state, nothing else. */
  const chips = $derived<Record<Readiness, { label: string; color: string }>>({
    ready: { label: t("agents.state.ready"), color: "var(--ok)" },
    needs_login: { label: t("agents.state.needs_login"), color: "var(--warn)" },
    needs_download: { label: t("agents.state.needs_download"), color: "var(--warn)" },
    not_installed: { label: t("agents.state.not_installed"), color: "var(--idle)" },
    error: { label: t("agents.state.error"), color: "var(--acct)" },
  });

  function adapterText(a: AgentStatus): string {
    switch (a.adapter.kind) {
      case "local_script":
        return t("agents.adapterLocal");
      case "npx":
        return t("agents.adapterNpx", { pkg: a.adapter.package ?? "" });
      case "cli":
        return t("agents.adapterCli");
      case "binary":
        return t("agents.adapterServer");
      case "needs_download":
        return t("agents.adapterNeedsDownload");
      default:
        return t("agents.adapterNone", { reason: a.adapter.reason ?? "none" });
    }
  }

  function version(a: AgentStatus): string {
    return a.probe?.agent_version ?? a.cli?.version ?? "";
  }

  function mb(n: number): string {
    return `${(n / 1048576).toFixed(1)} MB`;
  }

  /** 0–100 when the total is known, null for an indeterminate bar. */
  function percent(p: { received: number; total: number | null; phase: string }): number | null {
    if (p.phase === "unpacking" || p.phase === "done") return 100;
    if (!p.total) return null;
    return Math.min(100, Math.round((p.received / p.total) * 100));
  }
</script>

<div class="list">
  {#if store.detecting}
    <div class="detecting" class:tall={store.agents === null}>
      <span class="spinner" aria-hidden="true"></span>
      <span class="mono">{t("agents.detecting")}</span>
    </div>
  {:else if store.agents === null}
    <div class="empty mono">{t("agents.notYet")}</div>
  {/if}

  {#each store.agents ?? [] as a (a.kind)}
    {@const chip = chips[a.readiness]}
    {@const busy = store.working[a.kind]}
    <div class="row" class:selected={store.agent === a.kind && a.readiness === "ready"}>
      <span class="dot" style="background: {chip.color}"></span>
      <div class="main">
        <div class="head">
          <span class="name">{a.name}</span>
          <span class="mono id">{agentLabel(a.kind)}</span>
          {#if version(a)}<span class="mono ver">{version(a)}</span>{/if}
          <span class="grow"></span>
          <span class="mono chip" style="color: {chip.color}">{busy ?? chip.label}</span>
        </div>
        <div class="mono detail">
          {#if a.cli}
            {t("agents.cli")} · {a.cli.path}
          {:else}
            {t("agents.cli")} · {t("agents.cliMissing")} · {a.install_hint}
          {/if}
        </div>
        <div class="mono detail">{adapterText(a)}</div>
        {#if store.downloads[a.kind]}
          {@const d = store.downloads[a.kind]}
          {@const pct = percent(d)}
          <div class="dl">
            <div class="bar" class:indeterminate={pct === null}>
              <div class="fill" style="width: {pct ?? 30}%"></div>
            </div>
            <div class="mono dltext">
              {#if d.phase === "downloading"}
                {pct === null ? mb(d.received) : `${pct}% · ${mb(d.received)} / ${mb(d.total ?? 0)}`}
              {:else if d.phase === "unpacking"}
                {t("agents.unpacking")} · {mb(d.received)}
              {:else}
                {t("agents.downloadDone")}
              {/if}
            </div>
          </div>
        {/if}
        {#if a.readiness === "needs_login"}
          <div class="detail hint">
            {t("agents.needsLogin")}
            {#if a.probe?.auth_methods.length}
              {a.probe.auth_methods.map((m) => m.name).join(" · ")}
            {:else}
              {t("agents.inTerminal")} <span class="mono">{a.login_hint}</span>
            {/if}
          </div>
        {/if}
        {#if a.readiness === "error" && a.error}
          <div class="detail err">{a.error}</div>
        {/if}
        {#if a.readiness === "ready" || a.readiness === "needs_login" || a.readiness === "needs_download"}
          <div class="actions">
            {#if a.readiness === "needs_login"}
              {#each a.probe?.auth_methods ?? [] as m (m.id)}
                <button class="btn" disabled={!!busy} onclick={() => store.login(a.kind, m.id)}>{m.name}</button>
              {/each}
              {#if !a.probe?.auth_methods.length}
                <button class="btn" disabled={!!busy} onclick={() => store.login(a.kind)}>{t("agents.login")}</button>
              {/if}
            {:else if a.readiness === "needs_download"}
              <button class="btn" disabled={!!busy} onclick={() => store.download(a.kind)}>{t("agents.download")}</button>
            {:else if store.agent !== a.kind}
              <button class="btn" onclick={() => (store.agent = a.kind)}>{t("agents.makeDefault")}</button>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  {/each}
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
  }

  .empty {
    padding: 18px 0;
    font-size: 11px;
    color: var(--lab);
  }

  .detecting {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 18px 0;
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--lab);
  }

  /* Empty list: the spinner owns the space. */
  .detecting.tall {
    flex-direction: column;
    padding: 96px 0;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 1px solid var(--lines);
    border-top-color: var(--acc);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }

  .tall .spinner {
    width: 28px;
    height: 28px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .row {
    display: flex;
    gap: 14px;
    padding: 16px 0;
    border-top: 1px solid var(--line);
    border-left: 2px solid transparent;
  }

  .row.selected {
    border-left-color: var(--acc);
    padding-left: 14px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    margin-top: 6px;
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .name {
    font-size: 15px;
    color: var(--hi);
  }

  .id,
  .ver {
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .chip {
    font-size: 10px;
    letter-spacing: 0.18em;
  }

  .detail {
    font-size: 11px;
    line-height: 1.6;
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint {
    font-family: var(--sans);
    font-size: 12px;
    color: var(--body);
    white-space: normal;
  }

  .err {
    font-family: var(--sans);
    font-size: 12px;
    color: var(--acct);
    white-space: normal;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }

  .dl {
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* Hairline track, accent fill. No radius. */
  .bar {
    height: 3px;
    background: var(--line);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--acc);
    transition: width 120ms linear;
  }

  .indeterminate .fill {
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(340%);
    }
  }

  .dltext {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--lab);
  }
</style>
