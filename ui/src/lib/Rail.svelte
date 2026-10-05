<script lang="ts">
  import { inTauri, local, overWeb } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { kb } from "./knowledge.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import Mark from "./Mark.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t, type Key } from "./i18n.svelte";

  /**
   * The rail's map. Adding a feature is one entry here: an icon, a label,
   * and what selecting it does. `soon` entries are drawn but inert, so the
   * shape of the product is visible before every part of it exists.
   *
   * The rail is only ever one level deep; lists (tracks, workers, agents)
   * live in the column it opens, like Kiro Crew's session panel.
   */
  type Item = { id: string; icon: IconName; label: Key; soon?: boolean; go?: () => void };

  const primary: Item[] = [
    {
      id: "tracks",
      icon: "tracks",
      label: "rail.tracks",
      go: () => {
        // Second click on the active item folds the list away.
        if (store.view !== "settings" && store.view !== "design" && store.view !== "knowledge" && store.trackListOpen) store.setTrackList(false);
        else {
          store.view = store.currentTrack ? "track" : "new-track";
          store.setTrackList(true);
        }
      },
    },
    { id: "design", icon: "draft", label: "rail.design", go: () => void store.showDesigns() },
    {
      id: "knowledge",
      icon: "book",
      label: "rail.knowledge",
      go: () => {
        // As the designs: a second click folds the libraries column away, and back.
        if (store.view === "knowledge") void store.setKbList(!store.kbListOpen);
        else {
          if (!store.kbListOpen) void store.setKbList(true);
          void kb.show();
        }
      },
    },
    { id: "routines", icon: "routines", label: "rail.routines", go: () => void store.showRoutines() },
  ];

  const secondary: Item[] = [
    // A shell on the Divixi shown: this PC, the remote instance (as VS Code
    // Remote), or from a phone the PC that served it.
    ...(inTauri || overWeb ? [{ id: "terminal", icon: "terminal" as IconName, label: "rail.terminal" as Key, go: () => store.setTerminal(!store.termOpen) }] : []),
    // Opening THIS PC's Divixi on a phone. Only from the app: a phone has no
    // business being offered a way to pair another phone.
    ...(local ? [{ id: "phone", icon: "phone" as IconName, label: "rail.phone" as Key, go: () => (store.phoneDialog = true) }] : []),
    { id: "settings", icon: "settings", label: "rail.settings", go: () => store.openSettings("overview") },
  ];

  const collapsed = $derived(store.railCollapsed);

  function active(item: Item): boolean {
    if (item.id === "tracks")
      return store.view !== "settings" && store.view !== "design" && store.view !== "knowledge" && store.view !== "routines";
    if (item.id === "design") return store.view === "design";
    if (item.id === "knowledge") return store.view === "knowledge";
    if (item.id === "routines") return store.view === "routines";
    if (item.id === "settings") return store.view === "settings";
    if (item.id === "terminal") return store.termOpen;
    if (item.id === "phone") return store.phoneDialog;
    return false;
  }

  function badge(item: Item): string {
    if (item.id === "tracks") return store.tracks.length ? String(store.tracks.length) : "";
    if (item.id === "design") return store.designs.length ? String(store.designs.length) : "";
    if (item.id === "knowledge") {
      const n = store.artifacts.filter((a) => a.kind === "knowledge").length;
      return n ? String(n) : "";
    }
    if (item.id === "routines") return store.routines.length ? String(store.routines.length) : "";
    return "";
  }
</script>

<nav class:collapsed style={collapsed ? undefined : `width: ${store.railWidth}px`}>
  {#if !collapsed}
    <SplitHandle edge="right" width={store.railWidth} min={160} max={320} reset={200} label={t("rail.width")} onchange={(px, persist) => store.setRailWidth(px, persist)} />
  {/if}
  <div class="top">
    {#if collapsed}
      <!-- Folded, the mark is the way back out: it opens the rail. -->
      <button class="brand open" onclick={() => store.setRail(false)} title={t("rail.expand")} aria-label={t("rail.expand")} aria-expanded="false">
        <Mark size={16} live={store.anyLive} />
      </button>
    {:else}
      <span class="brand" title="DIVIXI"><Mark size={16} live={store.anyLive} /><span class="mono name">DIVIXI</span></span>
      <span class="grow"></span>
      <button class="toggle" onclick={() => store.setRail(true)} title={t("rail.collapse")} aria-label={t("rail.collapse")} aria-expanded="true">
        <Icon name="collapse" />
      </button>
    {/if}
  </div>

  {#snippet entry(item: Item)}
    <button
      class="item"
      class:active={active(item)}
      class:soon={item.soon}
      disabled={item.soon}
      title={collapsed ? t(item.label) : undefined}
      onclick={item.go}
    >
      <Icon name={item.icon} />
      {#if !collapsed}
        <span class="label">{t(item.label)}</span>
        {#if item.soon}
          <span class="mono tag">{t("rail.soon")}</span>
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
    position: relative;
    width: 200px;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    padding: 8px 0 12px;
    overflow: visible;
    transition: width 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }

  /* Labels are cut by the rail's width while it slides, not spilled over. */
  .top,
  .group {
    overflow: hidden;
  }

  @media (prefers-reduced-motion: reduce) {
    nav {
      transition: none;
    }
  }

  nav.collapsed {
    width: 48px;
  }

  .top {
    height: 36px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 16px;
    margin-bottom: 8px;
  }

  .collapsed .top {
    flex-direction: column;
    height: auto;
    gap: 6px;
    padding: 6px 0 0;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--txt);
  }

  .name {
    font-size: 10px;
    letter-spacing: 0.22em;
  }

  /* The folded rail's mark: a button the size of the rail items below it. */
  .brand.open {
    width: 32px;
    height: 32px;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 0;
  }

  .brand.open:hover {
    background: var(--sel);
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
