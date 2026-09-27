<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, instanceId, switchInstance, onInstanceStatus } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The header's instance switcher (Kiro Crew's, top left): this PC, then
   * the remote instances by name. Choosing one shows it in this window.
   * Each dot is its link now: green while it answers (the app pings it
   * every 10 s), amber while connected but not answering (reconnecting),
   * grey when not connected. It follows `instance-status` as it happens,
   * and the list is read again every 10 s besides.
   */
  type Host = { id: string; name: string; kind: string; url: string; ssh: string; connected: boolean; online: boolean };
  type Link = "online" | "trying" | "off";
  const link = (h: Host | undefined): Link => (h?.online ? "online" : h?.connected ? "trying" : "off");

  let open = $state(false);
  let hosts = $state<Host[]>([]);
  let el = $state<HTMLDivElement>();

  const current = $derived(hosts.find((h) => h.id === instanceId));

  /** A reconnect of the instance on screen, underway. */
  let reconnecting = false;

  async function load() {
    try {
      hosts = await invoke<Host[]>("remote_hosts");
    } catch (err) {
      store.lastError = String(err);
      return;
    }
    // The instance on screen lost its link (its SSH ended, say): connect
    // again rather than wait for the next thing asked of it.
    const shown = hosts.find((h) => h.id === instanceId);
    if (shown && !shown.connected && !reconnecting) {
      reconnecting = true;
      invoke<Host[]>("remote_host_connect", { id: shown.id })
        .then((h) => (hosts = h))
        .catch(() => {})
        .finally(() => (reconnecting = false));
    }
  }
  onMount(() => {
    void load();
    const every = setInterval(load, 10_000);
    const stop = onInstanceStatus(({ id, online }) => {
      hosts = hosts.map((h) => (h.id === id ? { ...h, online, connected: h.connected || online } : h));
    });
    return () => {
      clearInterval(every);
      void stop.then((f) => f());
    };
  });

  function toggle() {
    open = !open;
    if (open) void load();
  }

  function pick(id: string | null) {
    open = false;
    if (id !== instanceId) switchInstance(id);
  }

  function manage() {
    open = false;
    store.openSettings("remote");
  }

  function onDoc(e: MouseEvent) {
    if (open && el && !el.contains(e.target as Node)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") open = false;
  }
</script>

<svelte:document onmousedown={onDoc} onkeydown={onKey} />

<div class="sw" bind:this={el}>
  <button class="pill" class:remote={instanceId !== null} onclick={toggle} aria-haspopup="menu" aria-expanded={open}>
    {#if instanceId}
      {@const l = link(current)}
      <span class="dot {l}" title={t(`instances.link.${l}`)}></span>
      <span class="label">{current?.name ?? t("instances.remote")}</span>
    {:else}
      <Icon name="home" size={12} />
      <span class="label">{t("instances.local")}</span>
    {/if}
    <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M2 3.5l3 3 3-3" /></svg>
  </button>

  {#if open}
    <div class="menu" role="menu">
      <button class="item" class:on={instanceId === null} role="menuitem" onclick={() => pick(null)}>
        <Icon name="home" size={13} />
        <span class="what">
          <span class="name">{t("instances.local")}</span>
          <span class="sub">{t("instances.localSub")}</span>
        </span>
      </button>
      {#if hosts.length}<div class="sep"></div>{/if}
      {#each hosts as h (h.id)}
        <button class="item" class:on={instanceId === h.id} role="menuitem" onclick={() => pick(h.id)}>
          <span class="dot {link(h)}"></span>
          <span class="what">
            <span class="name">{h.name}</span>
            <span class="sub mono">{h.kind === "direct" ? h.url : h.ssh}</span>
          </span>
          {#if link(h) !== "off"}<span class="state {link(h)}">{t(`instances.link.${link(h)}`)}</span>{/if}
        </button>
      {/each}
      <div class="sep"></div>
      <button class="item manage" role="menuitem" onclick={manage}>{t("instances.manage")}</button>
    </div>
  {/if}
</div>

<style>
  .sw {
    position: relative;
    display: flex;
    align-items: center;
    padding: 0 8px;
  }

  .pill {
    height: 22px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
    font-size: 11.5px;
  }

  .pill:hover,
  .pill[aria-expanded="true"] {
    color: var(--hi);
    background: var(--sel);
  }

  /* Another machine is on screen: say so plainly. */
  .pill.remote {
    color: var(--hi);
    border-color: var(--acc);
  }

  .label {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--idle);
  }

  .dot.online {
    background: var(--ok);
  }

  .dot.trying {
    background: var(--warn);
    animation: blink 1.2s ease-in-out infinite;
  }

  @keyframes blink {
    50% {
      opacity: 0.35;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dot.trying {
      animation: none;
    }
  }

  .menu {
    position: absolute;
    top: 28px;
    left: 8px;
    z-index: 60;
    min-width: 280px;
    padding: 6px 0;
    background: var(--card);
    border: 1px solid var(--lines);
    box-shadow: 0 8px 24px color-mix(in srgb, var(--bg) 60%, transparent);
  }

  .item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    color: var(--dim);
  }

  .item:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .item.on {
    color: var(--hi);
    border-left-color: var(--acc);
  }

  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name {
    font-size: 13px;
    color: var(--hi);
  }

  .sub {
    font-size: 11px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state {
    font-size: 11px;
    color: var(--ok);
  }

  .state.trying {
    color: var(--warn);
  }

  .sep {
    height: 1px;
    margin: 6px 0;
    background: var(--line);
  }

  .manage {
    font-size: 12px;
  }
</style>
