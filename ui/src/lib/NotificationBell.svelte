<script lang="ts">
  import { onMount } from "svelte";
  import { instanceId, switchInstance } from "./ipc.svelte";
  import { inbox, markRead, markAllRead, remove, clearAll, GOTO, type Notice } from "./notify.svelte";
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * The bell at the top right (as Kiro Crew's): how many notices are
   * unread, and the list of them: search, read all, clear, grouped by day;
   * one opens its track, on the Divixi it came from (another webview is
   * told through localStorage, `GOTO`).
   */
  let open = $state(false);
  let query = $state("");
  let el = $state<HTMLDivElement>();

  const unread = $derived(inbox.items.filter((x) => !x.read).length);

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? inbox.items.filter((x) => `${x.title} ${x.body} ${x.instanceName ?? ""}`.toLowerCase().includes(q)) : inbox.items;
  });

  /** Today, yesterday, this week, earlier. */
  function day(at: number): "today" | "yesterday" | "week" | "older" {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    const d = start.getTime();
    if (at >= d) return "today";
    if (at >= d - 86_400_000) return "yesterday";
    if (at >= d - 6 * 86_400_000) return "week";
    return "older";
  }

  const groups = $derived.by(() => {
    const out: { key: ReturnType<typeof day>; items: Notice[] }[] = [];
    for (const n of shown) {
      const k = day(n.at);
      const last = out.at(-1);
      if (last?.key === k) last.items.push(n);
      else out.push({ key: k, items: [n] });
    }
    return out;
  });

  /** Open a notice's track where it lives. */
  function go(n: Notice) {
    markRead(n.id);
    open = false;
    if (n.instance === instanceId) {
      if (n.track) void store.selectTrack(n.track);
      return;
    }
    try {
      localStorage.setItem(GOTO, JSON.stringify({ instance: n.instance, track: n.track, at: Date.now() }));
    } catch {
      // No storage: it just switches.
    }
    switchInstance(n.instance).catch((err) => (store.lastError = String(err)));
  }

  /** A notice opened from another webview, meant for this one. */
  function take(raw: string | null) {
    let g: { instance: string | null; track: string | null; at: number } | null = null;
    try {
      g = raw ? JSON.parse(raw) : null;
    } catch {
      return;
    }
    if (!g || g.instance !== instanceId || Date.now() - g.at > 15_000 || !g.track) return;
    const track = g.track;
    try {
      localStorage.removeItem(GOTO);
    } catch {
      // Left there; it goes stale in 15 s.
    }
    // A fresh webview may not have its tracks yet.
    const tryOpen = (left: number) => {
      if (store.tracks.some((x) => x.id === track)) void store.selectTrack(track);
      else if (left > 0) setTimeout(() => tryOpen(left - 1), 300);
    };
    tryOpen(30);
  }

  onMount(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === GOTO) take(e.newValue);
    };
    window.addEventListener("storage", onStorage);
    // Opened for a notice before this webview existed.
    try {
      take(localStorage.getItem(GOTO));
    } catch {
      // No storage.
    }
    return () => window.removeEventListener("storage", onStorage);
  });

  function onDoc(e: MouseEvent) {
    if (open && el && !el.contains(e.target as Node)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") open = false;
  }
</script>

<svelte:document onmousedown={onDoc} onkeydown={onKey} />

<div class="nb" bind:this={el}>
  <button class="bell" class:on={open} onclick={() => (open = !open)} aria-haspopup="dialog" aria-expanded={open} aria-label={t("notify.title")} title={t("notify.title")}>
    <Icon name="bell" size={15} />
    {#if unread}<span class="badge mono">{unread > 99 ? "99+" : unread}</span>{/if}
  </button>

  {#if open}
    <div class="panel" role="dialog" aria-label={t("notify.title")}>
      <div class="head">
        <span class="title">{t("notify.title")}</span>
        <span class="grow"></span>
        <button class="ib" onclick={markAllRead} disabled={!unread} title={t("notify.readAll")} aria-label={t("notify.readAll")}>
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true"><path d="M1.5 8.5l3 3 6-7M7.5 11.5l1 0 6-7" /></svg>
        </button>
        <button class="ib" onclick={clearAll} disabled={!inbox.items.length} title={t("notify.clear")} aria-label={t("notify.clear")}>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9M7 7v4.5M9 7v4.5" /></svg>
        </button>
      </div>
      <input class="search" bind:value={query} placeholder={t("notify.search")} spellcheck="false" />
      <div class="list">
        {#each groups as g (g.key)}
          <div class="mlab-sm group">{t(`notify.day.${g.key}`)}</div>
          {#each g.items as n (n.id)}
            <div class="item" class:unread={!n.read}>
              <button class="main" onclick={() => go(n)}>
                <span class="kind {n.kind}"><Icon name="bell" size={13} /></span>
                <span class="what">
                  <span class="ntitle">{#if n.instanceName}<span class="inst">{n.instanceName}</span>{/if}{n.title}</span>
                  <span class="body">{n.body}</span>
                </span>
                <span class="side">
                  <span class="when">{whenLabel(n.at, store.now, store.lang)}</span>
                  {#if n.count > 1}<span class="count mono">{n.count}</span>{/if}
                  {#if !n.read}<span class="dot"></span>{/if}
                </span>
              </button>
              <button class="x" onclick={() => remove(n.id)} aria-label={t("notify.remove")} title={t("notify.remove")}><Icon name="close" size={10} /></button>
            </div>
          {/each}
        {:else}
          <div class="empty">{query ? t("notify.noMatch") : t("notify.empty")}</div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .nb {
    position: relative;
    display: flex;
    align-items: stretch;
  }

  .bell {
    position: relative;
    width: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--dim);
  }

  .bell:hover,
  .bell.on {
    color: var(--hi);
    background: var(--sel);
  }

  .badge {
    position: absolute;
    top: 3px;
    right: 5px;
    min-width: 15px;
    height: 15px;
    padding: 0 3px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    color: var(--accon);
    background: var(--acc);
    border-radius: 8px;
  }

  .panel {
    position: absolute;
    top: 34px;
    right: 0;
    z-index: 60;
    width: 380px;
    max-height: min(560px, calc(100vh - 60px));
    display: flex;
    flex-direction: column;
    background: var(--card);
    border: 1px solid var(--lines);
    box-shadow: 0 8px 24px color-mix(in srgb, var(--bg) 60%, transparent);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 12px 10px 8px 16px;
  }

  .title {
    font-size: 14px;
    color: var(--hi);
  }

  .grow {
    flex: 1;
  }

  .ib {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--dim);
  }

  .ib:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .ib:disabled {
    opacity: 0.35;
  }

  .search {
    margin: 0 14px 8px;
    height: 30px;
    padding: 0 10px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 12.5px;
    outline: none;
  }

  .search:focus {
    border-color: var(--acc);
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 8px;
  }

  .group {
    padding: 10px 16px 6px;
  }

  .item {
    display: flex;
    align-items: stretch;
    border-top: 1px solid var(--lineq);
  }

  .item:hover {
    background: var(--sel);
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 4px 10px 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .kind {
    width: 26px;
    height: 26px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line);
    background: var(--inp);
    color: var(--dim);
  }

  .kind.decision {
    color: var(--warn);
  }

  .kind.failed {
    color: var(--deltx);
  }

  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .ntitle {
    font-size: 12.5px;
    color: var(--hi);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item:not(.unread) .ntitle {
    color: var(--txt);
  }

  .inst {
    margin-right: 6px;
    padding: 0 5px;
    border: 1px solid var(--line);
    font-size: 10.5px;
    color: var(--dim);
  }

  .body {
    font-size: 12px;
    line-height: 1.45;
    color: var(--dim);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .side {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
  }

  .when {
    font-size: 11px;
    color: var(--lab);
    white-space: nowrap;
  }

  .count {
    min-width: 16px;
    padding: 0 4px;
    font-size: 10px;
    text-align: center;
    color: var(--dim);
    background: var(--sel);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--acc);
  }

  .x {
    width: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 12px;
    background: transparent;
    border: 0;
    color: var(--lab);
    opacity: 0;
  }

  .item:hover .x {
    opacity: 1;
  }

  .x:hover {
    color: var(--hi);
  }

  .empty {
    padding: 26px 16px;
    font-size: 12.5px;
    color: var(--lab);
    text-align: center;
  }
</style>
