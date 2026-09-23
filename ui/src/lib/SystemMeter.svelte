<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The title bar's pill: a dot for how the machine is holding up and how
   * many agent sessions are working out of those open. Hover or click opens
   * the readings: CPU, memory, the disk of the open track's folder,
   * sessions. Polled every few seconds while the window is visible; a
   * reading that stops arriving is called stale rather than shown as current.
   */
  type Metrics = {
    cpu: number;
    mem_used: number;
    mem_total: number;
    disk_used: number;
    disk_total: number;
    disk_mount: string;
    sessions: number;
    working: number;
  };

  const POLL_MS = 3000;
  const STALE_MS = 12_000;

  let m = $state<Metrics | null>(null);
  let lastOk = $state(0);
  let failed = $state(false);
  let now = $state(Date.now());
  let open = $state(false);
  let pinned = $state(false);
  let root = $state<HTMLDivElement>();

  async function poll() {
    now = Date.now();
    if (document.visibilityState !== "visible") return;
    try {
      m = await invoke<Metrics>("system_metrics", { track: store.track || null });
      lastOk = Date.now();
      failed = false;
    } catch {
      failed = true;
    }
  }

  onMount(() => {
    void poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  const stale = $derived(lastOk === 0 ? failed : now - lastOk > STALE_MS);
  const memPct = $derived(m && m.mem_total ? (m.mem_used / m.mem_total) * 100 : 0);
  const diskPct = $derived(m && m.disk_total ? (m.disk_used / m.disk_total) * 100 : 0);
  const cpuPct = $derived(m?.cpu ?? 0);

  type Level = "ok" | "warn" | "bad";
  function level(pct: number): Level {
    if (pct >= 90) return "bad";
    if (pct >= 70) return "warn";
    return "ok";
  }
  const worst = $derived.by((): Level => {
    const levels = [level(cpuPct), level(memPct), level(diskPct)];
    if (levels.includes("bad")) return "bad";
    if (levels.includes("warn")) return "warn";
    return "ok";
  });
  const dotClass = $derived(stale || !m ? "idle" : worst);

  const GB = 1024 ** 3;
  function gb(bytes: number, digits: number): string {
    return (bytes / GB).toFixed(digits);
  }

  function close() {
    pinned = false;
    open = false;
  }
  function onDocClick(e: MouseEvent) {
    if (pinned && root && !root.contains(e.target as Node)) close();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={onKey} />

<div class="meter" bind:this={root} onmouseenter={() => (open = true)} onmouseleave={() => (open = pinned)} role="presentation">
  <button
    class="pill"
    class:on={open}
    onclick={() => {
      pinned = !pinned;
      open = pinned;
    }}
    aria-label={t("meter.title")}
    aria-expanded={open}
  >
    <span class="dot {dotClass}"></span>
    <svg class="wave" width="14" height="12" viewBox="0 0 14 12" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round">
      <path d="M1 6h1.5M4 3v6M6.5 1v10M9 3.5v5M11.5 6h1.5" />
    </svg>
    {#if m}
      <span class="sep"></span>
      <span class="mono sess" class:busy={m.working > 0} title={t("meter.sessionsValue", { open: m.sessions, working: m.working })}>{m.working}/{m.sessions}</span>
    {/if}
  </button>

  {#if open}
    <div class="panel" role="dialog" aria-label={t("meter.title")}>
      <div class="title">{t("meter.title")}</div>
      {#if m}
        <div class="row mono">
          <span class="k">CPU</span><span class="grow"></span>
          <span class="v {level(cpuPct)}">{Math.round(cpuPct)}%</span>
        </div>
        <div class="row mono">
          <span class="k">MEM</span><span class="grow"></span>
          <span class="d">{gb(m.mem_used, 1)}/{gb(m.mem_total, 1)}GB</span>
          <span class="v {level(memPct)}">{Math.round(memPct)}%</span>
        </div>
        <div class="row mono" title={m.disk_mount}>
          <span class="k">DSK</span><span class="grow"></span>
          <span class="d">{gb(m.disk_used, 0)}/{gb(m.disk_total, 0)}GB</span>
          <span class="v {level(diskPct)}">{Math.round(diskPct)}%</span>
        </div>
        <div class="row mono">
          <span class="k">SES</span><span class="grow"></span>
          <span class="d">{t("meter.sessionsValue", { open: m.sessions, working: m.working })}</span>
        </div>
      {/if}
      {#if stale}
        <div class="stale">{t("meter.stale")}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .meter {
    position: relative;
    display: flex;
    align-items: center;
    padding: 0 10px;
  }

  .pill {
    height: 22px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
  }

  .pill:hover,
  .pill.on {
    border-color: var(--lines);
    color: var(--hi);
    background: var(--sel);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }

  .dot.warn {
    background: var(--warn);
  }

  .dot.bad {
    background: var(--acct);
  }

  .dot.idle {
    background: var(--idle);
  }

  .sep {
    width: 1px;
    height: 12px;
    background: var(--lines);
  }

  .sess {
    font-size: 10px;
    letter-spacing: 0.06em;
  }

  .sess.busy {
    color: var(--ok);
  }

  .panel {
    position: absolute;
    top: calc(100% + 4px);
    right: 10px;
    z-index: 50;
    width: 300px;
    padding: 12px 16px;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  .title {
    margin-bottom: 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--hi);
  }

  .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    height: 26px;
    font-size: 11px;
  }

  .k {
    color: var(--dim);
    letter-spacing: 0.06em;
  }

  .grow {
    flex: 1;
  }

  .d {
    color: var(--lab);
  }

  .v {
    min-width: 34px;
    text-align: right;
    color: var(--body);
  }

  .v.warn {
    color: var(--warn);
    font-weight: 600;
  }

  .v.bad {
    color: var(--acct);
    font-weight: 600;
  }

  .stale {
    margin-top: 6px;
    font-size: 11px;
    line-height: 1.5;
    color: var(--warn);
  }
</style>
