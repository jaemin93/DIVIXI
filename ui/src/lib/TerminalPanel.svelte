<script lang="ts">
  import { invoke, listen } from "./ipc.svelte";
  import { onMount, tick } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import "./xterm-css.js";
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The bottom panel: real shells in tabs, in the open track's folder.
   * The core runs each on a pseudo-terminal and streams its output here;
   * xterm.js draws it. The panel stays mounted while hidden so a folded
   * terminal keeps its shell and its scrollback.
   */
  type Tab = {
    /** The core's terminal id; 0 until it answers. */
    id: number;
    key: number;
    title: string;
    exited: boolean;
    term: Terminal;
    fit: FitAddon;
    el: HTMLDivElement;
  };

  let tabs = $state<Tab[]>([]);
  let activeKey = $state(0);
  let host = $state<HTMLDivElement>();
  let nextKey = 1;
  /** Output for ids not registered yet: the shell can speak before \`term_open\` returns. */
  const early = new Map<number, string[]>();
  /** Settles once the output listeners are registered; a shell is not started before. */
  let ready: Promise<unknown> = Promise.resolve();

  const active = $derived(tabs.find((x) => x.key === activeKey));

  function cssVar(name: string, fallback: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
  }

  function theme() {
    const bg = cssVar("--bg", "#0b0b0b");
    return {
      background: bg,
      foreground: cssVar("--txt", "#e9e7e4"),
      cursor: cssVar("--acc", "#e03127"),
      cursorAccent: bg,
      selectionBackground: cssVar("--lines", "#2a2a2a"),
    };
  }

  function fitActive() {
    const tab = active;
    if (!tab || !store.termOpen) return;
    try {
      tab.fit.fit();
    } catch {
      // Not laid out yet.
    }
  }

  async function newTab() {
    if (!host) return;
    const key = nextKey++;
    const el = document.createElement("div");
    el.className = "xterm-host";
    host.appendChild(el);
    const term = new Terminal({
      fontFamily: cssVar("--mono", "monospace"),
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: false,
      theme: theme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    // Ctrl+C copies when something is selected, else it interrupts; Ctrl+V pastes.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown" || !e.ctrlKey) return true;
      const k = e.key.toLowerCase();
      if (k === "c" && term.hasSelection()) {
        void navigator.clipboard.writeText(term.getSelection());
        term.clearSelection();
        return false;
      }
      if (k === "v") return false;
      if (e.key === "`") return false;
      return true;
    });
    const n = tabs.length + 1;
    const tab: Tab = { id: 0, key, title: n > 1 ? `${t("term.tab")} ${n}` : t("term.tab"), exited: false, term, fit, el };
    tabs.push(tab);
    activeKey = key;
    await tick();
    fitActive();
    try {
      await ready;
      const id = await invoke<number>("term_open", { track: store.track || null, cols: term.cols, rows: term.rows });
      const live = tabs.find((x) => x.key === key);
      if (!live) {
        void invoke("term_close", { id });
        return;
      }
      live.id = id;
      for (const chunk of early.get(id) ?? []) term.write(chunk);
      early.delete(id);
      term.onData((data) => {
        if (!live.exited) void invoke("term_write", { id, data }).catch(() => {});
      });
      term.onResize(({ cols, rows }) => {
        if (!live.exited) void invoke("term_resize", { id, cols, rows }).catch(() => {});
      });
      term.focus();
    } catch (err) {
      term.write(`\x1b[31m${String(err)}\x1b[0m\r\n`);
    }
  }

  function closeTab(key: number) {
    const i = tabs.findIndex((x) => x.key === key);
    if (i < 0) return;
    const [tab] = tabs.splice(i, 1);
    if (tab.id && !tab.exited) void invoke("term_close", { id: tab.id });
    tab.term.dispose();
    tab.el.remove();
    if (activeKey === key) activeKey = tabs[Math.min(i, tabs.length - 1)]?.key ?? 0;
    if (tabs.length === 0) void store.setTerminal(false);
  }

  onMount(() => {
    const unlisten = Promise.all([
      listen<{ id: number; data: string }>("term", (e) => {
        const tab = tabs.find((x) => x.id === e.payload.id);
        if (tab) tab.term.write(e.payload.data);
        else early.set(e.payload.id, [...(early.get(e.payload.id) ?? []), e.payload.data]);
      }),
      listen<{ id: number }>("term_exit", (e) => {
        const tab = tabs.find((x) => x.id === e.payload.id);
        if (!tab) return;
        tab.exited = true;
        tab.term.write(`\r\n\x1b[90m${t("term.exited")}\x1b[0m\r\n`);
      }),
    ]);
    ready = unlisten;
    const ro = new ResizeObserver(() => fitActive());
    if (host) ro.observe(host);
    return () => {
      ro.disconnect();
      void unlisten.then((fns) => fns.forEach((f) => f()));
      for (const tab of tabs) {
        if (tab.id && !tab.exited) void invoke("term_close", { id: tab.id });
        tab.term.dispose();
      }
    };
  });

  // Opening the panel with nothing in it starts a shell; showing a tab fits and focuses it.
  $effect(() => {
    if (!store.termOpen) return;
    if (tabs.length === 0) {
      void newTab();
      return;
    }
    const tab = active;
    if (!tab) return;
    for (const x of tabs) x.el.style.display = x.key === tab.key ? "block" : "none";
    void tick().then(() => {
      fitActive();
      tab.term.focus();
    });
  });

  // Follow the app's theme.
  $effect(() => {
    void store.theme;
    const th = theme();
    for (const tab of tabs) tab.term.options.theme = th;
  });

  /**
   * The slide: the outer box's height goes between 0 and the panel height
   * while the terminal inside keeps its full size, so it rises from the
   * bottom without the shell being resized every frame. `shown` trails
   * `store.termOpen` by a frame so the first open animates too; `gone`
   * hides the folded panel from focus once it has slid away.
   */
  let shown = $state(false);
  let gone = $state(true);
  let dragging = $state(false);
  $effect(() => {
    const open = store.termOpen;
    if (open) gone = false;
    const frame = requestAnimationFrame(() => (shown = open));
    // No transitionend without a transition (reduced motion): hide anyway.
    const timer = open ? 0 : setTimeout(() => (gone = !store.termOpen), 320);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
    };
  });
  function onSlid(e: TransitionEvent) {
    if (e.target === e.currentTarget && !store.termOpen) gone = true;
  }

  // Height: drag the top edge.
  function startDrag(e: PointerEvent) {
    e.preventDefault();
    dragging = true;
    const startY = e.clientY;
    const start = store.termHeight;
    const move = (ev: PointerEvent) => store.setTermHeight(start + (startY - ev.clientY), false);
    const stop = () => {
      dragging = false;
      store.setTermHeight(store.termHeight, true);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
  }
</script>

<section
  class="terminal"
  class:shown
  class:gone
  class:dragging
  style="height: {shown ? store.termHeight : 0}px"
  ontransitionend={onSlid}
  aria-label={t("term.tab")}
  aria-hidden={!store.termOpen}
>
  <div class="inner" style="height: {store.termHeight}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="grip" onpointerdown={startDrag} ondblclick={() => store.setTermHeight(280, true)} title={t("term.resize")}></div>
  <div class="bar">
    <div class="tabs">
      {#each tabs as tab (tab.key)}
        <div class="tab" class:on={tab.key === activeKey} class:exited={tab.exited}>
          <button class="pick" onclick={() => (activeKey = tab.key)}>
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><rect x="1.5" y="2.5" width="13" height="11" /><path d="M4.5 6l2 2-2 2M8 10.5h3.5" /></svg>
            <span>{tab.title}</span>
          </button>
          <button class="x" onclick={() => closeTab(tab.key)} aria-label={t("term.close")} title={t("term.close")}><Icon name="close" size={12} /></button>
        </div>
      {/each}
      <button class="icon" onclick={newTab} aria-label={t("term.new")} title={t("term.new")}>
        <svg width="12" height="12" viewBox="0 0 12 12" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M6 1v10M1 6h10" /></svg>
      </button>
    </div>
    <span class="grow"></span>
    <button class="icon" onclick={() => store.setTerminal(false)} aria-label={t("term.hide")} title={t("term.hide")}>
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M2 4l4 4 4-4" /></svg>
    </button>
  </div>
  <div class="host" bind:this={host}></div>
  </div>
</section>

<style>
  /* Slides up from the bottom edge and back down; the inside stays put. */
  .terminal {
    position: relative;
    flex-shrink: 0;
    overflow: hidden;
    border-top: 1px solid transparent;
    background: var(--bg);
    transition:
      height 220ms cubic-bezier(0.2, 0.8, 0.2, 1),
      border-color 220ms;
  }

  .terminal.shown {
    border-top-color: var(--line);
  }

  .terminal.gone {
    visibility: hidden;
  }

  /* Resizing by hand follows the pointer, not a curve. */
  .terminal.dragging {
    transition: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .terminal {
      transition: none;
    }
  }

  .inner {
    position: relative;
    display: flex;
    flex-direction: column;
  }

  .grip {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 6px;
    cursor: row-resize;
    z-index: 5;
  }

  .bar {
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px 0 10px;
  }

  .tabs {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tab {
    display: flex;
    align-items: center;
    height: 24px;
    border: 1px solid transparent;
    color: var(--lab);
    flex-shrink: 0;
  }

  .tab.on {
    border-color: var(--lines);
    color: var(--hi);
    background: var(--sel);
  }

  .tab.exited span {
    text-decoration: line-through;
  }

  .pick {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 100%;
    padding: 0 4px 0 9px;
    background: transparent;
    border: 0;
    color: inherit;
    font-size: 12px;
  }

  .x,
  .icon {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
    padding: 0;
  }

  .x:hover,
  .icon:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .grow {
    flex: 1;
  }

  .host {
    flex: 1;
    min-height: 0;
    padding: 2px 0 6px 14px;
    overflow: hidden;
  }

  .host :global(.xterm-host) {
    height: 100%;
  }

  .host :global(.xterm-viewport) {
    background: transparent !important;
  }
</style>
