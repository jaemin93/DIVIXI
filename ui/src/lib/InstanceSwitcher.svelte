<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, instanceId, switchInstance, onInstanceStatus } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The header's instance switcher (Kiro Crew's, top left): chips for Local
   * and the pinned instances (and the one on screen), a menu with all of
   * them. Choosing one shows it in this window at once (each instance keeps
   * a warm webview; see remote/client.rs).
   *
   * Each dot is its link now: green while it answers (the app pings it
   * every 10 s), amber while connected but not answering (reconnecting),
   * grey when not connected. It follows `instance-status` as it happens,
   * and the list is read again every 10 s besides. An instance built from
   * other code than this app is marked: its commands may not match.
   *
   * Pins and "keep tab order" are this PC's (localStorage, shared by every
   * webview of the app and kept in step through `storage` events). Kept
   * order: chips stay where they are and the one on screen is only lit;
   * otherwise (the default) the one on screen comes first.
   */
  type Host = {
    id: string;
    name: string;
    kind: string;
    url: string;
    ssh: string;
    connected: boolean;
    online: boolean;
    version: string | null;
    build: string | null;
    stale: boolean;
  };
  type Link = "online" | "trying" | "off";
  const link = (h: Host | undefined): Link => (h?.online ? "online" : h?.connected ? "trying" : "off");

  const PINS = "divixi.pins";
  const STABLE = "divixi.stableOrder";

  function read<T>(key: string, fallback: T): T {
    try {
      const v = localStorage.getItem(key);
      return v === null ? fallback : (JSON.parse(v) as T);
    } catch {
      return fallback;
    }
  }
  function write(key: string, value: unknown) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      // No storage: it lasts until the page goes.
    }
  }

  let open = $state(false);
  let hosts = $state<Host[]>([]);
  let pins = $state<string[]>(read(PINS, []));
  let stable = $state<boolean>(read(STABLE, false));
  let el = $state<HTMLDivElement>();

  const current = $derived(hosts.find((h) => h.id === instanceId));

  /** The chips: null is Local. */
  const chips = $derived.by(() => {
    const known = new Set(hosts.map((h) => h.id));
    const list: (string | null)[] = [null, ...pins.filter((p) => known.has(p))];
    if (instanceId && !list.includes(instanceId)) list.push(instanceId);
    if (!stable) {
      const at = list.indexOf(instanceId);
      if (at > 0) list.unshift(...list.splice(at, 1));
    }
    return list;
  });

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
    // Another webview of the app changed the pins or the order.
    const onStorage = (e: StorageEvent) => {
      if (e.key === PINS) pins = read(PINS, []);
      if (e.key === STABLE) stable = read(STABLE, false);
    };
    window.addEventListener("storage", onStorage);
    return () => {
      clearInterval(every);
      window.removeEventListener("storage", onStorage);
      void stop.then((f) => f());
    };
  });

  function toggle() {
    open = !open;
    if (open) void load();
  }

  function pick(id: string | null) {
    open = false;
    if (id !== instanceId) switchInstance(id).catch((err) => (store.lastError = String(err)));
  }

  function pin(id: string) {
    pins = pins.includes(id) ? pins.filter((p) => p !== id) : [...pins, id];
    write(PINS, pins);
  }

  function keepOrder() {
    stable = !stable;
    write(STABLE, stable);
  }

  function manage() {
    open = false;
    store.openSettings("remote");
  }

  function staleNote(h: Host): string {
    return t("instances.stale", { version: h.version ?? "?", build: h.build ?? "?" });
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
  {#each chips as c (c ?? "local")}
    {@const h = c ? hosts.find((x) => x.id === c) : undefined}
    <button class="chip" class:on={c === instanceId} onclick={() => pick(c)} title={h ? (h.stale ? staleNote(h) : t(`instances.link.${link(h)}`)) : t("instances.localSub")}>
      {#if c === null}
        <Icon name="home" size={12} />
        <span class="label">{t("instances.local")}</span>
      {:else}
        <span class="dot {link(h)}"></span>
        <span class="label">{h?.name ?? t("instances.remote")}</span>
        {#if h?.stale}<span class="stale" aria-label={staleNote(h)}>!</span>{/if}
      {/if}
    </button>
  {/each}
  <button class="more" onclick={toggle} aria-haspopup="menu" aria-expanded={open} aria-label={t("instances.all")} title={t("instances.all")}>
    <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M2 3.5l3 3 3-3" /></svg>
  </button>

  {#if open}
    <div class="menu" role="menu">
      <div class="row" class:on={instanceId === null}>
        <button class="item" role="menuitem" onclick={() => pick(null)}>
          <Icon name="home" size={13} />
          <span class="what">
            <span class="name">{t("instances.local")}</span>
            <span class="sub">{t("instances.localSub")}</span>
          </span>
        </button>
      </div>
      {#if hosts.length}<div class="sep"></div>{/if}
      {#each hosts as h (h.id)}
        {@const pinned = pins.includes(h.id)}
        <div class="row" class:on={instanceId === h.id}>
          <button class="item" role="menuitem" onclick={() => pick(h.id)}>
            <span class="dot {link(h)}"></span>
            <span class="what">
              <span class="name">{h.name}</span>
              <span class="sub mono">{h.kind === "direct" ? h.url : h.ssh}</span>
              {#if h.stale}<span class="warnline">{staleNote(h)}</span>{/if}
            </span>
            {#if link(h) !== "off"}<span class="state {link(h)}">{t(`instances.link.${link(h)}`)}</span>{/if}
          </button>
          <button class="pin" class:pinned onclick={() => pin(h.id)} aria-pressed={pinned} title={pinned ? t("instances.unpin") : t("instances.pin")} aria-label={pinned ? t("instances.unpin") : t("instances.pin")}>
            <Icon name="pin" size={12} />
          </button>
        </div>
      {/each}
      <div class="sep"></div>
      <button class="item opt" role="menuitemcheckbox" aria-checked={stable} onclick={keepOrder}>
        <span class="check">{stable ? "✓" : ""}</span>
        <span class="what"><span class="name">{t("instances.keepOrder")}</span></span>
      </button>
      <button class="item opt" role="menuitem" onclick={manage}>
        <span class="check"></span>
        <span class="what"><span class="name">{t("instances.manage")}</span></span>
      </button>
    </div>
  {/if}
</div>

<style>
  .sw {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    min-width: 0;
  }

  .chip {
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

  .chip:hover {
    color: var(--hi);
    background: var(--sel);
  }

  /* On screen: said plainly. */
  .chip.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .label {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .stale {
    width: 13px;
    height: 13px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 9.5px;
    font-weight: 700;
    color: var(--bg);
    background: var(--warn);
  }

  .more {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid transparent;
    color: var(--dim);
  }

  .more:hover,
  .more[aria-expanded="true"] {
    color: var(--hi);
    background: var(--sel);
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
    min-width: 300px;
    padding: 6px 0;
    background: var(--card);
    border: 1px solid var(--lines);
    box-shadow: 0 8px 24px color-mix(in srgb, var(--bg) 60%, transparent);
  }

  .row {
    display: flex;
    align-items: stretch;
    border-left: 2px solid transparent;
  }

  .row.on {
    border-left-color: var(--acc);
  }

  .row:hover {
    background: var(--sel);
  }

  .item {
    flex: 1;
    min-width: 0;
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px 8px 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .item.opt:hover {
    background: var(--sel);
  }

  .item:hover,
  .row.on .item {
    color: var(--hi);
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

  .warnline {
    font-size: 11px;
    line-height: 1.4;
    color: var(--warn);
  }

  .state {
    font-size: 11px;
    color: var(--ok);
  }

  .state.trying {
    color: var(--warn);
  }

  /* Pinning never switches: its own button at the row's end. */
  .pin {
    width: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
    opacity: 0.45;
  }

  .row:hover .pin,
  .pin.pinned {
    opacity: 1;
  }

  .pin.pinned {
    color: var(--acct);
  }

  .pin:hover {
    color: var(--hi);
  }

  .check {
    width: 13px;
    font-size: 12px;
    color: var(--acct);
    text-align: center;
  }

  .sep {
    height: 1px;
    margin: 6px 0;
    background: var(--line);
  }
</style>
