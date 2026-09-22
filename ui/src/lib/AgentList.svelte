<script lang="ts">
  import { store, agentLabel, type AgentStatus, type Readiness } from "./store.svelte";

  /** Status chip text and colour. Colour says state, nothing else. */
  const chips: Record<Readiness, { label: string; color: string }> = {
    ready: { label: "READY", color: "var(--ok)" },
    needs_login: { label: "LOGIN", color: "var(--warn)" },
    needs_download: { label: "DOWNLOAD", color: "var(--warn)" },
    not_installed: { label: "MISSING", color: "var(--idle)" },
    error: { label: "ERROR", color: "var(--acct)" },
  };

  function adapterText(a: AgentStatus): string {
    switch (a.adapter.kind) {
      case "local_script":
        return "adapter · node (local)";
      case "npx":
        return `adapter · npx ${a.adapter.package}`;
      case "cli":
        return "adapter · cli --acp";
      case "binary":
        return "adapter · acp server";
      case "needs_download":
        return "adapter · acp server not downloaded";
      default:
        return `adapter · ${a.adapter.reason ?? "none"}`;
    }
  }

  function version(a: AgentStatus): string {
    return a.probe?.agent_version ?? a.cli?.version ?? "";
  }
</script>

<div class="list">
  {#if store.agents === null && !store.detecting}
    <div class="empty mono">아직 감지하지 않았습니다.</div>
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
            cli · {a.cli.path}
          {:else}
            cli · 없음 · {a.install_hint}
          {/if}
        </div>
        <div class="mono detail">{adapterText(a)}</div>
        {#if a.readiness === "needs_login"}
          <div class="detail hint">
            로그인이 필요합니다.
            {#if a.probe?.auth_methods.length}
              {a.probe.auth_methods.map((m) => m.name).join(" · ")}
            {:else}
              터미널에서 <span class="mono">{a.login_hint}</span>
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
                <button class="btn" disabled={!!busy} onclick={() => store.login(a.kind)}>로그인</button>
              {/if}
            {:else if a.readiness === "needs_download"}
              <button class="btn" disabled={!!busy} onclick={() => store.download(a.kind)}>서버 다운로드</button>
            {:else if store.agent !== a.kind}
              <button class="btn" onclick={() => (store.agent = a.kind)}>기본 에이전트로</button>
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
</style>
