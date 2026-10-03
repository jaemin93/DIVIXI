<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, instanceId, switchInstance, onInstanceStatus } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";
  import { ORDER_KEY, PINS_KEY, STABLE_KEY, arrange, move, nudge, savedOrder, type InstanceRef } from "./instanceOrder";

  /**
   * The header's instance switcher (Kiro Crew's, top left): a chip for each
   * instance and a menu with all of them. Choosing one shows it in this
   * window at once (each instance keeps a warm webview; see remote/client.rs).
   *
   * Each dot is its link now: green while it answers (the app pings it
   * every 10 s), amber while connected but not answering (reconnecting),
   * grey when not connected. It follows `instance-status` as it happens,
   * and the list is read again every 10 s besides. An instance built from
   * other code than this app is marked: its commands may not match.
   *
   * The order is the human's: drag a row in the menu, or Alt with an arrow
   * key while it has the focus. The drag is pointer events, not HTML drag
   * and drop: the window takes native drags for dropping files (Board,
   * Composer), and on Windows that leaves none for the page. Local is one of the list and can
   * sit anywhere; the one on screen is only lit, never moved to the front.
   * See instanceOrder.ts for the rules — the order is this PC's
   * (localStorage, shared by every webview of the app and kept in step
   * through `storage` events).
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
    release: string | null;
    stale: boolean;
    freshness: "unknown" | "same" | "other" | "too_old";
  };
  type Link = "online" | "trying" | "off";
  const link = (h: Host | undefined): Link => (h?.online ? "online" : h?.connected ? "trying" : "off");

  function write(list: InstanceRef[]) {
    try {
      localStorage.setItem(ORDER_KEY, JSON.stringify(list));
    } catch {
      // No storage: the order lasts until the page goes.
    }
  }

  /** The order to start from, and the last of pinning cleared away. */
  function adopt(): InstanceRef[] {
    let stored: string | null = null;
    let pins: string | null = null;
    try {
      stored = localStorage.getItem(ORDER_KEY);
      pins = localStorage.getItem(PINS_KEY);
    } catch {
      return [];
    }
    const list = savedOrder(stored, pins);
    if (stored === null && list.length) write(list);
    try {
      localStorage.removeItem(PINS_KEY);
      localStorage.removeItem(STABLE_KEY);
    } catch {
      // Nothing to clear away, then.
    }
    return list;
  }

  let open = $state(false);
  let hosts = $state<Host[]>([]);
  let saved = $state<InstanceRef[]>(adopt());
  let el = $state<HTMLDivElement>();
  let menu = $state<HTMLDivElement>();
  /** The row a drag started from, and the gap it would land in; -1 is none. */
  let from = $state(-1);
  let gap = $state(-1);
  /** A button held on a row, not yet moved far enough to be a drag. */
  let press: { i: number; x: number; y: number } | null = null;
  /** A drag just ended: the click it ends in is not a pick. */
  let dragged = false;
  /** How far the pointer goes before a press is a drag rather than a click. */
  const SLOP = 4;
  /** The gap the line is drawn in: none where the drop would change nothing. */
  const mark = $derived(gap >= 0 && gap !== from && gap !== from + 1 ? gap : -1);
  /** The last move, for a screen reader to say. */
  let moved = $state("");

  /** The list in the human's order; null is Local. */
  const order = $derived.by(() => {
    const ids = hosts.map((h) => h.id);
    // The one on screen keeps its place even if it has just been deleted.
    if (instanceId && !ids.includes(instanceId)) ids.push(instanceId);
    return arrange(saved, ids);
  });

  const nameOf = (c: InstanceRef): string =>
    c === null ? t("instances.local") : (hosts.find((h) => h.id === c)?.name ?? t("instances.remote"));

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
    // Another webview of the app changed the order.
    const onStorage = (e: StorageEvent) => {
      if (e.key === ORDER_KEY) saved = savedOrder(e.newValue, null);
    };
    window.addEventListener("storage", onStorage);
    return () => {
      clearInterval(every);
      window.removeEventListener("storage", onStorage);
      dragEnd();
      void stop.then((f) => f());
    };
  });

  function toggle() {
    open = !open;
    moved = "";
    if (open) void load();
  }

  function pick(id: string | null) {
    open = false;
    if (id !== instanceId) switchInstance(id).catch((err) => (store.lastError = String(err)));
  }

  /** Keep a new order, and say where the instance that moved ended up. */
  function settle(list: InstanceRef[], what: InstanceRef) {
    if (list === order) return;
    saved = list;
    write(list);
    moved = t("instances.movedTo", { name: nameOf(what), at: list.indexOf(what) + 1, total: list.length });
  }

  function onRowDown(i: number, e: PointerEvent) {
    if (e.button !== 0) return;
    press = { i, x: e.clientX, y: e.clientY };
    window.addEventListener("pointermove", onDragMove);
    window.addEventListener("pointerup", onDragUp);
    window.addEventListener("pointercancel", dragEnd);
  }

  /** The gap under the pointer: the nearer half of a row says which side. */
  function gapAt(y: number): number {
    const rows = menu?.querySelectorAll<HTMLElement>(".row") ?? [];
    for (let k = 0; k < rows.length; k++) {
      const r = rows[k].getBoundingClientRect();
      if (y < r.top + r.height / 2) return k;
    }
    return rows.length;
  }

  function onDragMove(e: PointerEvent) {
    if (!press) return;
    if (from < 0) {
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < SLOP) return;
      from = press.i;
    }
    gap = gapAt(e.clientY);
  }

  function onDragUp() {
    if (from >= 0) {
      dragged = true;
      // Released off any button, no click comes to clear it.
      setTimeout(() => (dragged = false));
      if (gap >= 0) settle(move(order, from, gap), order[from]);
    }
    dragEnd();
  }

  function dragEnd() {
    press = null;
    from = -1;
    gap = -1;
    window.removeEventListener("pointermove", onDragMove);
    window.removeEventListener("pointerup", onDragUp);
    window.removeEventListener("pointercancel", dragEnd);
  }

  /** The click a drag ends in, swallowed before it reaches a row. */
  function onMenuClick(e: MouseEvent) {
    if (!dragged) return;
    dragged = false;
    e.preventDefault();
    e.stopPropagation();
  }

  /** Alt with an arrow key on a row: the same move, without dragging. */
  function onRowKey(i: number, e: KeyboardEvent) {
    if (!e.altKey) return;
    const step = e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
    if (!step) return;
    e.preventDefault();
    e.stopPropagation();
    settle(nudge(order, i, step), order[i]);
  }

  function manage() {
    open = false;
    store.openSettings("remote");
  }

  /**
   * Why the instance is out of step. A server that names no build at all is
   * not "a different build" -- it is one from before that field, which is a
   * different thing to do about it (update it, rather than match commits).
   */
  function staleNote(h: Host): string {
    if (h.freshness === "too_old") return t("instances.tooOld", { version: h.version ?? "?" });
    return t("instances.stale", { version: h.version ?? "?", build: h.build ?? "?" });
  }

  function onDoc(e: MouseEvent) {
    if (open && el && !el.contains(e.target as Node)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (!open || e.key !== "Escape") return;
    // Escape during a drag puts the row back; the menu stays.
    if (press) dragEnd();
    else open = false;
  }
</script>

<svelte:document onmousedown={onDoc} onkeydown={onKey} />

<div class="sw" bind:this={el}>
  <div class="chips">
    {#each order as c (c ?? "local")}
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
  </div>
  <button class="more" onclick={toggle} aria-haspopup="menu" aria-expanded={open} aria-label={t("instances.all")} title={t("instances.all")}>
    <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M2 3.5l3 3 3-3" /></svg>
  </button>

  {#if open}
    <div class="menu" class:dragging={from >= 0} role="menu" tabindex="-1" aria-label={t("instances.all")} bind:this={menu} onclickcapture={onMenuClick}>
      {#each order as c, i (c ?? "local")}
        {@const h = c ? hosts.find((x) => x.id === c) : undefined}
        <div
          class="row"
          class:on={c === instanceId}
          class:lift={from === i}
          class:above={mark === i}
          class:below={mark === order.length && i === order.length - 1}
          role="none"
          onpointerdown={(e) => onRowDown(i, e)}
        >
          <button
            class="grip"
            role="menuitem"
            aria-label={t("instances.reorder", { name: nameOf(c), at: i + 1, total: order.length })}
            aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
            title={t("instances.reorderHint")}
            onkeydown={(e) => onRowKey(i, e)}
          >
            <svg width="10" height="12" viewBox="0 0 10 12" aria-hidden="true" fill="currentColor" stroke="none">
              <circle cx="3" cy="2.5" r="1" /><circle cx="7" cy="2.5" r="1" />
              <circle cx="3" cy="6" r="1" /><circle cx="7" cy="6" r="1" />
              <circle cx="3" cy="9.5" r="1" /><circle cx="7" cy="9.5" r="1" />
            </svg>
          </button>
          <button class="item" role="menuitem" onclick={() => pick(c)} onkeydown={(e) => onRowKey(i, e)}>
            {#if c === null}
              <Icon name="home" size={13} />
              <span class="what">
                <span class="name">{t("instances.local")}</span>
                <span class="sub">{t("instances.localSub")}</span>
              </span>
            {:else}
              <span class="dot {link(h)}"></span>
              <span class="what">
                <span class="name">{h?.name ?? t("instances.remote")}</span>
                {#if h}<span class="sub mono">{h.kind === "direct" ? h.url : h.ssh}</span>{/if}
                {#if h?.stale}<span class="warnline">{staleNote(h)}</span>{/if}
              </span>
              {#if link(h) !== "off"}<span class="state {link(h)}">{t(`instances.link.${link(h)}`)}</span>{/if}
            {/if}
          </button>
        </div>
      {/each}
      <div class="sep"></div>
      <button class="item opt" role="menuitem" onclick={manage}>
        <span class="what"><span class="name">{t("instances.manage")}</span></span>
      </button>
      <p class="said" role="status" aria-live="polite">{moved}</p>
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
    max-width: 60%;
  }

  /* Every instance has a chip, in the human's order: more of them than the
     header holds scrolls sideways rather than squeezing the names. */
  .chips {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .chips::-webkit-scrollbar {
    display: none;
  }

  .chip {
    height: 22px;
    flex-shrink: 0;
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
    flex-shrink: 0;
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

  /* Dragging: the row it came from is lifted, and a line stands in the gap
     it would drop into. */
  .row.lift {
    opacity: 0.4;
  }

  /* No text gets selected on the way, and the whole menu says "moving". */
  .menu.dragging {
    user-select: none;
    cursor: grabbing;
  }

  .menu.dragging .row:hover {
    background: transparent;
  }

  .menu.dragging .item,
  .menu.dragging .grip {
    cursor: grabbing;
  }

  .row.above {
    box-shadow: inset 0 2px 0 var(--acct);
  }

  .row.below {
    box-shadow: inset 0 -2px 0 var(--acct);
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

  /* Lined up with the rows above it, which start with a handle. */
  .item.opt {
    padding-left: 36px;
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

  /* Ordering never switches: its own handle at the row's head. */
  .grip {
    width: 22px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
    opacity: 0.4;
    cursor: grab;
  }

  .row:hover .grip,
  .grip:focus-visible {
    opacity: 1;
    color: var(--hi);
  }

  .grip:active {
    cursor: grabbing;
  }

  .sep {
    height: 1px;
    margin: 6px 0;
    background: var(--line);
  }

  /* Said to a screen reader after a move; not drawn. */
  .said {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
