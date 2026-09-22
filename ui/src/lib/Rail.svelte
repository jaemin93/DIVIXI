<script lang="ts">
  import { store } from "./store.svelte";
  import Icon, { type IconName } from "./Icon.svelte";

  /**
   * The rail's map. Adding a feature is one entry here: an icon, a label,
   * and what selecting it does. `soon` entries are drawn but inert, so the
   * shape of the product is visible before every part of it exists.
   *
   * The rail is only ever one level deep; lists (tracks, lanes, agents)
   * live in the column it opens, like Kiro Crew's session panel.
   */
  type Item = { id: string; icon: IconName; label: string; soon?: boolean; go?: () => void };

  const primary: Item[] = [
    {
      id: "tracks",
      icon: "tracks",
      label: "Tracks",
      go: () => {
        // Second click on the active item folds the list away.
        if (store.view === "track" && store.trackListOpen) store.setTrackList(false);
        else {
          store.view = "track";
          store.setTrackList(true);
        }
      },
    },
    { id: "drafts", icon: "draft", label: "Drafts", soon: true },
    { id: "wrapup", icon: "wrapup", label: "Wrap-up", soon: true },
  ];

  const secondary: Item[] = [
    { id: "agents", icon: "agents", label: "Agents", go: () => store.openSettings("agents") },
    { id: "settings", icon: "settings", label: "설정", go: () => store.openSettings("overview") },
  ];

  const collapsed = $derived(store.railCollapsed);

  function active(item: Item): boolean {
    if (item.id === "tracks") return store.view === "track";
    if (item.id === "settings" || item.id === "agents") return store.view === "settings";
    return false;
  }

  function badge(item: Item): string {
    if (item.id === "tracks") return "1";
    if (item.id === "agents") return String(store.readyAgents.length);
    return "";
  }
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

  {#snippet entry(item: Item)}
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
        {:else if badge(item)}
          <span class="mono count">{badge(item)}</span>
        {/if}
      {/if}
    </button>
  {/snippet}

  <div class="group">
    {#each primary as item (item.id)}
      {@render entry(item)}
    {/each}
  </div>

  <span class="grow"></span>

  <div class="group bottom">
    {#each secondary as item (item.id)}
      {@render entry(item)}
    {/each}
  </div>
</nav>

<style>
  nav {
    width: 200px;
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

  .item:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .item.active {
    color: var(--hi);
    border-left-color: var(--acc);
    background: var(--sel);
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

  .count {
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
