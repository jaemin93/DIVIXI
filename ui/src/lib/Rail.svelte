<script lang="ts">
  import { store, agentLabel } from "./store.svelte";
  import Icon, { type IconName } from "./Icon.svelte";

  /**
   * The rail's map. Adding a feature is one entry here: an icon, a label,
   * and what selecting it does. `soon` entries are drawn but inert, so the
   * shape of the product is visible before every part of it exists.
   */
  type Item = { id: string; icon: IconName; label: string; soon?: boolean; go?: () => void };

  const primary: Item[] = [
    { id: "tracks", icon: "tracks", label: "Tracks", go: () => (store.view = "track") },
    { id: "lanes", icon: "lanes", label: "Lanes", go: () => (store.view = "track") },
    { id: "drafts", icon: "draft", label: "Drafts", soon: true },
    { id: "wrapup", icon: "wrapup", label: "Wrap-up", soon: true },
  ];

  const secondary: Item[] = [
    { id: "agents", icon: "agents", label: "Agents", go: () => (store.view = "settings") },
    { id: "settings", icon: "settings", label: "설정", go: () => (store.view = "settings") },
  ];

  const collapsed = $derived(store.railCollapsed);

  function active(item: Item): boolean {
    if (item.id === "tracks" || item.id === "lanes") return store.view === "track";
    if (item.id === "settings" || item.id === "agents") return store.view === "settings";
    return false;
  }

  // Phase 0 runs everything in one lane; the rail shows the shape the
  // domain model already has, not more than the core can back.
  const lanes = $derived([
    {
      name: "solo",
      status: store.activeRun ? "running" : store.runs.length ? "idle" : "unspawned",
      runs: store.runs.length,
    },
  ]);
</script>

<nav class:collapsed>
  <div class="top">
    <button
      class="toggle"
      onclick={() => store.setRail(!collapsed)}
      title={collapsed ? "사이드 패널 펼치기" : "사이드 패널 접기"}
      aria-label={collapsed ? "사이드 패널 펼치기" : "사이드 패널 접기"}
      aria-expanded={!collapsed}
    >
      <Icon name={collapsed ? "expand" : "collapse"} />
    </button>
  </div>

  <div class="group">
    {#each primary as item (item.id)}
      <button
        class="item"
        class:active={active(item)}
        class:soon={item.soon}
        disabled={item.soon}
        title={collapsed ? item.label : undefined}
        onclick={item.go}
      >
        <Icon name={item.icon} />
        {#if !collapsed}
          <span class="label">{item.label}</span>
          {#if item.soon}
            <span class="mono tag">soon</span>
          {:else if item.id === "tracks"}
            <span class="mono count">1</span>
          {:else if item.id === "lanes"}
            <span class="mono count">{lanes.length}</span>
          {/if}
        {/if}
      </button>

      {#if !collapsed && item.id === "tracks"}
        <button class="row" class:on={store.view === "track"} onclick={() => (store.view = "track")}>
          <span class="dot" style="background: var(--ok)"></span>
          <span class="name">ACP 브리지</span>
          <span class="mono count">{store.runs.length}</span>
        </button>
      {/if}

      {#if !collapsed && item.id === "lanes"}
        {#each lanes as lane (lane.name)}
          <div class="row static">
            <span
              class="dot"
              class:pulse={lane.status === "running"}
              style="background: {lane.status === 'running' ? 'var(--ok)' : 'var(--idle)'}"
            ></span>
            <span class="name mono">{lane.name}</span>
            <span class="mono count">{lane.runs}</span>
          </div>
        {/each}
      {/if}
    {/each}
  </div>

  <span class="grow"></span>

  <div class="group bottom">
    {#each secondary as item (item.id)}
      <button
        class="item"
        class:active={active(item)}
        title={collapsed ? item.label : undefined}
        onclick={item.go}
      >
        <Icon name={item.icon} />
        {#if !collapsed}
          <span class="label">{item.label}</span>
          {#if item.id === "agents"}
            <span class="mono count">{store.readyAgents.length}</span>
          {/if}
        {/if}
      </button>

      {#if !collapsed && item.id === "agents"}
        {#each store.agents ?? [] as a (a.kind)}
          <button
            class="row agent mono"
            class:on={store.agent === a.kind}
            disabled={a.readiness !== "ready"}
            onclick={() => (store.agent = a.kind)}
          >
            <span
              class="dot"
              style="background: {a.readiness === 'ready' ? 'var(--ok)' : a.readiness === 'needs_login' || a.readiness === 'needs_download' ? 'var(--warn)' : a.readiness === 'error' ? 'var(--acct)' : 'var(--idle)'}"
            ></span>
            <span class="name">{agentLabel(a.kind)}</span>
            <span class="count">{a.readiness === "ready" ? "acp" : a.readiness === "needs_login" ? "login" : a.readiness === "needs_download" ? "dl" : "—"}</span>
          </button>
        {/each}
      {/if}
    {/each}
  </div>
</nav>

<style>
  nav {
    width: 224px;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    padding: 8px 0 12px;
    transition: width 140ms ease-out;
    overflow: hidden;
  }

  nav.collapsed {
    width: 48px;
  }

  .top {
    display: flex;
    justify-content: flex-end;
    padding: 0 8px 10px;
  }

  .collapsed .top {
    justify-content: center;
    padding: 0 0 10px;
  }

  .toggle {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .toggle:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .group {
    display: flex;
    flex-direction: column;
  }

  .item {
    width: 100%;
    height: 38px;
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 0 16px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    font-size: 13px;
    color: var(--dim);
    white-space: nowrap;
  }

  .collapsed .item {
    justify-content: center;
    padding: 0;
    border-left-width: 0;
  }

  .collapsed .item.active {
    color: var(--hi);
    background: var(--sel);
  }

  .item:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .item.active {
    color: var(--hi);
    border-left-color: var(--acc);
  }

  .item.soon {
    color: var(--lab);
    cursor: default;
    opacity: 0.6;
  }

  .label {
    flex: 1;
  }

  .tag {
    font-size: 9px;
    letter-spacing: 0.16em;
    color: var(--lab);
  }

  .row {
    width: 100%;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px 0 43px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
  }

  .row.static {
    cursor: default;
  }

  .row.on {
    color: var(--hi);
    border-left-color: var(--acc);
    background: var(--sel);
  }

  .row.agent {
    font-size: 11px;
  }

  .row.agent:disabled {
    cursor: default;
    opacity: 0.7;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .count {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .bottom {
    border-top: 1px solid var(--line);
    padding-top: 8px;
  }
</style>
